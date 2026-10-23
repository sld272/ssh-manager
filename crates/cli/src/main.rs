mod mcp;
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use ssh_manager_core::{importer, transfer::TransferMode, trust, *};
use std::{
    io::{self, IsTerminal, Read},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(name="ssh-manager",version,about="管理 SSH 主机、工作上下文和远程操作",long_about=None)]
struct Cli {
    #[arg(long, global = true, help = "本机数据目录")]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true, help = "输出 JSON")]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    #[command(about = "启动本地 MCP 接口")]
    Mcp,
    #[command(about = "管理服务器")]
    Host {
        #[command(subcommand)]
        command: HostCommand,
    },
    #[command(about = "管理同一主机的登录配置")]
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    #[command(about = "绑定系统凭据")]
    Credential {
        #[command(subcommand)]
        command: CredentialCommand,
    },
    #[command(about = "核对服务器身份")]
    Trust {
        #[command(subcommand)]
        command: TrustCommand,
    },
    #[command(about = "维护项目与服务器说明")]
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    #[command(about = "检查连接并更新系统信息")]
    Check {
        host: String,
        #[command(flatten)]
        options: RunArgs,
    },
    #[command(about = "执行远端 Shell 命令")]
    Exec {
        host: String,
        #[arg(long, conflicts_with = "project", help = "远端工作目录，使用绝对路径")]
        cwd: Option<String>,
        #[arg(long, conflicts_with = "cwd", help = "已保存的项目名称")]
        project: Option<String>,
        #[command(flatten)]
        options: RunArgs,
        #[arg(last = true, required = true, help = "完整的 Shell 命令，请用引号包裹")]
        command: String,
    },
    #[command(about = "上传文件或目录内容")]
    Upload {
        host: String,
        source: PathBuf,
        destination: String,
        #[arg(
            long,
            value_enum,
            default_value = "file",
            help = "file 为完整文件名；contents 将目录内容合并到目标目录"
        )]
        mode: Mode,
        #[command(flatten)]
        options: RunArgs,
    },
    #[command(about = "下载单个文件")]
    Download {
        host: String,
        source: String,
        destination: PathBuf,
        #[command(flatten)]
        options: RunArgs,
    },
    #[command(about = "查看操作记录")]
    History {
        #[arg(long)]
        host: Option<String>,
        #[arg(long,default_value_t=30,value_parser=clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
    },
    #[command(about = "检查本机工具和存储位置")]
    Doctor,
}
#[derive(Args)]
struct RunArgs {
    #[arg(long, help = "登录配置名称或 ID")]
    connection: Option<String>,
    #[arg(long,default_value_t=60,value_parser=clap::value_parser!(u64).range(1..=86400),help="整个远程命令的最长等待秒数")]
    timeout: u64,
    #[arg(long,default_value_t=65536,value_parser=clap::value_parser!(u32).range(256..=16777216),help="每个输出流最多返回的字节数")]
    output_limit: u32,
    #[arg(long, default_value = "cli", help = "调用方名称")]
    caller: String,
    #[arg(long, help = "此次操作的目的；会保存到操作记录")]
    reason: Option<String>,
}
impl RunArgs {
    fn execution(&self, cancel: Arc<AtomicBool>) -> ExecutionOptions {
        ExecutionOptions {
            timeout: Duration::from_secs(self.timeout),
            output_limit: self.output_limit as usize,
            cancel,
            caller: self.caller.clone(),
            reason: self.reason.clone(),
        }
    }
}
#[derive(Clone, Copy, clap::ValueEnum)]
enum Mode {
    File,
    Contents,
}
#[derive(Args)]
struct AuthenticationArgs {
    #[arg(long,conflicts_with_all=["password","agent"],help="密钥文件，可重复指定")]
    key: Vec<PathBuf>,
    #[arg(long,conflicts_with_all=["key","agent"],help="使用密码认证，随后通过 credential set 绑定密码")]
    password: bool,
    #[arg(long,conflicts_with_all=["key","password"],help="使用 ssh-agent")]
    agent: bool,
    #[arg(
        long,
        conflicts_with = "password",
        help = "使用指定的 ssh-agent 套接字，例如 1Password 或 Secretive 提供的套接字"
    )]
    identity_agent: Option<PathBuf>,
}
impl AuthenticationArgs {
    fn selected(&self) -> bool {
        !self.key.is_empty() || self.password || self.agent
    }
    fn agent_socket(&self) -> Result<Option<String>> {
        self.identity_agent
            .as_deref()
            .map(|p| absolute(p).map(|p| p.to_string_lossy().into()))
            .transpose()
    }
    fn value(&self) -> Result<Authentication> {
        if self.password {
            Ok(Authentication::Password {
                credential_ref: None,
            })
        } else if self.key.is_empty() {
            Ok(Authentication::Agent)
        } else {
            Ok(Authentication::Key {
                paths: self
                    .key
                    .iter()
                    .map(|p| absolute(p).map(|p| p.to_string_lossy().into()))
                    .collect::<Result<_>>()?,
                passphrase_ref: None,
            })
        }
    }
}
#[derive(Subcommand)]
enum HostCommand {
    #[command(about = "列出主机")]
    List {
        #[arg(long)]
        tag: Option<String>,
    },
    #[command(about = "查看主机与上下文")]
    Show { host: String },
    #[command(about = "添加主机")]
    Add {
        name: String,
        #[arg(long)]
        address: String,
        #[arg(long)]
        user: String,
        #[arg(long, default_value_t = 22)]
        port: u16,
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, value_delimiter = ',', help = "按顺序指定已有跳板主机")]
        jump: Vec<String>,
        #[command(flatten)]
        auth: AuthenticationArgs,
    },
    #[command(about = "修改主机")]
    Edit {
        host: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',', conflicts_with = "no_jump")]
        jump: Option<Vec<String>>,
        #[arg(long)]
        no_jump: bool,
        #[arg(long, help = "确认已处理列出的配置导入问题")]
        clear_issues: bool,
        #[arg(long, conflicts_with = "identity_agent", help = "改用默认的 ssh-agent")]
        no_identity_agent: bool,
        #[arg(long)]
        connection: Option<String>,
        #[command(flatten)]
        auth: AuthenticationArgs,
    },
    #[command(about = "删除主机，保留历史记录")]
    Remove { host: String },
    #[command(about = "预览或导入 OpenSSH 配置")]
    Import {
        #[arg(long,default_value="openssh",value_parser=["openssh"])]
        from: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, value_delimiter = ',', help = "只选择这些别名及其跳板依赖")]
        select: Vec<String>,
        #[arg(long, help = "保存导入结果；默认仅预览")]
        apply: bool,
    },
    #[command(about = "导出配置，不包含密码或凭据绑定")]
    Export,
    #[command(about = "从 JSON 文件导入配置，不覆盖已有主机")]
    Restore { file: PathBuf },
}
#[derive(Subcommand)]
enum ConnectionCommand {
    List {
        host: String,
    },
    Add {
        host: String,
        name: String,
        #[arg(long)]
        address: String,
        #[arg(long)]
        user: String,
        #[arg(long, default_value_t = 22)]
        port: u16,
        #[arg(long, value_delimiter = ',')]
        jump: Vec<String>,
        #[command(flatten)]
        auth: AuthenticationArgs,
    },
    Default {
        host: String,
        connection: String,
    },
    Remove {
        host: String,
        connection: String,
    },
}
#[derive(Subcommand)]
enum CredentialCommand {
    #[command(about = "保存登录密码或密钥口令；输入不会回显")]
    Set {
        host: String,
        #[arg(long)]
        connection: Option<String>,
        #[arg(long, help = "绑定为密钥口令")]
        passphrase: bool,
        #[arg(long, help = "从标准输入读取，供本机凭据工具使用")]
        password_stdin: bool,
    },
    #[command(about = "删除保存的凭据")]
    Remove {
        host: String,
        #[arg(long)]
        connection: Option<String>,
    },
}
#[derive(Subcommand)]
enum TrustCommand {
    #[command(about = "读取公钥候选；需通过可信来源核对指纹")]
    Scan {
        host: String,
        #[arg(long)]
        connection: Option<String>,
    },
    #[command(about = "查看已信任的主机公钥")]
    Show {
        host: String,
        #[arg(long)]
        connection: Option<String>,
    },
    #[command(about = "导入已有 known_hosts 中此主机的信任记录")]
    Import {
        host: String,
        #[arg(long)]
        connection: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, help = "替换此前信任的公钥")]
        replace: bool,
    },
    #[command(about = "确认已核对的指纹")]
    Accept {
        host: String,
        #[arg(long)]
        connection: Option<String>,
        #[arg(long)]
        fingerprint: String,
        #[arg(long, help = "替换此前信任的公钥")]
        replace: bool,
    },
}
#[derive(Subcommand)]
enum ContextCommand {
    Show {
        host: String,
    },
    #[command(about = "设置主机说明")]
    Describe {
        host: String,
        text: String,
    },
    #[command(about = "添加或更新项目路径")]
    Project {
        host: String,
        name: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    #[command(about = "删除项目记录")]
    RemoveProject {
        host: String,
        name: String,
    },
    #[command(about = "追加带时间的探索记录")]
    Note {
        host: String,
        text: String,
    },
}
pub(crate) fn credential_helper() -> Result<PathBuf> {
    Ok(std::env::current_exe()?.with_file_name("ssh-manager-credentials"))
}
fn absolute(p: &std::path::Path) -> Result<PathBuf> {
    let expanded = importer::expand_home(&p.to_string_lossy())?;
    Ok(if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    })
}
fn jumps(store: &Store, names: &[String]) -> Result<Vec<uuid::Uuid>> {
    names
        .iter()
        .map(|name| Ok(store.host(name)?.default_connection))
        .collect()
}
fn credential_refs(h: &Host) -> Vec<String> {
    h.connections
        .iter()
        .filter_map(|c| match &c.authentication {
            Authentication::Password { credential_ref } => credential_ref.clone(),
            Authentication::Key { passphrase_ref, .. } => passphrase_ref.clone(),
            _ => None,
        })
        .collect()
}
fn selected_index(h: &Host, selector: Option<&str>) -> Result<usize> {
    let id = h.connection(selector)?.id;
    Ok(h.connections
        .iter()
        .position(|c| c.id == id)
        .expect("selected connection belongs to host"))
}
fn to_value<T: serde::Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}
fn dispatch(cli: &Cli, cancel: Arc<AtomicBool>) -> Result<Value> {
    let root = match &cli.data_dir {
        Some(p) => absolute(p)?,
        None => Store::default_root()?,
    };
    let store = Store::open(root)?;
    let credentials = SystemCredentials {
        helper_executable: credential_helper()?,
    };
    let ssh = OpenSsh {
        store: &store,
        credentials: &credentials,
        askpass_executable: std::env::current_exe()?,
    };
    match &cli.command {
        Commands::Mcp => unreachable!("stdio server is dispatched before rendering"),
        Commands::Host { command } => match command {
            HostCommand::List { tag } => to_value(
                store
                    .list()?
                    .into_iter()
                    .filter(|h| tag.as_ref().is_none_or(|t| h.tags.contains(t)))
                    .collect::<Vec<_>>(),
            ),
            HostCommand::Show { host } => to_value(store.host(host)?),
            HostCommand::Add {
                name,
                address,
                user,
                port,
                tags,
                description,
                jump,
                auth,
            } => {
                let mut h = Host::new(
                    name.clone(),
                    address.clone(),
                    user.clone(),
                    *port,
                    auth.value()?,
                );
                h.tags = tags.clone();
                h.context.description = description.clone();
                h.connections[0].jumps = jumps(&store, jump)?;
                h.connections[0].identity_agent = auth.agent_socket()?;
                store.save(&h)?;
                to_value(h)
            }
            HostCommand::Edit {
                host,
                name,
                address,
                user,
                port,
                tags,
                description,
                jump,
                no_jump,
                clear_issues,
                no_identity_agent,
                connection,
                auth,
            } => {
                let mut h = store.host(host)?;
                let index = selected_index(&h, connection.as_deref())?;
                let c = &mut h.connections[index];
                if let Some(v) = address {
                    c.address = v.clone();
                }
                if let Some(v) = user {
                    c.user = v.clone();
                }
                if let Some(v) = port {
                    c.port = *v;
                }
                if let Some(v) = jump {
                    c.jumps = jumps(&store, v)?;
                }
                if *no_jump {
                    c.jumps.clear();
                }
                if *clear_issues {
                    c.issues.clear();
                }
                if auth.selected() {
                    c.identities_only = !auth.key.is_empty();
                    c.authentication = auth.value()?;
                }
                if auth.password {
                    c.identity_agent = None;
                }
                if auth.identity_agent.is_some() {
                    c.identity_agent = auth.agent_socket()?;
                }
                if *no_identity_agent {
                    c.identity_agent = None;
                }
                if let Some(v) = name {
                    h.name = v.clone();
                }
                if let Some(v) = tags {
                    h.tags = v.clone();
                }
                if let Some(v) = description {
                    h.context.description = v.clone();
                }
                store.save(&h)?;
                to_value(h)
            }
            HostCommand::Remove { host } => {
                let h = store.remove(host)?;
                let remaining = store
                    .list()?
                    .iter()
                    .flat_map(credential_refs)
                    .collect::<Vec<_>>();
                let mut cleanup = vec![];
                for reference in credential_refs(&h) {
                    if !remaining.contains(&reference) && credentials.delete(&reference).is_err() {
                        cleanup.push(reference);
                    }
                }
                Ok(json!({"removed":h.name,"id":h.id,"credentials_pending_cleanup":cleanup}))
            }
            HostCommand::Import {
                file,
                select,
                apply,
                ..
            } => {
                let file = file
                    .clone()
                    .unwrap_or(importer::home()?.join(".ssh/config"));
                let plan = importer::preview(&absolute(&file)?, select)?;
                if *apply {
                    let n = importer::apply(&store, plan)?;
                    Ok(json!({"imported":n}))
                } else {
                    to_value(plan)
                }
            }
            HostCommand::Export => to_value(store.export()?),
            HostCommand::Restore { file } => {
                let export = serde_json::from_str(&std::fs::read_to_string(file)?)?;
                let n = store.import_catalog(export)?;
                Ok(json!({"imported":n}))
            }
        },
        Commands::Connection { command } => match command {
            ConnectionCommand::List { host } => to_value(store.host(host)?.connections),
            ConnectionCommand::Add {
                host,
                name,
                address,
                user,
                port,
                jump,
                auth,
            } => {
                let mut h = store.host(host)?;
                let c = model::Connection {
                    id: uuid::Uuid::new_v4(),
                    name: name.clone(),
                    address: address.clone(),
                    user: user.clone(),
                    port: *port,
                    identities_only: !auth.key.is_empty(),
                    identity_agent: auth.agent_socket()?,
                    authentication: auth.value()?,
                    jumps: jumps(&store, jump)?,
                    issues: vec![],
                };
                h.connections.push(c.clone());
                store.save(&h)?;
                to_value(c)
            }
            ConnectionCommand::Default { host, connection } => {
                let mut h = store.host(host)?;
                h.default_connection = h.connection(Some(connection))?.id;
                store.save(&h)?;
                to_value(h)
            }
            ConnectionCommand::Remove { host, connection } => {
                let mut h = store.host(host)?;
                let id = h.connection(Some(connection))?.id;
                error::require(
                    id != h.default_connection,
                    "default_connection_in_use",
                    "请先选择其他默认登录配置",
                )?;
                error::require(
                    !store
                        .list()?
                        .iter()
                        .flat_map(|h| &h.connections)
                        .any(|c| c.jumps.contains(&id)),
                    "connection_in_use",
                    "此登录配置仍被用作跳板",
                )?;
                let index = selected_index(&h, Some(connection))?;
                let c = h.connections.remove(index);
                if let Some(r) = match c.authentication {
                    Authentication::Password { credential_ref } => credential_ref,
                    Authentication::Key { passphrase_ref, .. } => passphrase_ref,
                    _ => None,
                } {
                    credentials.delete(&r)?;
                }
                store.save(&h)?;
                Ok(json!({"removed":id}))
            }
        },
        Commands::Credential { command } => match command {
            CredentialCommand::Set {
                host,
                connection,
                passphrase,
                password_stdin,
            } => {
                let mut h = store.host(host)?;
                let i = selected_index(&h, connection.as_deref())?;
                if *passphrase {
                    error::require(
                        matches!(h.connections[i].authentication, Authentication::Key { .. }),
                        "key_required",
                        "请先为此登录配置选择密钥文件",
                    )?;
                }
                let secret = if *password_stdin {
                    let mut text = Zeroizing::new(String::new());
                    io::stdin().take(65537).read_to_string(&mut text)?;
                    error::require(text.len() <= 65536, "credential_too_long", "凭据过长")?;
                    while text.ends_with(['\n', '\r']) {
                        text.pop();
                    }
                    text
                } else {
                    error::require(
                        io::stdin().is_terminal(),
                        "needs_interaction",
                        "请在终端输入凭据，或通过 --password-stdin 从本机凭据工具读取",
                    )?;
                    Zeroizing::new(rpassword::prompt_password(if *passphrase {
                        "密钥口令："
                    } else {
                        "登录密码："
                    })?)
                };
                let previous = match &h.connections[i].authentication {
                    Authentication::Password { credential_ref } => credential_ref.clone(),
                    Authentication::Key { passphrase_ref, .. } => passphrase_ref.clone(),
                    _ => None,
                };
                let reference = uuid::Uuid::new_v4().to_string();
                credentials.set(&reference, &secret)?;
                if *passphrase {
                    if let Authentication::Key { passphrase_ref, .. } =
                        &mut h.connections[i].authentication
                    {
                        *passphrase_ref = Some(reference);
                    }
                } else {
                    h.connections[i].authentication = Authentication::Password {
                        credential_ref: Some(reference),
                    };
                }
                store.save(&h)?;
                let pending_cleanup =
                    previous.filter(|reference| credentials.delete(reference).is_err());
                Ok(
                    json!({"previous_credential_pending_cleanup":pending_cleanup,"saved":true,"host_id":h.id,"connection_id":h.connections[i].id,"purpose":if *passphrase{"key_passphrase"}else{"login_password"}}),
                )
            }
            CredentialCommand::Remove { host, connection } => {
                let mut h = store.host(host)?;
                let i = selected_index(&h, connection.as_deref())?;
                let reference = match &mut h.connections[i].authentication {
                    Authentication::Password { credential_ref } => credential_ref.take(),
                    Authentication::Key { passphrase_ref, .. } => passphrase_ref.take(),
                    _ => None,
                };
                if let Some(r) = reference {
                    credentials.delete(&r)?;
                }
                store.save(&h)?;
                Ok(json!({"removed":true}))
            }
        },
        Commands::Trust { command } => match command {
            TrustCommand::Scan { host, connection } => {
                to_value(trust::scan(&store, host, connection.as_deref())?)
            }
            TrustCommand::Show { host, connection } => to_value(trust::list(
                &store,
                store.host(host)?.connection(connection.as_deref())?.id,
            )?),
            TrustCommand::Import {
                host,
                connection,
                file,
                replace,
            } => to_value(trust::import_known_hosts(
                &store,
                host,
                connection.as_deref(),
                &file
                    .clone()
                    .unwrap_or(importer::home()?.join(".ssh/known_hosts")),
                *replace,
            )?),
            TrustCommand::Accept {
                host,
                connection,
                fingerprint,
                replace,
            } => to_value(trust::accept(
                &store,
                host,
                connection.as_deref(),
                fingerprint,
                *replace,
            )?),
        },
        Commands::Context { command } => {
            let host = match command {
                ContextCommand::Show { host }
                | ContextCommand::Describe { host, .. }
                | ContextCommand::Project { host, .. }
                | ContextCommand::RemoveProject { host, .. }
                | ContextCommand::Note { host, .. } => host,
            };
            let mut h = store.host(host)?;
            match command {
                ContextCommand::Show { .. } => {}
                ContextCommand::Describe { text, .. } => h.context.description = text.clone(),
                ContextCommand::Project {
                    name,
                    path,
                    description,
                    ..
                } => {
                    model::validate_name(name)?;
                    model::validate_absolute_path(path)?;
                    let p = Project {
                        name: name.clone(),
                        path: path.clone(),
                        description: description.clone(),
                    };
                    if let Some(existing) = h.context.projects.iter_mut().find(|p| p.name == *name)
                    {
                        *existing = p;
                    } else {
                        h.context.projects.push(p);
                    }
                }
                ContextCommand::RemoveProject { name, .. } => {
                    let n = h.context.projects.len();
                    h.context.projects.retain(|p| p.name != *name);
                    error::require(
                        h.context.projects.len() != n,
                        "project_not_found",
                        "未找到此项目",
                    )?;
                }
                ContextCommand::Note { text, .. } => {
                    error::require(!text.trim().is_empty(), "empty_note", "记录不能为空")?;
                    h.context.agent_notes.push(AgentNote {
                        text: text.clone(),
                        recorded_at: chrono::Utc::now(),
                    });
                }
            }
            store.save(&h)?;
            to_value(h.context)
        }
        Commands::Check { host, options } => to_value(ssh.check(
            host,
            options.connection.as_deref(),
            &options.execution(cancel),
        )?),
        Commands::Exec {
            host,
            cwd,
            project,
            options,
            command,
        } => {
            let project_path = project
                .as_ref()
                .map(|name| {
                    store
                        .host(host)?
                        .context
                        .projects
                        .into_iter()
                        .find(|p| p.name == *name)
                        .map(|p| p.path)
                        .ok_or_else(|| Error::new("project_not_found", "未找到此项目"))
                })
                .transpose()?;
            to_value(ssh.exec(
                host,
                options.connection.as_deref(),
                command,
                cwd.as_deref().or(project_path.as_deref()),
                &options.execution(cancel),
            )?)
        }
        Commands::Upload {
            host,
            source,
            destination,
            mode,
            options,
        } => to_value(ssh.upload(
            host,
            options.connection.as_deref(),
            source,
            destination,
            match mode {
                Mode::File => TransferMode::File,
                Mode::Contents => TransferMode::Contents,
            },
            &options.execution(cancel),
        )?),
        Commands::Download {
            host,
            source,
            destination,
            options,
        } => to_value(ssh.download(
            host,
            options.connection.as_deref(),
            source,
            destination,
            &options.execution(cancel),
        )?),
        Commands::History { host, limit } => to_value(store.history(host.as_deref(), *limit)?),
        Commands::Doctor => {
            let output = std::process::Command::new("ssh").arg("-V").output()?;
            let version = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let sftp = std::process::Command::new("sftp")
                .arg("-h")
                .output()
                .is_ok();
            Ok(
                json!({"schema_version":1,"ssh":version,"sftp_available":sftp,"data_directory":store.root(),"credential_backend":if cfg!(target_os="macos"){"macos_keychain"}else{"secret_service"},"credential_helper":credentials.helper_executable,"credential_helper_available":credentials.helper_executable.is_file(),"credential_interaction":if cfg!(target_os="macos"){"never"}else{"password_backend_not_ready"},"remote_shell":"POSIX sh","verified_platform":"macOS","config_import":"static OpenSSH","output_encoding":"UTF-8 with replacement for invalid bytes"}),
            )
        }
    }
}

