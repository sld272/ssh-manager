use crate::{process, ssh::alias, *};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostKey {
    pub algorithm: String,
    pub key: String,
    pub fingerprint: String,
}
fn parse_keys(text: &str) -> Result<Vec<HostKey>> {
    let mut keys = vec![];
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let p: Vec<_> = line.split_whitespace().collect();
        // @revoked and @cert-authority records have different trust semantics; never turn them into ordinary accepted keys.
        error::require(
            p.len() >= 3 && !p[0].starts_with('@'),
            "unsupported_trust_record",
            "此记录不是普通主机公钥，不能直接导入",
        )?;
        let bytes = STANDARD
            .decode(p[2])
            .map_err(|_| Error::new("invalid_host_key", "主机公钥格式无效"))?;
        let fingerprint = format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(&bytes)));
        if !keys.iter().any(|k: &HostKey| k.key == p[2]) {
            keys.push(HostKey {
                algorithm: p[1].into(),
                key: p[2].into(),
                fingerprint,
            });
        }
    }
    Ok(keys)
}
pub fn scan(store: &Store, host: &str, connection: Option<&str>) -> Result<Vec<HostKey>> {
    let h = store.host(host)?;
    let c = h.connection(connection)?;
    error::require(
        c.jumps.is_empty(),
        "trust_scan_requires_direct_connection",
        "跳板后的主机请从已有 known_hosts 导入信任记录",
    )?;
    let mut cmd = Command::new("ssh-keyscan");
    cmd.args(["-T", "5", "-p", &c.port.to_string(), &c.address]);
    let r = process::run(
        cmd,
        None,
        Duration::from_secs(12),
        Arc::new(AtomicBool::new(false)),
        131072,
        None,
    )?;
    let keys = parse_keys(&String::from_utf8_lossy(&r.stdout.bytes))?;
    error::require(
        r.termination.is_none() && !keys.is_empty(),
        "host_key_scan_failed",
        "无法读取主机公钥，请检查地址、端口和网络",
    )?;
    Ok(keys)
}
/// The keys that `files` (OpenSSH known_hosts files) record for `name`, as OpenSSH files it:
/// the HostKeyAlias, the address, or "[address]:port" off port 22.
fn find(name: &str, files: &[std::path::PathBuf]) -> Result<Vec<HostKey>> {
    let mut text = String::new();
    for file in files.iter().filter(|f| f.is_file()) {
        let mut cmd = Command::new("ssh-keygen");
        cmd.arg("-F").arg(name).arg("-f").arg(file);
        let r = process::run(
            cmd,
            None,
            Duration::from_secs(5),
            Arc::new(AtomicBool::new(false)),
            131072,
            None,
        )?;
        text.push_str(&String::from_utf8_lossy(&r.stdout.bytes));
        text.push('\n');
    }
    // "@cert-authority *.corp.example.com ..." matches the name by pattern but is a CA key, not this host's key.
    let plain: Vec<_> = text.lines().filter(|l| !l.starts_with('@')).collect();
    parse_keys(&plain.join("\n"))
}
pub fn known_hosts_name(c: &Connection) -> String {
    if c.port == 22 {
        c.address.clone()
    } else {
        format!("[{}]:{}", c.address, c.port)
    }
}
pub fn import_known_hosts(
    store: &Store,
    host: &str,
    connection: Option<&str>,
    source: &Path,
    replace: bool,
) -> Result<Vec<HostKey>> {
    let h = store.host(host)?;
    let c = h.connection(connection)?;
    let keys = find(&known_hosts_name(c), &[source.to_path_buf()])?;
    error::require(
        !keys.is_empty(),
        "trusted_key_not_found",
        "文件中没有此地址和端口的信任记录",
    )?;
    save(store, c.id, &keys, replace)?;
    Ok(keys)
}
/// Copies what OpenSSH already trusts for `name` into the managed known_hosts of a connection that has
/// no key yet. Returns how many keys were copied: none when this machine never connected to the server.
pub fn adopt(
    store: &Store,
    id: uuid::Uuid,
    name: &str,
    files: &[std::path::PathBuf],
) -> Result<usize> {
    let keys = find(name, files)?;
    if !keys.is_empty() {
        save(store, id, &keys, false)?;
    }
    Ok(keys.len())
}
/// Drops a connection's saved keys; its next connection trusts whatever key the server presents.
pub fn forget(store: &Store, id: uuid::Uuid) -> Result<()> {
    save(store, id, &[], true)
}
pub fn accept(
    store: &Store,
    host: &str,
    connection: Option<&str>,
    fingerprint: &str,
    replace: bool,
) -> Result<HostKey> {
    let keys = scan(store, host, connection)?;
    let key = keys
        .into_iter()
        .find(|k| k.fingerprint == fingerprint)
        .ok_or_else(|| Error::new("fingerprint_mismatch", "当前主机公钥与指定指纹不匹配"))?;
    let h = store.host(host)?;
    save(
        store,
        h.connection(connection)?.id,
        std::slice::from_ref(&key),
        replace,
    )?;
    Ok(key)
}
pub fn list(store: &Store, id: uuid::Uuid) -> Result<Vec<HostKey>> {
    let path = store.root().join("known_hosts");
    let text = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let selected = text
        .lines()
        .filter(|l| l.split_whitespace().next() == Some(alias(id).as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    parse_keys(&selected)
}
fn save(store: &Store, id: uuid::Uuid, keys: &[HostKey], replace: bool) -> Result<()> {
    use fs2::FileExt;
    use std::os::unix::fs::OpenOptionsExt;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(store.root().join("trust.lock"))?;
    lock.lock_exclusive()?;
    let existing = list(store, id)?;
    error::require(
        replace
            || existing.is_empty()
            || keys.iter().all(|k| existing.iter().any(|e| e.key == k.key)),
        "host_key_conflict",
        "已保存不同的主机公钥；核实变化后使用 --replace 更新",
    )?;
    let path = store.root().join("known_hosts");
    let original = match std::fs::read_to_string(&path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut temp = tempfile::NamedTempFile::new_in(store.root())?;
    for line in original
        .lines()
        .filter(|l| l.split_whitespace().next() != Some(alias(id).as_str()))
    {
        writeln!(temp, "{line}")?;
    }
    for k in keys {
        writeln!(temp, "{} {} {}", alias(id), k.algorithm, k.key)?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| Error::from(e.error))?;
    Ok(())
}
