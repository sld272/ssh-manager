use crate::{
    credentials::Broker,
    process::{self, ProcessResult},
    *,
};
use chrono::Utc;
use std::{
    collections::HashSet,
    io::Write,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct ExecutionOptions {
    pub timeout: Duration,
    pub output_limit: usize,
    pub cancel: Arc<AtomicBool>,
    pub caller: String,
    pub reason: Option<String>,
}
impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            output_limit: 65536,
            cancel: Arc::new(AtomicBool::new(false)),
            caller: "library".into(),
            reason: None,
        }
    }
}
/// An executable that implements credentials::askpass before parsing arguments (the CLI does).
/// Embedders pass a bundled CLI path; the core never assumes the embedding executable is an askpass helper.
pub struct OpenSsh<'a> {
    pub store: &'a Store,
    pub credentials: &'a dyn CredentialStore,
    pub askpass_executable: PathBuf,
}
pub(crate) struct Session {
    pub _directory: tempfile::TempDir,
    pub config: PathBuf,
    pub alias: String,
    pub broker: Broker,
}
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn config_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}
pub fn alias(id: Uuid) -> String {
    format!("sshm-{id}")
}
impl OpenSsh<'_> {
    fn collect(
        &self,
        c: &model::Connection,
        visiting: &mut HashSet<Uuid>,
        out: &mut Vec<model::Connection>,
    ) -> Result<()> {
        // Config A -> B -> A would otherwise recurse forever when generating a jump chain.
        error::require(visiting.insert(c.id), "jump_cycle", "跳板配置存在循环引用")?;
        for j in &c.jumps {
            self.collect(&self.store.connection(*j)?, visiting, out)?;
        }
        visiting.remove(&c.id);
        if !out.iter().any(|v| v.id == c.id) {
            out.push(c.clone());
        }
        Ok(())
    }
    pub(crate) fn session(&self, h: &Host, c: &model::Connection) -> Result<Session> {
        h.validate()?;
        let mut connections = vec![];
        self.collect(c, &mut HashSet::new(), &mut connections)?;
        for conn in &connections {
            error::require(
                conn.issues.is_empty(),
                "connection_incomplete",
                &format!("登录配置需要补全：{}", conn.issues.join("；")),
            )?;
            for path in match &conn.authentication {
                Authentication::Key { paths, .. } => paths.as_slice(),
                _ => &[],
            } {
                error::require(
                    std::path::Path::new(path).is_file(),
                    "identity_unavailable",
                    &format!("密钥文件不存在：{path}"),
                )?;
            }
            if let Authentication::Password {
                credential_ref: None,
            } = conn.authentication
            {
                return Err(Error::new("needs_interaction", "请先绑定此登录配置的密码"));
            }
        }
        let broker = Broker::start(&connections, self.credentials)?;
        // ProxyJump's implicit command interpolates -F without shell quoting; a private /tmp path avoids spaces in macOS Application Support.
        let directory = tempfile::Builder::new()
            .prefix("sshm-config-")
            .tempdir_in("/tmp")?;
        let config = directory.path().join("config");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&config)?;
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        writeln!(
            file,
            "Host *\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile /dev/null\n  StrictHostKeyChecking yes\n  UpdateHostKeys no\n  CheckHostIP no\n  ForwardAgent no\n  ForwardX11 no\n  PermitLocalCommand no\n  ControlMaster no\n  ControlPath none\n  RequestTTY no\n  ConnectTimeout 10\n  ConnectionAttempts 1\n  ServerAliveInterval 5\n  ServerAliveCountMax 2\n  NumberOfPasswordPrompts 1\n  KbdInteractiveAuthentication no\n  LogLevel ERROR",
            config_quote(&self.store.root().join("known_hosts").to_string_lossy())
        )?;
        for conn in &connections {
            writeln!(
                file,
                "Host {}\n  HostName {}\n  User {}\n  Port {}\n  HostKeyAlias {}",
                alias(conn.id),
                conn.address,
                conn.user,
                conn.port,
                alias(conn.id)
            )?;
            match &conn.authentication {
                Authentication::Password { .. } => writeln!(
                    file,
                    "  PreferredAuthentications password\n  PubkeyAuthentication no\n  PasswordAuthentication yes\n  BatchMode no"
                )?,
                Authentication::Agent => writeln!(
                    file,
                    "  PreferredAuthentications publickey\n  IdentityFile none\n  PasswordAuthentication no\n  BatchMode no\n  IdentitiesOnly {}",
                    if conn.identities_only { "yes" } else { "no" }
                )?,
                Authentication::Key { paths, .. } => {
                    writeln!(
                        file,
                        "  PreferredAuthentications publickey\n  IdentitiesOnly {}\n  PasswordAuthentication no\n  BatchMode no",
                        if conn.identities_only { "yes" } else { "no" }
                    )?;
                    for path in paths {
                        writeln!(file, "  IdentityFile {}", config_quote(path))?;
                    }
                }
            }
            if !conn.jumps.is_empty() {
                writeln!(
                    file,
                    "  ProxyJump {}",
                    conn.jumps
                        .iter()
                        .map(|id| alias(*id))
                        .collect::<Vec<_>>()
                        .join(",")
                )?;
            }
        }
        Ok(Session {
            _directory: directory,
            config,
            alias: alias(c.id),
            broker,
        })
    }
    pub(crate) fn command(&self, program: &str, session: &Session) -> Command {
        let mut cmd = Command::new(program);
        cmd.arg("-F")
            .arg(&session.config)
            .env("LC_ALL", "C")
            .env("SSH_ASKPASS", &self.askpass_executable)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", "ssh-manager:0")
            .env("SSH_MANAGER_ASKPASS", "1")
            .env("SSH_MANAGER_AUTH_SOCKET", &session.broker.socket)
            .env("SSH_MANAGER_AUTH_TOKEN", &session.broker.token);
        cmd
    }
    pub fn exec(
        &self,
        host: &str,
        connection: Option<&str>,
        command: &str,
        cwd: Option<&str>,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        self.exec_kind(host, connection, command, cwd, options, "exec")
    }
    fn exec_kind(
        &self,
        host: &str,
        connection: Option<&str>,
        command: &str,
        cwd: Option<&str>,
        options: &ExecutionOptions,
        kind: &str,
    ) -> Result<OperationResult> {
        error::require(
            !command.is_empty() && !command.contains('\0'),
            "invalid_command",
            "命令不能为空或包含空字符",
        )?;
        error::require(
            !options.timeout.is_zero() && (1..=16777216).contains(&options.output_limit),
            "invalid_options",
            "超时和输出上限必须大于零",
        )?;
        if let Some(p) = cwd {
            model::validate_absolute_path(p)?;
        }
        let h = self.store.host(host)?;
        let c = h.connection(connection)?;
        let started_at = Utc::now();
        let clock = Instant::now();
        let operation_id = Uuid::new_v4();
        let session = self.session(&h, c)?;
        let token = Uuid::new_v4().simple().to_string();
        let script = match cwd {
            Some(path) => format!("cd {} && {}", shell_quote(path), command),
            None => command.into(),
        };
        let wrapper = format!(
            "printf '\\036{token}:start\\037' >&2; sh -c {}; result=$?; printf '\\036{token}:exit:%s\\037' \"$result\" >&2; exit \"$result\"",
            shell_quote(&script)
        );
        let mut cmd = self.command("ssh", &session);
        cmd.args(["-T", "-n", "--", &session.alias, &wrapper]);
        let raw = process::run(
            cmd,
            None,
            options.timeout,
            options.cancel.clone(),
            options.output_limit + 256,
            Some(token.clone()),
        )?;
        let result = self.result(
            &h,
            c,
            kind,
            operation_id,
            started_at,
            clock.elapsed(),
            raw,
            &session.broker,
            options.output_limit,
            Some(&token),
        );
        self.store
            .record(&result, &h.name, &options.caller, options.reason.as_deref())?;
        Ok(result)
    }
    pub fn check(
        &self,
        host: &str,
        connection: Option<&str>,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        let result = self.exec_kind(
            host,
            connection,
            "uname -s; uname -m; id -un; printf '%s\\n' \"$HOME\"",
            None,
            options,
            "check",
        )?;
        if result.successful() {
            let lines: Vec<_> = result.stdout.lines().collect();
            if lines.len() == 4 {
                let mut h = self.store.host(host)?;
                h.context.probed_facts = Some(ProbedFacts {
                    observed_at: Utc::now(),
                    system: lines[0].into(),
                    architecture: lines[1].into(),
                    user: lines[2].into(),
                    home: lines[3].into(),
                });
                self.store.save(&h)?;
            }
        }
        Ok(result)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn result(
        &self,
        h: &Host,
        c: &model::Connection,
        kind: &str,
        operation_id: Uuid,
        started_at: chrono::DateTime<Utc>,
        duration: Duration,
        raw: ProcessResult,
        broker: &Broker,
        limit: usize,
        token: Option<&str>,
    ) -> OperationResult {
        let mut stderr = String::from_utf8_lossy(&raw.stderr.bytes).into_owned();
        if let Some(t) = token {
            stderr = stderr.replace(&format!("\x1e{t}:start\x1f"), "");
            if let Some(code) = raw.stderr.remote_exit {
                stderr = stderr.replace(&format!("\x1e{t}:exit:{code}\x1f"), "");
            }
            // The output limit can end in the middle of our final control record; never expose a fragment to callers.
            if let Some(i) = stderr.find(&format!("\x1e{t}")) {
                stderr.truncate(i);
            }
        }
        let mut stdout = String::from_utf8_lossy(&raw.stdout.bytes).into_owned();
        let phase = if raw.stderr.remote_exit.is_some() {
            ExecutionPhase::Completed
        } else if raw.stderr.started {
            ExecutionPhase::Started
        } else {
            ExecutionPhase::Unknown
        };
        let io_failed = raw.stdout.io_error || raw.stderr.io_error;
        let (status, next_step) = if phase == ExecutionPhase::Completed && !io_failed {
            (
                if raw.stderr.remote_exit == Some(0) {
                    Status::Succeeded
                } else {
                    Status::CommandFailed
                },
                None,
            )
        } else if phase == ExecutionPhase::Started || raw.termination.is_some() || io_failed {
            (
                Status::ResultUnknown,
                Some("远端操作可能仍在运行，请先检查实际结果，再决定后续操作".into()),
            )
        } else if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
            || stderr.contains("Offending ")
        {
            (
                Status::HostKeyChanged,
                Some("服务器身份发生变化，请核实后重新建立信任".into()),
            )
        } else if stderr.contains("Host key verification failed")
            || stderr.contains("No ED25519 host key is known")
            || stderr.contains("No ECDSA host key is known")
            || stderr.contains("No RSA host key is known")
        {
            (
                Status::HostKeyRequired,
                Some("请导入已有的主机信任记录，或核对指纹后确认".into()),
            )
        } else if broker.denied.load(Ordering::Relaxed) {
            (
                Status::NeedsInteraction,
                Some("认证需要用户操作，请检查密钥口令或硬件密钥".into()),
            )
        } else if stderr.contains("Permission denied") {
            (
                Status::AuthenticationFailed,
                Some("请检查登录账号及绑定的凭据".into()),
            )
        } else if [
            "Connection refused",
            "Could not resolve hostname",
            "Network is unreachable",
            "No route to host",
            "Connection timed out",
        ]
        .iter()
        .any(|p| stderr.contains(p))
        {
            (
                Status::ConnectionFailed,
                Some("请检查地址、端口、网络和跳板连接".into()),
            )
        } else {
            (
                Status::ResultUnknown,
                Some("未能确认远端执行结果，请检查服务器上的实际状态".into()),
            )
        };
        let phase = if matches!(
            status,
            Status::HostKeyChanged
                | Status::HostKeyRequired
                | Status::NeedsInteraction
                | Status::AuthenticationFailed
                | Status::ConnectionFailed
        ) {
            ExecutionPhase::NotStarted
        } else {
            phase
        };
        let stdout_truncated = raw.stdout.total > limit as u64;
        let overhead = token.map_or(0, |t| {
            (format!("\x1e{t}:start\x1f").len() * usize::from(raw.stderr.started)
                + raw
                    .stderr
                    .remote_exit
                    .map_or(0, |code| format!("\x1e{t}:exit:{code}\x1f").len())) as u64
        });
        let stderr_bytes = raw.stderr.total.saturating_sub(overhead);
        let stderr_truncated = stderr_bytes > limit as u64;
        truncate_utf8(&mut stdout, limit);
        truncate_utf8(&mut stderr, limit);
        broker.redact(&mut stdout);
        broker.redact(&mut stderr);
        OperationResult {
            schema_version: 1,
            operation_id,
            host_id: h.id,
            connection_id: c.id,
            kind: kind.into(),
            started_at,
            duration_ms: duration.as_millis() as u64,
            status,
            phase,
            exit_code: raw.stderr.remote_exit,
            termination: raw.termination,
            stdout,
            stderr,
            stdout_bytes: raw.stdout.total,
            stderr_bytes,
            stdout_truncated,
            stderr_truncated,
            next_step,
            transfer: None,
        }
    }
}
fn truncate_utf8(s: &mut String, limit: usize) {
    if s.len() > limit {
        let mut i = limit;
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        s.truncate(i);
    }
}
