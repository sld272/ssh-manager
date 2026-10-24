//! Commands the window calls. Each opens the shared host catalog, so the CLI, MCP and the window see the same data.
use serde::{Deserialize, Serialize};
use ssh_manager_core::{
    credentials::{self, SecretPurpose},
    importer::{self, ImportPlan},
    trust::{self, HostKey},
    *,
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, atomic::AtomicBool},
    time::Duration,
};
use zeroize::Zeroize;

const CALLER: &str = "desktop";

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| Error::new("task_failed", e.to_string()))?
}
fn store() -> Result<Store> {
    Store::open(Store::default_root()?)
}
fn credential_helper() -> Result<PathBuf> {
    // A development build in target/ sits next to a freshly built helper that the keychain has never
    // granted; the installed helper is the one that holds the passwords the CLI saved.
    let installed = importer::home()?.join(".local/bin/ssh-manager-credentials");
    if installed.is_file() {
        return Ok(installed);
    }
    Ok(std::env::current_exe()?.with_file_name("ssh-manager-credentials"))
}
fn system_credentials() -> Result<SystemCredentials> {
    Ok(SystemCredentials {
        helper_executable: credential_helper()?,
    })
}
fn options(timeout: u64) -> ExecutionOptions {
    ExecutionOptions {
        timeout: Duration::from_secs(timeout),
        output_limit: 65536,
        cancel: Arc::new(AtomicBool::new(false)),
        caller: CALLER.into(),
        reason: None,
    }
}
/// Runs an SSH operation with this executable as the askpass helper (see main).
fn with_ssh<T>(f: impl FnOnce(&OpenSsh) -> Result<T>) -> Result<T> {
    let store = store()?;
    let credentials = system_credentials()?;
    let ssh = OpenSsh {
        store: &store,
        credentials: &credentials,
        askpass_executable: std::env::current_exe()?,
    };
    f(&ssh)
}

#[derive(Serialize)]
pub struct HostView {
    #[serde(flatten)]
    host: Host,
    trusted_keys: Vec<HostKey>,
    /// The default login's key file is protected by a passphrase.
    key_passphrase: bool,
    last_check: Option<OperationRecord>,
    last_operation: Option<OperationRecord>,
}
#[derive(Serialize)]
pub struct Overview {
    hosts: Vec<HostView>,
    ssh_version: Option<String>,
    credential_helper_available: bool,
}
fn ssh_version() -> Option<String> {
    static VERSION: OnceLock<Option<String>> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            // "OpenSSH_9.9p2, LibreSSL 3.3.6" is printed on stderr.
            let out = std::process::Command::new("ssh").arg("-V").output().ok()?;
            let text = String::from_utf8_lossy(&out.stderr);
            let first = text.split([',', ' ']).next()?.replace('_', " ");
            first.starts_with("OpenSSH").then_some(first)
        })
        .clone()
}

#[tauri::command]
pub async fn overview() -> Result<Overview> {
    blocking(|| {
        let store = store()?;
        let history = store.history(None, None, None, 1000)?;
        let mut hosts = vec![];
        for host in store.list()? {
            let records = history.iter().filter(|r| r.host_id == host.id);
            let key_passphrase = match &host.connection(None)?.authentication {
                Authentication::Key { paths, .. } => {
                    paths.iter().any(|p| credentials::key_has_passphrase(p))
                }
                _ => false,
            };
            hosts.push(HostView {
                trusted_keys: trust::list(&store, host.default_connection)?,
                key_passphrase,
                last_check: records.clone().find(|r| r.kind == "check").cloned(),
                last_operation: records.clone().next().cloned(),
                host,
            });
        }
        Ok(Overview {
            hosts,
            ssh_version: ssh_version(),
            credential_helper_available: credential_helper()?.is_file(),
        })
    })
    .await
}

#[tauri::command]
pub async fn history(limit: u32) -> Result<Vec<OperationRecord>> {
    blocking(move || store()?.history(None, None, None, limit.min(1000))).await
}

#[tauri::command]
pub async fn check(host: String) -> Result<OperationResult> {
    blocking(move || with_ssh(|ssh| ssh.check(&host, None, &options(30)))).await
}

/// Forgets a server's saved key after it changed; the next connection trusts the key it presents.
#[tauri::command]
pub async fn trust_reset(host: String) -> Result<()> {
    blocking(move || {
        let store = store()?;
        let id = store.host(&host)?.default_connection;
        trust::forget(&store, id)
    })
    .await
}

