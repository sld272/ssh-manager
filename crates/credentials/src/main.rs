//! Native credentials only. Keep this executable independent of the CLI and SSH core.
use serde::Serialize;
use std::io::{Read, Write};
use zeroize::Zeroizing;

#[derive(Serialize)]
struct Error {
    code: String,
    message: String,
}
type Result<T> = std::result::Result<T, Error>;
impl Error {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::new("credential_io_failed", "无法读取或返回凭据")
    }
}

fn credential_error(e: keyring::Error) -> Error {
    match e {
        keyring::Error::NoEntry => {
            Error::new("credential_unavailable", "未找到保存的凭据，请重新绑定")
        }
        // Only the macOS keychain reports a locked store in a way that is told apart below.
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
        keyring::Error::PlatformFailure(ref native)
        | keyring::Error::NoStorageAccess(ref native) => {
            #[cfg(target_os = "macos")]
            // A locked keychain or a replaced executable can return errSecInteractionNotAllowed (-25308) or errSecAuthFailed (-25293).
            if native
                .downcast_ref::<security_framework::base::Error>()
                .is_some_and(|e| matches!(e.code(), -25308 | -25293))
            {
                return Error::new(
                    "credential_access_required",
                    "凭据已锁定或尚未授权，请在本机处理后重试；连接不会弹出授权窗口",
                );
            }
            Error::new(
                "credential_backend_unavailable",
                "无法访问系统凭据存储，请检查是否可用",
            )
        }
        _ => Error::new(
            "credential_backend_unavailable",
            "无法访问系统凭据存储，请检查是否可用",
        ),
    }
}
/// Called only in a short-lived credential helper. stdout is consumed privately by SystemCredentials.
fn operate(reference: &str) -> Result<()> {
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
            // A direct helper invocation with 65537 bytes must not bypass the CLI's one-line credential limit.
            if secret.is_empty() || secret.len() > 65536 || secret.contains(['\n', '\r', '\0']) {
                return Err(Error::new(
                    "invalid_credential",
                    "凭据不能为空、过长或包含换行",
                ));
            }
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

fn main() {
    let result = std::env::var("SSH_MANAGER_CREDENTIAL_HELPER")
        .map_err(|_| {
            Error::new(
                "credential_reference_required",
                "请通过 ssh-manager 管理凭据",
            )
        })
        .and_then(|reference| operate(&reference));
    if let Err(error) = result {
        let _ = serde_json::to_writer(std::io::stderr(), &error);
        std::process::exit(1);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use security_framework::os::macos::{
        keychain::{CreateOptions, SecKeychain},
        passwords::find_generic_password,
    };
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::process::Command::new("security")
                .arg("delete-keychain")
                .arg(&self.0)
                .output();
        }
    }
    #[test]
    fn native_access_works_and_locked_keychain_fails_without_dialogs() {
        let _no_ui = SecKeychain::disable_user_interaction().unwrap();
        assert!(!SecKeychain::user_interaction_allowed().unwrap());
        let directory = tempfile::tempdir().unwrap();
        let fixture = Fixture(directory.path().join("fixture.keychain"));
        let keychain = CreateOptions::new()
            .password("test-keychain-password")
            .create(&fixture.0)
            .unwrap();
        keychain
            .set_generic_password("ssh-manager-test", "account", b"fixture-secret")
            .unwrap();
        assert_eq!(
            find_generic_password(
                Some(std::slice::from_ref(&keychain)),
                "ssh-manager-test",
                "account"
            )
            .unwrap()
            .0
            .as_ref(),
            b"fixture-secret"
        );
        assert!(
            std::process::Command::new("security")
                .arg("lock-keychain")
                .arg(&fixture.0)
                .output()
                .unwrap()
                .status
                .success()
        );
        let clock = Instant::now();
        let failure = find_generic_password(
            Some(std::slice::from_ref(&keychain)),
            "ssh-manager-test",
            "account",
        )
        .unwrap_err();
        assert!(matches!(failure.code(), -25308 | -25293));
        assert!(clock.elapsed() < Duration::from_secs(2));
        assert!(!SecKeychain::user_interaction_allowed().unwrap());
    }
}
