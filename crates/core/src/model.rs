use crate::{Result, error::require};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Authentication {
    Agent,
    Key {
        paths: Vec<String>,
        passphrase_ref: Option<String>,
    },
    Password {
        credential_ref: Option<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: Uuid,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub user: String,
    pub authentication: Authentication,
    // Catalogs created before the first VM verification did not store IdentitiesOnly; their default was false.
    #[serde(default)]
    pub identities_only: bool,
    /// Agent socket for key and agent authentication, such as the one 1Password or Secretive provides; None uses SSH_AUTH_SOCK.
    #[serde(default)]
    pub identity_agent: Option<String>,
    /// Ordered connection IDs; all hops are local OpenSSH connections.
    pub jumps: Vec<Uuid>,
    pub issues: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub path: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentNote {
    pub text: String,
    pub recorded_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbedFacts {
    pub observed_at: DateTime<Utc>,
    pub system: String,
    pub architecture: String,
    pub user: String,
    pub home: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    pub description: String,
    pub projects: Vec<Project>,
    pub agent_notes: Vec<AgentNote>,
    pub probed_facts: Option<ProbedFacts>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSource {
    pub path: String,
    pub alias: String,
    pub imported_at: DateTime<Utc>,
    pub snapshot: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: Uuid,
    pub name: String,
    pub tags: Vec<String>,
    pub connections: Vec<Connection>,
    pub default_connection: Uuid,
    pub context: Context,
    pub source: Option<ImportSource>,
}
impl Host {
    pub fn new(
        name: String,
        address: String,
        user: String,
        port: u16,
        authentication: Authentication,
    ) -> Self {
        let id = Uuid::new_v4();
        Self {
            id: Uuid::new_v4(),
            name,
            tags: vec![],
            connections: vec![Connection {
                id,
                name: "default".into(),
                address,
                port,
                user,
                identities_only: matches!(&authentication, Authentication::Key { .. }),
                identity_agent: None,
                authentication,
                jumps: vec![],
                issues: vec![],
            }],
            default_connection: id,
            context: Context::default(),
            source: None,
        }
    }
    pub fn connection(&self, selector: Option<&str>) -> Result<&Connection> {
        self.connections
            .iter()
            .find(|c| {
                selector.map_or(c.id == self.default_connection, |s| {
                    c.name == s || c.id.to_string() == s
                })
            })
            .ok_or_else(|| crate::Error::new("connection_not_found", "未找到登录配置"))
    }
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        require(
            !self.connections.is_empty(),
            "invalid_host",
            "主机至少需要一个登录配置",
        )?;
        require(
            self.connections
                .iter()
                .any(|c| c.id == self.default_connection),
            "invalid_host",
            "默认登录配置不存在",
        )?;
        let mut names = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for c in &self.connections {
            validate_name(&c.name)?;
            require(
                names.insert(&c.name) && ids.insert(c.id),
                "invalid_connection",
                "登录配置名称和 ID 不能重复",
            )?;
            require(
                !c.address.is_empty()
                    && !c.address.starts_with('-')
                    && c.address
                        .chars()
                        .all(|v| v.is_ascii_alphanumeric() || ".:-_%".contains(v)),
                "invalid_address",
                "请输入域名、IPv4 或 IPv6 地址",
            )?;
            require(
                !c.user.is_empty()
                    && !c.user.starts_with('-')
                    && c.user
                        .chars()
                        .all(|v| v.is_ascii_alphanumeric() || "._-@".contains(v)),
                "invalid_user",
                "登录用户名包含不支持的字符",
            )?;
            require(c.port > 0, "invalid_port", "端口必须在 1 到 65535 之间")?;
            if let Some(p) = &c.identity_agent {
                validate_absolute_path(p)?;
            }
            if let Authentication::Key { paths, .. } = &c.authentication {
                require(
                    !paths.is_empty(),
                    "invalid_identity",
                    "至少需要一个密钥文件",
                )?;
                for p in paths {
                    validate_absolute_path(p)?;
                }
            }
        }
        let mut projects = std::collections::HashSet::new();
        for p in &self.context.projects {
            validate_name(&p.name)?;
            validate_absolute_path(&p.path)?;
            require(
                projects.insert(&p.name),
                "duplicate_project",
                "项目名称不能重复",
            )?;
        }
        Ok(())
    }
}
// A selector such as "web/production" or "--help" must not become a config alias or CLI option.
pub fn validate_name(s: &str) -> Result<()> {
    require(
        !s.is_empty()
            && s.len() <= 128
            && !s.starts_with('-')
            && s.chars().all(|c| c.is_alphanumeric() || "_.-".contains(c)),
        "invalid_name",
        "名称可使用文字、数字、点、下划线和连字符",
    )
}
// A relative cwd such as "../srv" is ambiguous, and a path containing a newline would create a second SFTP batch command.
pub fn validate_absolute_path(s: &str) -> Result<()> {
    require(
        s.starts_with('/') && !s.chars().any(|c| c.is_control()),
        "invalid_path",
        "请提供不含控制字符的绝对路径",
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Succeeded,
    CommandFailed,
    ConnectionFailed,
    AuthenticationFailed,
    /// Recorded by releases that needed a server confirmed before its first connection; their history still reads.
    HostKeyRequired,
    HostKeyChanged,
    NeedsInteraction,
    ResultUnknown,
    TransferFailed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    NotStarted,
    Started,
    Completed,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub host_id: Uuid,
    pub connection_id: Uuid,
    pub kind: String,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub status: Status,
    pub phase: ExecutionPhase,
    pub exit_code: Option<i32>,
    pub termination: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub next_step: Option<String>,
    pub transfer: Option<TransferReceipt>,
}
impl OperationResult {
    pub fn successful(&self) -> bool {
        self.status == Status::Succeeded
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferReceipt {
    pub source: String,
    pub destination: String,
    pub direction: String,
    pub mode: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    pub operation_id: Uuid,
    pub host_id: Uuid,
    pub connection_id: Uuid,
    pub host_name: String,
    pub kind: String,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub status: Status,
    pub phase: ExecutionPhase,
    pub exit_code: Option<i32>,
    pub termination: Option<String>,
    pub caller: String,
    pub reason: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogExport {
    pub schema_version: u32,
    pub hosts: Vec<Host>,
}