#[tauri::command]
pub async fn credential_set(
    host: String,
    purpose: SecretPurpose,
    mut secret: String,
) -> Result<credentials::BindReceipt> {
    blocking(move || {
        let result = credentials::bind(
            &store()?,
            &system_credentials()?,
            &host,
            None,
            purpose,
            &secret,
        );
        secret.zeroize();
        result
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInput {
    name: String,
    address: String,
    user: String,
    port: u16,
    /// "key" or "password".
    auth: String,
    /// Key files; empty when the keys come from an agent. Imported hosts often list several, and an
    /// imported 1Password login names the public key that picks one of the agent's keys.
    key_paths: Vec<String>,
    /// Whether the keys come from an agent rather than from key files.
    use_agent: bool,
    /// The agent's socket; None is the system ssh-agent.
    identity_agent: Option<String>,
    description: String,
    tags: Vec<String>,
}
impl HostInput {
    fn authentication(&self) -> Result<Authentication> {
        Ok(match self.auth.as_str() {
            "password" => Authentication::Password {
                credential_ref: None,
            },
            _ if self.use_agent && self.key_paths.is_empty() => Authentication::Agent,
            _ => Authentication::Key {
                paths: self
                    .key_paths
                    .iter()
                    .map(|p| Ok(importer::expand_home(p.trim())?.to_string_lossy().into()))
                    .collect::<Result<_>>()?,
                passphrase_ref: None,
            },
        })
    }
    fn identity_agent(&self) -> Option<String> {
        self.identity_agent
            .clone()
            .filter(|_| self.auth == "key" && self.use_agent)
    }
}

#[tauri::command]
pub async fn host_add(input: HostInput) -> Result<Host> {
    blocking(move || {
        let mut h = Host::new(
            input.name.trim().into(),
            input.address.trim().into(),
            input.user.trim().into(),
            input.port,
            input.authentication()?,
        );
        h.connections[0].identity_agent = input.identity_agent();
        h.context.description = input.description.trim().into();
        h.tags = input.tags;
        store()?.save(&h)?;
        Ok(h)
    })
    .await
}

/// Edits the host and its default login. Returns the saved secrets the keychain could not delete.
#[tauri::command]
pub async fn host_update(host: String, input: HostInput) -> Result<Vec<String>> {
    blocking(move || {
        let store = store()?;
        let mut h = store.host(&host)?;
        h.name = input.name.trim().into();
        h.context.description = input.description.trim().into();
        h.tags = input.tags.clone();
        let id = h.default_connection;
        let c = h
            .connections
            .iter_mut()
            .find(|c| c.id == id)
            .expect("validated hosts contain their default login");
        c.address = input.address.trim().into();
        c.user = input.user.trim().into();
        c.port = input.port;
        c.identity_agent = input.identity_agent();
        let next = input.authentication()?;
        // The saved password or passphrase stays only while the method and key files stay the same.
        let same = match (&c.authentication, &next) {
            (Authentication::Key { paths: a, .. }, Authentication::Key { paths: b, .. }) => a == b,
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        };
        if !same {
            // An imported "IdentitiesOnly yes" with agent login would offer no key at all.
            c.identities_only = matches!(next, Authentication::Key { .. });
            c.authentication = next;
        }
        credentials::update_host(&store, &system_credentials()?, &h)
    })
    .await
}

#[tauri::command]
pub async fn host_clear_issues(host: String) -> Result<()> {
    blocking(move || {
        let store = store()?;
        let mut h = store.host(&host)?;
        for c in &mut h.connections {
            c.issues.clear();
        }
        store.save(&h)
    })
    .await
}

#[tauri::command]
pub async fn host_remove(host: String) -> Result<Vec<String>> {
    blocking(move || {
        let (_, pending) = credentials::remove_host(&store()?, &system_credentials()?, &host)?;
        Ok(pending)
    })
    .await
}

fn import_source(path: Option<String>) -> Result<PathBuf> {
    match path.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => importer::expand_home(p),
        _ => Ok(importer::home()?.join(".ssh/config")),
    }
}
#[derive(Serialize)]
pub struct ImportPreview {
    plan: ImportPlan,
    existing: Vec<String>,
}

#[tauri::command]
pub async fn import_preview(path: Option<String>) -> Result<ImportPreview> {
    blocking(move || {
        let plan = importer::preview(&import_source(path)?, &[])?;
        // A host imported earlier counts as present even after it was renamed.
        let catalog = store()?.list()?;
        let existing = plan
            .hosts
            .iter()
            .map(|p| &p.name)
            .filter(|name| {
                catalog.iter().any(|h| {
                    &h.name == *name
                        || h.source
                            .as_ref()
                            .is_some_and(|s| s.path == plan.source && &s.alias == *name)
                })
            })
            .cloned()
            .collect();
        Ok(ImportPreview { plan, existing })
    })
    .await
}

#[tauri::command]
pub async fn import_apply(path: Option<String>, names: Vec<String>) -> Result<usize> {
    blocking(move || {
        error::require(!names.is_empty(), "nothing_selected", "请至少选择一台主机")?;
        let plan = importer::preview(&import_source(path)?, &names)?;
        importer::apply(&store()?, plan)
    })
    .await
}

/// Window preferences, kept beside the host catalog so they survive reinstalling the app.
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// "system", "zh-CN" or "en".
    language: String,
    /// "system", "light" or "dark".
    theme: String,
    /// One of TERMINALS' ids.
    terminal: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            theme: "system".into(),
            terminal: "terminal".into(),
        }
    }
}
fn settings_path() -> Result<PathBuf> {
    Ok(Store::default_root()?.join("desktop.json"))
}
fn load_settings() -> Result<Settings> {
    match std::fs::read_to_string(settings_path()?) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.into()),
    }
}

