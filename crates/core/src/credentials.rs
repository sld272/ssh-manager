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
/// A separate invocation of the bundled CLI isolates native keychain UI policy from embedding applications.
pub struct SystemCredentials {
    pub helper_executable: PathBuf,
}
impl SystemCredentials {
    fn invoke(
        &self,
        reference: &str,
        action: &str,
        input: Option<Vec<u8>>,
    ) -> Result<Zeroizing<String>> {
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
        if output.termination.is_some() || output.code != Some(0) {
            output.stdout.bytes.zeroize();
            return Err(Error::new(
                "credential_backend_unavailable",
                "无法访问系统凭据，请解锁并授权访问，或重新绑定凭据",
            ));
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
fn credential_error(e: keyring::Error) -> Error {
    // A saved reference whose Keychain item was manually deleted is unavailable, not a login rejection.
    match e {
        keyring::Error::NoEntry => {
            Error::new("credential_unavailable", "未找到保存的凭据，请重新绑定")
        }
        _ => Error::new(
            "credential_backend_unavailable",
            "无法访问系统凭据存储，请检查是否已解锁并允许访问",
        ),
    }
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

/// Called only in a short-lived credential helper. stdout is consumed privately by SystemCredentials.
pub fn system_credential_helper(reference: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let _no_ui =
        security_framework::os::macos::keychain::SecKeychain::disable_user_interaction()
            .map_err(|_| Error::new("credential_backend_unavailable", "无法设置凭据访问方式"))?;
    let entry = keyring::Entry::new("dev.ssh-manager", reference).map_err(credential_error)?;
    match std::env::var("SSH_MANAGER_CREDENTIAL_ACTION").as_deref() {
        Ok("get") => {
            let secret = Zeroizing::new(entry.get_password().map_err(credential_error)?);
            std::io::stdout().write_all(secret.as_bytes())?;
        }
        Ok("set") => {
            let mut secret = Zeroizing::new(String::new());
            std::io::stdin().take(65537).read_to_string(&mut secret)?;
            crate::error::require(secret.len() <= 65536, "invalid_credential", "凭据过长")?;
            entry.set_password(&secret).map_err(credential_error)?;
        }
        Ok("delete") => {
            #[cfg(target_os = "macos")]
            {
                match security_framework::passwords::delete_generic_password(
                    "dev.ssh-manager",
                    reference,
                ) {
                    Ok(()) => {}
                    Err(e) if e.code() == -25300 => {}
                    Err(_) => {
                        return Err(Error::new(
                            "credential_backend_unavailable",
                            "无法删除系统凭据",
                        ));
                    }
                }
            }
            #[cfg(target_os = "linux")]
            {
                match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => {}
                    Err(e) => return Err(credential_error(e)),
                }
            }
        }
        _ => {
            return Err(Error::new(
                "invalid_credential_operation",
                "不支持此凭据操作",
            ));
        }
    }
    Ok(())
}
