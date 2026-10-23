use crate::{Authentication, Error, Result, model::Connection};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

/// Embedders may supply their own local credential provider. Secrets never enter catalog models.
pub trait CredentialStore {
    fn get(&self, reference: &str) -> Result<Zeroizing<String>>;
    fn set(&self, reference: &str, secret: &str) -> Result<()>;
    fn delete(&self, reference: &str) -> Result<()>;
}
/// A dedicated executable isolates keychain policy and identity from the CLI and embedding applications.
pub struct SystemCredentials {
    pub helper_executable: PathBuf,
}
impl SystemCredentials {
    #[cfg(target_os = "linux")]
    fn invoke(
        &self,
        _reference: &str,
        _action: &str,
        _input: Option<Vec<u8>>,
    ) -> Result<Zeroizing<String>> {
        // On a Linux desktop with a locked collection, keyring's current Secret Service adapter calls Unlock and Prompt automatically.
        Err(Error::new(
            "credential_backend_not_ready",
            "此平台的密码存储尚未开放，请使用密钥文件或 ssh-agent",
        ))
    }
    #[cfg(target_os = "macos")]
    fn invoke(
        &self,
        reference: &str,
        action: &str,
        input: Option<Vec<u8>>,
    ) -> Result<Zeroizing<String>> {
        // Installing only the CLI omits this file; fail only when a credential is actually requested.
        crate::error::require(
            self.helper_executable.is_file(),
            "credential_helper_missing",
            "未找到凭据组件，请一同安装 ssh-manager-credentials",
        )?;
        let mut command = std::process::Command::new(&self.helper_executable);
        command
            .env("SSH_MANAGER_CREDENTIAL_HELPER", reference)
            .env("SSH_MANAGER_CREDENTIAL_ACTION", action);
        let mut output = crate::process::run(
            command,
            input,
            Duration::from_secs(5),
            Arc::new(AtomicBool::new(false)),
            65536,
            None,
        )?;
        // A locked keychain or an executable whose access was revoked must fail within five seconds, without opening a dialog.
        if output.termination.is_some() {
            output.stdout.bytes.zeroize();
            return Err(Error::new(
                "credential_backend_timeout",
                "系统凭据访问超时，请稍后重试",
            ));
        }
        if output.code != Some(0) {
            output.stdout.bytes.zeroize();
            let failure: CredentialFailure = serde_json::from_slice(&output.stderr.bytes)?;
            // A locked keychain returns credential_access_required; show the user how to recover, not the helper's policy details.
            let message = if failure.code == "credential_access_required" {
                "暂时无法读取已保存的凭据，请在本机解锁凭据存储或重新绑定".to_owned()
            } else {
                failure.message
            };
            return Err(Error::new(&failure.code, message));
        }
        let bytes = Zeroizing::new(output.stdout.bytes);
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            Error::new(
                "invalid_credential_encoding",
                "保存的凭据不是 UTF-8 文本，请重新绑定",
            )
        })?;
        Ok(Zeroizing::new(text.to_owned()))
    }
}
impl CredentialStore for SystemCredentials {
    fn get(&self, reference: &str) -> Result<Zeroizing<String>> {
        self.invoke(reference, "get", None)
    }
    fn set(&self, reference: &str, secret: &str) -> Result<()> {
        // Empty input and a multiline value cannot be represented by OpenSSH askpass's one-line response.
        crate::error::require(
            !secret.is_empty() && secret.len() <= 65536 && !secret.contains(['\n', '\r', '\0']),
            "invalid_credential",
            "凭据不能为空、过长或包含换行",
        )?;
        self.invoke(reference, "set", Some(secret.as_bytes().to_vec()))?;
        Ok(())
    }
    fn delete(&self, reference: &str) -> Result<()> {
        self.invoke(reference, "delete", None)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretPurpose {
    LoginPassword,
    KeyPassphrase,
}
#[derive(Debug, Serialize)]
pub struct BindReceipt {
    pub host_id: uuid::Uuid,
    pub connection_id: uuid::Uuid,
    pub purpose: SecretPurpose,
    /// The replaced entry, when the credential store could not delete it.
    pub previous_pending_cleanup: Option<String>,
}
/// Saves the secret under a new reference, points the connection at it, then deletes the replaced entry.
pub fn bind(
    store: &crate::Store,
    credentials: &dyn CredentialStore,
    host: &str,
    connection: Option<&str>,
    purpose: SecretPurpose,
    secret: &str,
) -> Result<BindReceipt> {
    let mut h = store.host(host)?;
    let id = h.connection(connection)?.id;
    let c = h
        .connections
        .iter_mut()
        .find(|c| c.id == id)
        .expect("selected connection belongs to host");
    if purpose == SecretPurpose::KeyPassphrase {
        crate::error::require(
            matches!(c.authentication, Authentication::Key { .. }),
            "key_required",
            "请先为此登录配置选择密钥文件",
        )?;
    }
    let previous = match &c.authentication {
        Authentication::Password { credential_ref } => credential_ref.clone(),
        Authentication::Key { passphrase_ref, .. } => passphrase_ref.clone(),
        Authentication::Agent => None,
    };
    let reference = uuid::Uuid::new_v4().to_string();
    credentials.set(&reference, secret)?;
    match (&mut c.authentication, purpose) {
        (Authentication::Key { passphrase_ref, .. }, SecretPurpose::KeyPassphrase) => {
            *passphrase_ref = Some(reference)
        }
        (authentication, _) => {
            *authentication = Authentication::Password {
                credential_ref: Some(reference),
            }
        }
    }
    store.save(&h)?;
    Ok(BindReceipt {
        host_id: h.id,
        connection_id: id,
        purpose,
        previous_pending_cleanup: previous.filter(|r| credentials.delete(r).is_err()),
    })
}
fn references(h: &crate::Host) -> Vec<String> {
    h.connections
        .iter()
        .filter_map(|c| match &c.authentication {
            Authentication::Password { credential_ref } => credential_ref.clone(),
            Authentication::Key { passphrase_ref, .. } => passphrase_ref.clone(),
            Authentication::Agent => None,
        })
        .collect()
}
/// Removes the host, then deletes its saved secrets that no remaining host references.
/// Returns the removed host and the references the credential store could not delete.
pub fn remove_host(
    store: &crate::Store,
    credentials: &dyn CredentialStore,
    host: &str,
) -> Result<(crate::Host, Vec<String>)> {
    let h = store.remove(host)?;
    let remaining: Vec<_> = store.list()?.iter().flat_map(references).collect();
    let pending = references(&h)
        .into_iter()
        .filter(|r| !remaining.contains(r) && credentials.delete(r).is_err())
        .collect();
    Ok((h, pending))
}
#[cfg(target_os = "macos")]
#[derive(Deserialize)]
struct CredentialFailure {
    code: String,
    message: String,
}
#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    prompt: String,
}
pub struct Broker {
    _directory: tempfile::TempDir,
    pub socket: PathBuf,
    pub token: String,
    pub denied: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    redactions: Vec<Zeroizing<String>>,
}
impl Broker {
    pub fn start(connections: &[Connection], credentials: &dyn CredentialStore) -> Result<Self> {
        let mut prompts = HashMap::<String, Zeroizing<String>>::new();
        for c in connections {
            match &c.authentication {
                Authentication::Password {
                    credential_ref: Some(reference),
                } => {
                    let prompt = format!("{}@{}'s password: ", c.user, crate::ssh::alias(c.id));
                    let secret = credentials.get(reference)?;
                    prompts.insert(prompt, secret);
                }
                Authentication::Key {
                    paths,
                    passphrase_ref: Some(reference),
                } => {
                    let secret = credentials.get(reference)?;
                    for p in paths {
                        prompts.insert(format!("Enter passphrase for key '{p}': "), secret.clone());
                    }
                }
                _ => {}
            }
        }
        let redactions = prompts.values().cloned().collect();
        // Use the OS temporary directory: data roots such as ~/Library/Application Support/... exceed sockaddr_un's path limit.
        let directory = tempfile::Builder::new()
            .prefix("sshm-")
            .tempdir_in("/tmp")?;
        let socket = directory.path().join("auth");
        let listener = UnixListener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let token = uuid::Uuid::new_v4().to_string();
        let worker_token = token.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let denied = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_denied = denied.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                        let mut data = String::new();
                        if (&mut stream).take(8192).read_to_string(&mut data).is_ok() {
                            if let Ok(r) = serde_json::from_str::<Request>(&data) {
                                if r.token == worker_token {
                                    if let Some(secret) = prompts.get(&r.prompt) {
                                        let _ = stream.write_all(secret.as_bytes());
                                    } else {
                                        worker_denied.store(true, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            _directory: directory,
            socket,
            token,
            denied,
            stop,
            worker: Some(worker),
            redactions,
        })
    }
    pub fn redact(&self, s: &mut String) {
        for secret in &self.redactions {
            *s = s.replace(secret.as_str(), "[REDACTED]");
        }
    }
}
impl Drop for Broker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
        self.token.zeroize();
    }
}
/// Entry used by the CLI before argument parsing. A helper only receives a secret for an exact prompt in this operation.
pub fn askpass(prompt: &str) -> Result<()> {
    let path = std::env::var_os("SSH_MANAGER_AUTH_SOCKET")
        .ok_or_else(|| Error::new("authentication_unavailable", "认证会话不存在"))?;
    let token = std::env::var("SSH_MANAGER_AUTH_TOKEN")
        .map_err(|_| Error::new("authentication_unavailable", "认证会话不存在"))?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(
        serde_json::to_string(&Request {
            token,
            prompt: prompt.into(),
        })?
        .as_bytes(),
    )?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut secret = Zeroizing::new(Vec::new());
    stream.take(65536).read_to_end(&mut secret)?;
    crate::error::require(
        !secret.is_empty(),
        "needs_interaction",
        "此次认证需要用户操作",
    )?;
    std::io::stdout().write_all(&secret)?;
    Ok(())
}