/// Terminals that can run the connect script: id, name and app bundle name.
const TERMINALS: &[(&str, &str, &str)] = &[
    ("terminal", "Terminal", "Terminal.app"),
    ("ghostty", "Ghostty", "Ghostty.app"),
    ("iterm", "iTerm", "iTerm.app"),
];
/// Where the terminal is installed: Terminal ships with macOS, others go to /Applications or ~/Applications.
fn terminal_app(id: &str) -> Result<Option<PathBuf>> {
    let Some((_, _, bundle)) = TERMINALS.iter().find(|(t, ..)| *t == id) else {
        return Ok(None);
    };
    let places = [
        PathBuf::from("/System/Applications/Utilities"),
        PathBuf::from("/Applications"),
        importer::home()?.join("Applications"),
    ];
    Ok(places.iter().map(|p| p.join(bundle)).find(|p| p.exists()))
}
/// The saved terminal, or the system Terminal once the saved one is uninstalled.
fn current_terminal(settings: &Settings) -> Result<String> {
    Ok(match terminal_app(&settings.terminal)? {
        Some(_) => settings.terminal.clone(),
        None => "terminal".into(),
    })
}
#[derive(Serialize)]
pub struct TerminalApp {
    id: &'static str,
    name: &'static str,
}
#[derive(Serialize)]
pub struct Environment {
    ssh_version: Option<String>,
    credential_helper_available: bool,
    data_dir: String,
    command: Option<String>,
    version: &'static str,
}
#[derive(Serialize)]
pub struct SettingsView {
    settings: Settings,
    terminals: Vec<TerminalApp>,
    environment: Environment,
}
fn installed_command() -> Result<Option<PathBuf>> {
    let command = importer::home()?.join(".local/bin/ssh-manager");
    Ok(command.is_file().then_some(command))
}

#[tauri::command]
pub async fn settings_get() -> Result<SettingsView> {
    blocking(|| {
        let mut settings = load_settings()?;
        settings.terminal = current_terminal(&settings)?;
        let mut terminals = vec![];
        for (id, name, _) in TERMINALS {
            if terminal_app(id)?.is_some() {
                terminals.push(TerminalApp { id, name });
            }
        }
        Ok(SettingsView {
            settings,
            terminals,
            environment: Environment {
                ssh_version: ssh_version(),
                credential_helper_available: credential_helper()?.is_file(),
                data_dir: Store::default_root()?.to_string_lossy().into_owned(),
                command: installed_command()?.map(|p| p.to_string_lossy().into_owned()),
                version: env!("CARGO_PKG_VERSION"),
            },
        })
    })
    .await
}