fn render(value: &Value, cli: &Cli) -> Result<()> {
    if cli.json {
        println!("{}", serde_json::to_string_pretty(value)?);
        return Ok(());
    }
    match &cli.command {
        Commands::Host {
            command: HostCommand::List { .. },
        } => {
            let hosts: Vec<Host> = serde_json::from_value(value.clone())?;
            if hosts.is_empty() {
                println!("还没有主机，可添加一台服务器或导入 SSH 配置。");
            }
            for host in hosts {
                print_host(&host, false)?;
            }
            return Ok(());
        }
        Commands::Host {
            command: HostCommand::Show { .. },
        } => {
            let host: Host = serde_json::from_value(value.clone())?;
            print_host(&host, true)?;
            return Ok(());
        }
        Commands::Host {
            command: HostCommand::Import { apply: false, .. },
        } => {
            let plan: importer::ImportPlan = serde_json::from_value(value.clone())?;
            println!(
                "找到 {} 台主机，确认后可使用 --apply 保存。",
                plan.hosts.len()
            );
            for host in plan.hosts {
                print_host(&host, false)?;
            }
            for warning in plan.warnings {
                println!("提示：{warning}");
            }
            for notice in plan.skipped {
                println!("已跳过：{notice}");
            }
            for notice in plan.ignored {
                println!("已忽略：{notice}");
            }
            return Ok(());
        }
        _ => {}
    }
    if value.get("operation_id").is_some() {
        let r: OperationResult = serde_json::from_value(value.clone())?;
        let status = match r.status {
            Status::Succeeded => "已完成",
            Status::CommandFailed => "命令未成功",
            Status::ConnectionFailed => "连接失败",
            Status::AuthenticationFailed => "认证失败",
            Status::HostKeyRequired => "请确认服务器身份",
            Status::HostKeyChanged => "服务器身份已变化",
            Status::NeedsInteraction => "需要用户操作",
            Status::ResultUnknown => "结果未知",
            Status::TransferFailed => "传输未完成",
        };
        println!("{status} · {} ms", r.duration_ms);
        if let Some(receipt) = &r.transfer {
            println!("{} → {}", receipt.source, receipt.destination);
        }
        if !r.stdout.is_empty() {
            print!("{}", r.stdout);
        }
        if !r.stderr.is_empty() {
            eprint!("{}", r.stderr);
        }
        if r.stdout_truncated || r.stderr_truncated {
            eprintln!("\n输出已截断，可缩小查询范围后继续查看。");
        }
        if let Some(next) = r.next_step {
            eprintln!("\n{next}");
        }
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}
fn print_host(h: &Host, details: bool) -> Result<()> {
    let c = h.connection(None)?;
    let auth = match c.authentication {
        Authentication::Agent => "ssh-agent",
        Authentication::Key { .. } => "密钥",
        Authentication::Password { .. } => "密码",
    };
    println!(
        "{}  {}@{}:{}  {}{}",
        h.name,
        c.user,
        c.address,
        c.port,
        auth,
        if c.issues.is_empty() {
            ""
        } else {
            " · 待补全"
        }
    );
    if !h.context.description.is_empty() {
        println!("  {}", h.context.description);
    }
    for issue in &c.issues {
        println!("  提示：{issue}");
    }
    if let Some(socket) = &c.identity_agent {
        println!("  ssh-agent 套接字：{socket}");
    }
    if details {
        println!("  主机 ID：{}", h.id);
        for p in &h.context.projects {
            println!("  项目 {}：{}", p.name, p.path);
        }
        for note in &h.context.agent_notes {
            println!(
                "  {}  {}",
                note.recorded_at
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M"),
                note.text
            );
        }
        if let Some(facts) = &h.context.probed_facts {
            println!(
                "  最近检查：{} · {} {}",
                facts
                    .observed_at
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M"),
                facts.system,
                facts.architecture
            );
        }
        if h.connections.len() > 1 {
            println!(
                "  登录配置：{}",
                h.connections
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            );
        }
    }
    Ok(())
}
fn main() {
    if std::env::var_os("SSH_MANAGER_ASKPASS").is_some() {
        let prompt = std::env::args().nth(1).unwrap_or_default();
        std::process::exit(if credentials::askpass(&prompt).is_ok() {
            0
        } else {
            1
        });
    }
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ = e.print();
                std::process::exit(0);
            }
            // --json exec vm --timeout 0 must return a structured argument error without entering an interactive prompt.
            if std::env::args()
                .take_while(|s| s != "--")
                .any(|s| s == "--json")
            {
                println!(
                    "{}",
                    json!({"schema_version":1,"error":Error::new("invalid_arguments",e.to_string())})
                );
            } else {
                let _ = e.print();
            }
            std::process::exit(2);
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        signal.store(true, std::sync::atomic::Ordering::Relaxed);
    }) {
        eprintln!("无法处理取消信号：{e}");
        std::process::exit(2);
    }
    if matches!(cli.command, Commands::Mcp) {
        let result = (|| {
            let root = match &cli.data_dir {
                Some(p) => absolute(p)?,
                None => Store::default_root()?,
            };
            mcp::serve(root, cancel)
        })();
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(2);
        }
        return;
    }
    match dispatch(&cli, cancel) {
        Ok(value) => {
            let failed = value
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|s| s != "succeeded");
            if let Err(e) = render(&value, &cli) {
                eprintln!("{e}");
                std::process::exit(2);
            }
            if failed {
                std::process::exit(1);
            }
        }
        Err(error) => {
            if cli.json {
                println!("{}", json!({"schema_version":1,"error":error}));
            } else {
                eprintln!("{error}");
            }
            std::process::exit(2);
        }
    }
}
