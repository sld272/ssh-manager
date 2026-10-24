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
        self.prepare(h, c, false)
    }
    /// An interactive session lets the person type what was not saved, so it needs no stored password.
    fn prepare(&self, h: &Host, c: &model::Connection, interactive: bool) -> Result<Session> {
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
            // 1Password or Secretive removes its socket while the app is not running.
            if let (Some(socket), Authentication::Agent | Authentication::Key { .. }) =
                (&conn.identity_agent, &conn.authentication)
            {
                error::require(
                    std::path::Path::new(socket).exists(),
                    "agent_unavailable",
                    &format!("ssh-agent 套接字不存在：{socket}，请确认提供密钥的应用正在运行"),
                )?;
            }
            if !interactive
                && let Authentication::Password {
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
            "Host *\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile /dev/null\n  UpdateHostKeys no\n  CheckHostIP no\n  ForwardAgent no\n  ForwardX11 no\n  PermitLocalCommand no\n  ControlMaster no\n  ControlPath none\n  ConnectTimeout 10\n  ConnectionAttempts 1\n  ServerAliveInterval 5\n  ServerAliveCountMax 2\n  NumberOfPasswordPrompts 1\n  KbdInteractiveAuthentication no\n  LogLevel ERROR",
            config_quote(&self.store.root().join("known_hosts").to_string_lossy())
        )?;
        // Still inside "Host *": operations never get a terminal, an interactive session does.
        if !interactive {
            writeln!(file, "  RequestTTY no")?;
        }
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
            // A login with no saved key records the one its server presents on this first connection; once a key
            // is saved, any other key is refused, including one of a type that was not recorded.
            writeln!(
                file,
                "  StrictHostKeyChecking {}",
                if crate::trust::list(self.store, conn.id)?.is_empty() {
                    "accept-new"
                } else {
                    "yes"
                }
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
            if let (Some(socket), Authentication::Agent | Authentication::Key { .. }) =
                (&conn.identity_agent, &conn.authentication)
            {
                writeln!(file, "  IdentityAgent {}", config_quote(socket))?;
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
    /// Opens a login shell on the caller's own terminal and returns ssh's exit code once it ends.
    /// Saved passwords and passphrases are answered for the person; anything else they type.
    pub fn interactive(&self, host: &str, connection: Option<&str>) -> Result<i32> {
        let h = self.store.host(host)?;
        let c = h.connection(connection)?;
        let session = self.prepare(&h, c, true)?;
        let mut cmd = Command::new("ssh");
        cmd.arg("-F").arg(&session.config);
        if session.broker.answers_prompts() {
            cmd.env("SSH_ASKPASS", &self.askpass_executable)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", "ssh-manager:0")
                .env("SSH_MANAGER_ASKPASS", "1")
                .env("SSH_MANAGER_AUTH_SOCKET", &session.broker.socket)
                .env("SSH_MANAGER_AUTH_TOKEN", &session.broker.token);
        }
        let status = cmd
            .args(["--", &session.alias])
            .status()
            .map_err(|e| Error::new("process_start_failed", format!("无法启动 SSH 工具：{e}")))?;
        Ok(status.code().unwrap_or(255))
    }
    /// `stdin` is passed to the command, so a script can go to `python3 -` without another layer of quoting.
    pub fn exec(
        &self,
        host: &str,
        connection: Option<&str>,
        command: &str,
        cwd: Option<&str>,
        stdin: Option<Vec<u8>>,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        self.exec_kind(host, connection, command, cwd, stdin, options, "exec")
    }
    #[allow(clippy::too_many_arguments)]
    fn exec_kind(
        &self,
        host: &str,
        connection: Option<&str>,
        command: &str,
        cwd: Option<&str>,
        stdin: Option<Vec<u8>>,
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
        let prepared = clock.elapsed();
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
        cmd.arg("-T");
        if stdin.is_none() {
            cmd.arg("-n");
        }
        cmd.args(["--", &session.alias, &wrapper]);
        let raw = process::run(
            cmd,
            stdin,
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
            prepared,
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
        prepared: Duration,
        duration: Duration,
        raw: ProcessResult,
        broker: &Broker,
        limit: usize,
        token: Option<&str>,
    ) -> OperationResult {
        // ssh's own error messages are short and come first, so the kept head holds them for the checks below.
        let stderr = String::from_utf8_lossy(&raw.stderr.bytes).into_owned();
        let phase = if raw.stderr.remote_exit.is_some() {
            ExecutionPhase::Completed
        } else if raw.stderr.started.is_some() {
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
        } else if stderr.contains("Host key verification failed") {
            // Only a saved login refuses a key: either it changed, or the server now offers a type that was never recorded.
            (
                Status::HostKeyChanged,
                Some("服务器身份发生变化，请核实后重新建立信任".into()),
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
            (format!("\x1e{t}:start\x1f").len() * usize::from(raw.stderr.started.is_some())
                + raw
                    .stderr
                    .remote_exit
                    .map_or(0, |code| format!("\x1e{t}:exit:{code}\x1f").len())) as u64
        });
        let stderr_bytes = raw.stderr.total.saturating_sub(overhead);
        let stderr_truncated = stderr_bytes > limit as u64;
        let (stdout, stdout_tail) =
            excerpt(&raw.stdout, stdout_truncated, limit, |s| broker.redact(s));
        let (stderr, stderr_tail) = excerpt(&raw.stderr, stderr_truncated, limit, |s| {
            if let Some(t) = token {
                *s = s.replace(&format!("\x1e{t}:start\x1f"), "");
                if let Some(code) = raw.stderr.remote_exit {
                    *s = s.replace(&format!("\x1e{t}:exit:{code}\x1f"), "");
                }
                // A dropped connection can end in the middle of our final control record; never expose a fragment.
                if let Some(i) = s.find(&format!("\x1e{t}")) {
                    s.truncate(i);
                }
            }
            broker.redact(s)
        });
        OperationResult {
            schema_version: 1,
            operation_id,
            host_id: h.id,
            connection_id: c.id,
            kind: kind.into(),
            started_at,
            duration_ms: duration.as_millis() as u64,
            prepare_ms: prepared.as_millis() as u64,
            connect_ms: raw.stderr.started.map(|d| d.as_millis() as u64),
            status,
            phase,
            exit_code: raw.stderr.remote_exit,
            termination: raw.termination,
            stdout,
            stderr,
            stdout_tail,
            stderr_tail,
            stdout_bytes: raw.stdout.total,
            stderr_bytes,
            stdout_truncated,
            stderr_truncated,
            next_step,
            transfer: None,
        }
    }
}
/// A stream within the limit comes back whole; a longer one as its first and last halves of the limit.
/// `clean` runs before cutting, so a secret the output holds in full is redacted even where the cut would split it.
fn excerpt(
    capture: &process::Capture,
    truncated: bool,
    limit: usize,
    clean: impl Fn(&mut String),
) -> (String, String) {
    let text = |bytes: &[u8]| {
        let mut s = String::from_utf8_lossy(bytes).into_owned();
        clean(&mut s);
        s
    };
    let (mut head, tail) = match capture.whole() {
        Some(whole) if !truncated => return (text(&whole), String::new()),
        Some(whole) => {
            let s = text(&whole);
            let tail = suffix_utf8(&s, limit / 2).to_owned();
            (s, tail)
        }
        None => {
            let bytes: Vec<u8> = capture.tail.iter().copied().collect();
            // The kept tail can begin inside a multi-byte character; skip its continuation bytes.
            let start = bytes
                .iter()
                .take(3)
                .take_while(|b| **b & 0xC0 == 0x80)
                .count();
            (
                text(&capture.bytes),
                suffix_utf8(&text(&bytes[start..]), limit / 2).to_owned(),
            )
        }
    };
    truncate_utf8(&mut head, limit - limit / 2);
    (head, tail)
}
fn suffix_utf8(s: &str, limit: usize) -> &str {
    let mut i = s.len().saturating_sub(limit);
    while !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(head: &[u8], tail: &[u8], total: u64) -> process::Capture {
        process::Capture {
            bytes: head.to_vec(),
            tail: tail.iter().copied().collect(),
            total,
            started: None,
            remote_exit: None,
            io_error: false,
        }
    }

    #[test]
    fn excerpt_returns_halves_of_a_long_stream() {
        // The kept tail starts inside "尾", and a four-byte half only fits the whole "束".
        let c = capture(b"first-part", &"尾部结束".as_bytes()[1..], 100_000);
        assert_eq!(excerpt(&c, true, 8, |_| {}), ("firs".into(), "束".into()));
    }

    #[test]
    fn excerpt_redacts_before_cutting() {
        let c = capture(b"token=secret-v", b"alue done", 23);
        let (head, tail) = excerpt(&c, true, 16, |s| {
            *s = s.replace("secret-value", "[REDACTED]")
        });
        assert_eq!((head.as_str(), tail.as_str()), ("token=[R", "ED] done"));
    }
}
