//! Commands the window calls. Each opens the shared host catalog, so the CLI, MCP and the window see the same data.
use serde::{Deserialize, Serialize};
use ssh_manager_core::{
    credentials::{self, SecretPurpose},
    importer::{self, ImportPlan},
    trust::{self, HostKey},
    *,
};
use std::{
    collections::HashMap,
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
        let history = store.history(None, 1000)?;
        let mut hosts = vec![];
        for host in store.list()? {
            let records = history.iter().filter(|r| r.host_id == host.id);
            hosts.push(HostView {
                trusted_keys: trust::list(&store, host.default_connection)?,
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
    blocking(move || store()?.history(None, limit.min(1000))).await
}

#[tauri::command]
pub async fn check(host: String) -> Result<OperationResult> {
    blocking(move || with_ssh(|ssh| ssh.check(&host, None, &options(30)))).await
}

#[tauri::command]
pub async fn trust_scan(host: String) -> Result<Vec<HostKey>> {
    blocking(move || trust::scan(&store()?, &host, None)).await
}

#[tauri::command]
pub async fn trust_accept(host: String, fingerprint: String, replace: bool) -> Result<HostKey> {
    blocking(move || trust::accept(&store()?, &host, None, fingerprint.trim(), replace)).await
}

#[tauri::command]
pub async fn trust_import(host: String, replace: bool) -> Result<Vec<HostKey>> {
    blocking(move || {
        let source = importer::home()?.join(".ssh/known_hosts");
        trust::import_known_hosts(&store()?, &host, None, &source, replace)
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
pub struct NewHost {
    name: String,
    address: String,
    user: String,
    port: u16,
    /// "key", "agent" or "password".
    auth: String,
    key_path: Option<String>,
    description: String,
    tags: Vec<String>,
}

#[tauri::command]
pub async fn host_add(input: NewHost) -> Result<Host> {
    blocking(move || {
        let authentication = match input.auth.as_str() {
            "key" => {
                let path = input.key_path.unwrap_or_default();
                Authentication::Key {
                    paths: vec![importer::expand_home(path.trim())?.to_string_lossy().into()],
                    passphrase_ref: None,
                }
            }
            "password" => Authentication::Password {
                credential_ref: None,
            },
            _ => Authentication::Agent,
        };
        let mut h = Host::new(
            input.name.trim().into(),
            input.address.trim().into(),
            input.user.trim().into(),
            input.port,
            authentication,
        );
        h.context.description = input.description.trim().into();
        h.tags = input.tags;
        store()?.save(&h)?;
        Ok(h)
    })
    .await
}

#[tauri::command]
pub async fn host_update(
    host: String,
    description: Option<String>,
    tags: Option<Vec<String>>,
    clear_issues: bool,
) -> Result<Host> {
    blocking(move || {
        let store = store()?;
        let mut h = store.host(&host)?;
        if let Some(v) = description {
            h.context.description = v.trim().into();
        }
        if let Some(v) = tags {
            h.tags = v;
        }
        if clear_issues {
            for c in &mut h.connections {
                c.issues.clear();
            }
        }
        store.save(&h)?;
        Ok(h)
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

#[tauri::command]
pub async fn project_save(
    host: String,
    name: String,
    path: String,
    description: String,
) -> Result<Host> {
    blocking(move || {
        let store = store()?;
        let mut h = store.host(&host)?;
        let project = Project {
            name: name.trim().into(),
            path: path.trim().into(),
            description: description.trim().into(),
        };
        match h
            .context
            .projects
            .iter_mut()
            .find(|p| p.name == project.name)
        {
            Some(existing) => *existing = project,
            None => h.context.projects.push(project),
        }
        store.save(&h)?;
        Ok(h)
    })
    .await
}

#[tauri::command]
pub async fn project_remove(host: String, name: String) -> Result<Host> {
    blocking(move || {
        let store = store()?;
        let mut h = store.host(&host)?;
        h.context.projects.retain(|p| p.name != name);
        store.save(&h)?;
        Ok(h)
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
        let existing = store()?.list()?.into_iter().map(|h| h.name).collect();
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

#[derive(Serialize)]
pub struct AgentClient {
    caller: String,
    operations: usize,
    last_used: chrono::DateTime<chrono::Utc>,
}
#[derive(Serialize)]
pub struct Agents {
    clients: Vec<AgentClient>,
    /// The installed command that MCP clients launch.
    command: Option<String>,
}

#[tauri::command]
pub async fn agents() -> Result<Agents> {
    blocking(|| {
        let mut clients: HashMap<String, AgentClient> = HashMap::new();
        for r in store()?.history(None, 1000)? {
            if matches!(r.caller.as_str(), "cli" | "desktop" | "library") {
                continue;
            }
            let entry = clients.entry(r.caller.clone()).or_insert(AgentClient {
                caller: r.caller.clone(),
                operations: 0,
                last_used: r.started_at,
            });
            entry.operations += 1;
            entry.last_used = entry.last_used.max(r.started_at);
        }
        let mut clients: Vec<_> = clients.into_values().collect();
        clients.sort_by_key(|c| std::cmp::Reverse(c.last_used));
        let command = importer::home()?.join(".local/bin/ssh-manager");
        Ok(Agents {
            clients,
            command: command
                .is_file()
                .then(|| command.to_string_lossy().into_owned()),
        })
    })
    .await
}