#[tauri::command]
pub async fn settings_set(settings: Settings) -> Result<Settings> {
    blocking(move || {
        error::require(
            terminal_app(&settings.terminal)?.is_some(),
            "invalid_settings",
            "未找到所选终端",
        )?;
        let path = settings_path()?;
        store::private_dir(path.parent().expect("settings live in the data directory"))?;
        std::fs::write(&path, serde_json::to_string_pretty(&settings)?)?;
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub async fn reveal_data_dir() -> Result<()> {
    blocking(|| {
        std::process::Command::new("open")
            .arg(Store::default_root()?)
            .status()?;
        Ok(())
    })
    .await
}

/// Opens the chosen terminal on `ssh-manager connect`, so the session uses this catalog's
/// trust, jump hosts and saved passwords without any secret passing through the terminal.
#[tauri::command]
pub async fn open_terminal(host: String) -> Result<()> {
    blocking(move || {
        let h = store()?.host(&host)?;
        let command = installed_command()?.ok_or_else(|| {
            Error::new(
                "command_unavailable",
                "未找到 ssh-manager 命令行工具，请先安装后再在终端中连接",
            )
        })?;
        let terminal = current_terminal(&load_settings()?)?;
        let app = terminal_app(&terminal)?.expect("current_terminal returns an installed terminal");
        // Terminal and iTerm run a .command file directly; the script removes itself before connecting.
        let script = tempfile::Builder::new()
            .prefix("ssh-manager-")
            .suffix(".command")
            .tempfile()?;
        let (mut file, path) = script.keep().map_err(|e| Error::from(e.error))?;
        use std::{io::Write, os::unix::fs::PermissionsExt};
        writeln!(
            file,
            "#!/bin/sh\nrm -f -- \"$0\"\nexec {} connect {}",
            ssh::shell_quote(&command.to_string_lossy()),
            h.id
        )?;
        file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        drop(file);
        let mut open = std::process::Command::new("open");
        if terminal == "ghostty" {
            open.arg("-na").arg(&app).args(["--args", "-e"]).arg(&path);
        } else {
            open.arg("-a").arg(&app).arg(&path);
        }
        let status = open.status()?;
        error::require(status.success(), "terminal_failed", "无法打开终端")?;
        Ok(())
    })
    .await
}

/// Opens a Finder panel in ~/.ssh, where private keys usually live; None when the person cancels.
#[tauri::command]
pub async fn pick_key_file(app: tauri::AppHandle) -> Result<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let directory = importer::home()?.join(".ssh");
    blocking(move || {
        Ok(app
            .dialog()
            .file()
            .set_directory(directory)
            .blocking_pick_file()
            .and_then(|f| f.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned()))
    })
    .await
}

#[derive(Serialize)]
pub struct KeyAgent {
    /// None is the system ssh-agent.
    socket: Option<String>,
    name: &'static str,
}

/// Agents that hold keys on this Mac: the system ssh-agent, and 1Password or Secretive when their agent is on.
#[tauri::command]
pub async fn key_agents() -> Result<Vec<KeyAgent>> {
    blocking(|| {
        let home = importer::home()?;
        let mut agents = vec![];
        if std::env::var_os("SSH_AUTH_SOCK").is_some_and(|p| std::path::Path::new(&p).exists()) {
            agents.push(KeyAgent {
                socket: None,
                name: "ssh-agent",
            });
        }
        for (name, path) in [
            (
                "1Password",
                "Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock",
            ),
            (
                "Secretive",
                "Library/Containers/com.maxgoedjen.Secretive.SecretAgent/Data/socket.ssh",
            ),
        ] {
            let socket = home.join(path);
            if socket.exists() {
                agents.push(KeyAgent {
                    socket: Some(socket.to_string_lossy().into_owned()),
                    name,
                });
            }
        }
        Ok(agents)
    })
    .await
}

#[derive(Serialize)]
pub struct Reach {
    host_id: uuid::Uuid,
    reachable: bool,
    /// A host behind a jump host cannot be reached directly; its first jump host was probed instead.
    via_jump: bool,
}

/// Probes the hosts in parallel without logging in to any of them.
#[tauri::command]
pub async fn probe(hosts: Vec<String>) -> Result<Vec<Reach>> {
    blocking(move || {
        let catalog = store()?.list()?;
        let connection = |id: uuid::Uuid| {
            catalog
                .iter()
                .flat_map(|h| &h.connections)
                .find(|c| c.id == id)
        };
        let targets: Vec<_> = catalog
            .iter()
            .filter(|h| hosts.iter().any(|s| *s == h.id.to_string()))
            .filter_map(|h| {
                let c = h.connection(None).ok()?;
                let first = match c.jumps.first() {
                    Some(id) => connection(*id)?,
                    None => c,
                };
                Some((h.id, first.address.clone(), first.port, !c.jumps.is_empty()))
            })
            .collect();
        Ok(std::thread::scope(|scope| {
            let running: Vec<_> = targets
                .into_iter()
                .map(|(host_id, address, port, via_jump)| {
                    scope.spawn(move || Reach {
                        host_id,
                        reachable: ssh_manager_core::reach::probe(
                            &address,
                            port,
                            Duration::from_secs(4),
                        ),
                        via_jump,
                    })
                })
                .collect();
            running
                .into_iter()
                .map(|r| r.join().expect("a probe does not panic"))
                .collect()
        }))
    })
    .await
}
