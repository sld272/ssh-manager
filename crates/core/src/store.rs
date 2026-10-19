use crate::{error::require, *};
use rusqlite::{Connection as Db, OptionalExtension, params};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

pub struct Store {
    db: Db,
    root: PathBuf,
}
pub fn private_dir(path: &Path) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
impl Store {
    pub fn default_root() -> Result<PathBuf> {
        directories::ProjectDirs::from("dev", "ssh-manager", "ssh-manager")
            .map(|p| p.data_local_dir().to_path_buf())
            .ok_or_else(|| {
                Error::new(
                    "data_directory_unavailable",
                    "无法确定本机数据目录，请通过 --data-dir 指定",
                )
            })
    }
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        private_dir(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        let path = root.join("catalog.sqlite3");
        let db = Db::open(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        // Two MCP requests can open a new catalog simultaneously; serialize the version check and schema creation.
        db.execute_batch("BEGIN IMMEDIATE")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        // A catalog created by a newer release (user_version=2) must not be rewritten as v1.
        require(
            version <= 1,
            "unsupported_schema",
            "数据由较新版本创建，请更新 ssh-manager",
        )?;
        if version == 0 {
            db.execute_batch("CREATE TABLE hosts(id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, body TEXT NOT NULL); CREATE TABLE operations(id TEXT PRIMARY KEY, host_id TEXT NOT NULL, started_at TEXT NOT NULL, body TEXT NOT NULL); CREATE INDEX operations_host ON operations(host_id, started_at); PRAGMA user_version=1;")?;
        }
        db.execute_batch("COMMIT")?;
        Ok(Self { db, root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn list(&self) -> Result<Vec<Host>> {
        let mut q = self.db.prepare("SELECT body FROM hosts ORDER BY name")?;
        let rows = q.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|v| Ok(serde_json::from_str(&v?)?)).collect()
    }
    pub fn host(&self, selector: &str) -> Result<Host> {
        let body: Option<String> = self
            .db
            .query_row(
                "SELECT body FROM hosts WHERE id=?1 OR name=?1",
                [selector],
                |r| r.get(0),
            )
            .optional()?;
        body.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .unwrap_or_else(|| {
                Err(Error::new(
                    "host_not_found",
                    format!("未找到主机：{selector}"),
                ))
            })
    }
    pub fn save(&self, host: &Host) -> Result<()> {
        host.validate()?;
        let all = self.list()?;
        require(
            !all.iter().any(|h| h.id != host.id && h.name == host.name),
            "duplicate_host",
            "此主机名称已存在",
        )?;
        for c in &host.connections {
            require(
                !all.iter()
                    .filter(|h| h.id != host.id)
                    .any(|h| h.connections.iter().any(|other| other.id == c.id)),
                "duplicate_connection",
                "登录配置 ID 已被其他主机使用",
            )?;
        }
        self.db.execute("INSERT INTO hosts(id,name,body) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,body=excluded.body", params![host.id.to_string(), host.name, serde_json::to_string(host)?])?;
        Ok(())
    }
    pub fn remove(&self, selector: &str) -> Result<Host> {
        let h = self.host(selector)?;
        let ids: Vec<_> = h.connections.iter().map(|c| c.id).collect();
        require(
            !self.list()?.iter().filter(|v| v.id != h.id).any(|v| {
                v.connections
                    .iter()
                    .any(|c| c.jumps.iter().any(|j| ids.contains(j)))
            }),
            "host_in_use",
            "此主机被其他主机用作跳板，请先修改跳板配置",
        )?;
        self.db
            .execute("DELETE FROM hosts WHERE id=?1", [h.id.to_string()])?;
        Ok(h)
    }
    pub fn connection(&self, id: uuid::Uuid) -> Result<model::Connection> {
        self.list()?
            .into_iter()
            .flat_map(|h| h.connections)
            .find(|c| c.id == id)
            .ok_or_else(|| Error::new("missing_jump", format!("跳板登录配置不存在：{id}")))
    }
    pub fn record(
        &self,
        result: &OperationResult,
        name: &str,
        caller: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let r = OperationRecord {
            operation_id: result.operation_id,
            host_id: result.host_id,
            connection_id: result.connection_id,
            host_name: name.into(),
            kind: result.kind.clone(),
            started_at: result.started_at,
            duration_ms: result.duration_ms,
            status: result.status,
            phase: result.phase,
            exit_code: result.exit_code,
            termination: result.termination.clone(),
            caller: caller.into(),
            reason: reason.map(str::to_owned),
        };
        self.db.execute(
            "INSERT INTO operations VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET body=excluded.body",
            params![
                r.operation_id.to_string(),
                r.host_id.to_string(),
                r.started_at.to_rfc3339(),
                serde_json::to_string(&r)?
            ],
        )?;
        Ok(())
    }
    pub fn history(&self, host: Option<&str>, limit: u32) -> Result<Vec<OperationRecord>> {
        let host_id = host
            .map(|s| self.host(s).map(|h| h.id.to_string()))
            .transpose()?;
        let mut q = self.db.prepare("SELECT body FROM operations WHERE (?1 IS NULL OR host_id=?1) ORDER BY started_at DESC LIMIT ?2")?;
        let rows = q.query_map(params![host_id, limit], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn export(&self) -> Result<CatalogExport> {
        let mut hosts = self.list()?;
        for h in &mut hosts {
            for c in &mut h.connections {
                match &mut c.authentication {
                    Authentication::Password { credential_ref } => *credential_ref = None,
                    Authentication::Key { passphrase_ref, .. } => *passphrase_ref = None,
                    Authentication::Agent => {}
                }
            }
        }
        Ok(CatalogExport {
            schema_version: 1,
            hosts,
        })
    }
    pub fn import_catalog(&self, export: CatalogExport) -> Result<usize> {
        require(
            export.schema_version == 1,
            "unsupported_schema",
            "不支持此配置文件版本",
        )?;
        let current = self.list()?;
        let mut names = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        let mut conn_ids: std::collections::HashSet<_> = current
            .iter()
            .flat_map(|h| h.connections.iter().map(|c| c.id))
            .collect();
        for h in &export.hosts {
            h.validate()?;
            require(
                names.insert(&h.name)
                    && ids.insert(h.id)
                    && !current.iter().any(|v| v.name == h.name || v.id == h.id),
                "catalog_conflict",
                "导入包含重复主机；请先处理名称和 ID 冲突",
            )?;
            for c in &h.connections {
                require(
                    conn_ids.insert(c.id),
                    "catalog_conflict",
                    "导入包含重复登录配置",
                )?;
            }
        }
        for h in &export.hosts {
            for c in &h.connections {
                for j in &c.jumps {
                    require(conn_ids.contains(j), "missing_jump", "导入缺少跳板配置")?;
                }
            }
        }
        let tx = self.db.unchecked_transaction()?;
        for mut h in export.hosts.clone() {
            for c in &mut h.connections {
                match &mut c.authentication {
                    Authentication::Password { credential_ref } => *credential_ref = None,
                    Authentication::Key { passphrase_ref, .. } => *passphrase_ref = None,
                    Authentication::Agent => {}
                }
            }
            self.save(&h)?;
        }
        tx.commit()?;
        Ok(export.hosts.len())
    }
}
