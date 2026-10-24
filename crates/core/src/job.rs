//! Background jobs run under setsid on the server and keep their state in a per-job directory, so a task can
//! outlive the connection that started it and still be checked, read and stopped by later operations.
use crate::{ssh::shell_quote, *};
use uuid::Uuid;

/// Shared by every job script. Every process a job starts inherits SSH_MANAGER_JOB, so `procs` finds them all,
/// including ones that left its process group, and never mistakes a recycled pid for one of them.
const PRELUDE: &str = r#"d="${XDG_STATE_HOME:-$HOME/.local/state}/ssh-manager/jobs"
j="$d/$id"
procs() { grep -lsxz "SSH_MANAGER_JOB=$1" /proc/[0-9]*/environ | cut -d/ -f3; }
state() {
  if [ -f "$j/exit" ]; then
    if [ -f "$j/cancelled" ]; then echo "cancelled $(cat "$j/exit")"; else echo "exited $(cat "$j/exit")"; fi
  elif [ -n "$(procs "$id")" ]; then echo "running -"
  elif [ -f "$j/cancelled" ]; then echo "cancelled -"
  else echo "lost -"; fi
}
report() {
  set -- "$1" "$2" $(state)
  size=$(($(wc -c < "$j/output")))
  if [ "$1" = tail ]; then start=$((size > $2 ? size - $2 : 0)); else start=$(($1 < size ? $1 : size)); fi
  printf '\036%s:job %s %s %s %s\037' "$token" "$3" "$4" "$size" "$start"
  if [ "$2" -gt 0 ]; then tail -c +$((start + 1)) "$j/output" | head -c "$2"; fi
}
"#;
const MISSING: &str = r#"[ -d "$j" ] || { printf '\036%s:job missing\037' "$token"; exit 0; }
"#;
/// Runs in the job's own session. It outlives TERM so that it can still record how the command ended.
const RUNNER: &str = r#"j=$1
trap : TERM INT HUP
echo $$ > "$j/pid.tmp" && mv "$j/pid.tmp" "$j/pid"
if [ -f "$j/input" ]; then exec 0<"$j/input"; rm -f "$j/input"; fi
if [ -z "$2" ] || cd "$2"; then sh -c "$3"; fi
code=$?
echo "$code" > "$j/exit.tmp" && mv "$j/exit.tmp" "$j/exit""#;

fn job_id(id: &str) -> Result<String> {
    Uuid::parse_str(id)
        .map(|u| u.simple().to_string())
        .map_err(|_| Error::new("invalid_job", "后台任务 ID 无效"))
}
fn script(id: &str, token: &str, body: &str) -> String {
    format!("id={id}\ntoken={token}\n{PRELUDE}{body}")
}
/// Reads the report header that `report` printed and the log excerpt after it into the result.
fn read_report(
    result: &mut OperationResult,
    stdout: Option<Vec<u8>>,
    redact: impl Fn(&mut String),
    id: &str,
    token: &str,
) -> Result<()> {
    // A failed connection or script leaves the result as it is; its status already says what went wrong.
    let (Some(bytes), true) = (stdout, result.successful()) else {
        return Ok(());
    };
    let marker = format!("\x1e{token}:job ");
    let at = bytes
        .windows(marker.len())
        .position(|w| w == marker.as_bytes())
        .ok_or_else(|| Error::new("job_report_missing", "未能读取后台任务的状态"))?;
    let rest = &bytes[at + marker.len()..];
    let end = rest.iter().position(|b| *b == 0x1f).unwrap_or(rest.len());
    let header = String::from_utf8_lossy(&rest[..end]).into_owned();
    if header == "missing" {
        return Err(Error::new(
            "job_not_found",
            "未找到此后台任务，可能已被清理",
        ));
    }
    let mut fields = header.split(' ');
    let mut field = || fields.next().unwrap_or_default();
    let (state, code, size, start) = (field(), field(), field(), field());
    let size: u64 = size.parse().unwrap_or(0);
    let mut start: u64 = start.parse().unwrap_or(0);
    let mut chunk = rest.get(end + 1..).unwrap_or_default();
    // The tail of a log can begin inside a multi-byte character, and a limited read can end inside one.
    let lead = chunk
        .iter()
        .take(3)
        .take_while(|b| **b & 0xC0 == 0x80)
        .count();
    chunk = &chunk[lead..];
    start += lead as u64;
    if start + (chunk.len() as u64) < size
        && let Err(e) = std::str::from_utf8(chunk)
        && e.error_len().is_none()
    {
        chunk = &chunk[..e.valid_up_to()];
    }
    let mut text = String::from_utf8_lossy(chunk).into_owned();
    redact(&mut text);
    result.stdout_bytes = chunk.len() as u64;
    result.stdout_truncated = false;
    result.stdout = text;
    result.stdout_tail.clear();
    result.job = Some(JobReport {
        id: id.into(),
        state: match state {
            "running" => JobState::Running,
            "exited" => JobState::Exited,
            "cancelled" => JobState::Cancelled,
            _ => JobState::Lost,
        },
        exit_code: code.parse().ok(),
        output_bytes: size,
        output_offset: start,
        next_offset: start + chunk.len() as u64,
    });
    Ok(())
}

impl OpenSsh<'_> {
    /// Starts `command` in the background and returns as soon as it runs. Finished jobs older than a week are
    /// removed when the next one starts on that server.
    pub fn job_start(
        &self,
        host: &str,
        connection: Option<&str>,
        command: &str,
        cwd: Option<&str>,
        stdin: Option<Vec<u8>>,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        error::require(
            !command.is_empty() && !command.contains('\0'),
            "invalid_command",
            "命令不能为空或包含空字符",
        )?;
        if let Some(p) = cwd {
            model::validate_absolute_path(p)?;
        }
        let id = Uuid::new_v4().simple().to_string();
        let token = Uuid::new_v4().simple().to_string();
        let body = format!(
            r#"umask 077
command -v setsid >/dev/null 2>&1 || {{ echo "此服务器没有 setsid，无法启动后台任务" >&2; exit 127; }}
mkdir -p "$d" && mkdir "$j" || exit 1
for old in "$d"/*; do
  [ "$old" != "$j" ] && [ -n "$(find "$old" -maxdepth 0 -mtime +7 2>/dev/null)" ] || continue
  [ -n "$(procs "${{old##*/}}")" ] && continue
  rm -rf -- "$old"
done
{input}: > "$j/output"
SSH_MANAGER_JOB=$id setsid sh -c {runner} sh "$j" {cwd} {command} </dev/null >>"$j/output" 2>&1 &
i=0; while [ ! -f "$j/pid" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i+1)); done
[ -f "$j/pid" ] || {{ echo "后台任务没有启动" >&2; exit 1; }}
report tail 0
"#,
            input = if stdin.is_some() {
                "cat > \"$j/input\" || exit 1\n"
            } else {
                ""
            },
            runner = shell_quote(RUNNER),
            cwd = shell_quote(cwd.unwrap_or("")),
            command = shell_quote(command),
        );
        self.exec_kind(
            host,
            connection,
            &script(&id, &token, &body),
            None,
            stdin.map(process::Input::Bytes),
            options,
            "job_start",
            |r, out, broker| read_report(r, out, |s| broker.redact(s), &id, &token),
        )
    }
    /// Reports the job's state and up to `output_limit` bytes of its log: from `offset`, or the latest bytes
    /// when it is omitted. `wait` holds the answer back until the job ends or that many seconds pass.
    pub fn job_status(
        &self,
        host: &str,
        connection: Option<&str>,
        id: &str,
        offset: Option<u64>,
        wait: u64,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        let id = job_id(id)?;
        // A wait that fills the timeout would turn every slow job into an unknown result.
        error::require(
            wait + 10 <= options.timeout.as_secs(),
            "invalid_arguments",
            "等待时间至少要比超时短 10 秒",
        )?;
        let token = Uuid::new_v4().simple().to_string();
        let limit = options.output_limit;
        let body = format!(
            r#"{MISSING}i=0
while [ $i -lt {wait} ] && [ ! -f "$j/exit" ] && [ -n "$(procs "$id")" ]; do sleep 1; i=$((i+1)); done
report {from} {limit}
"#,
            from = offset.map_or("tail".into(), |o| o.to_string()),
        );
        // The report header comes before the excerpt; leave room for it next to the requested limit.
        let options = ExecutionOptions {
            output_limit: limit + 512,
            ..options.clone()
        };
        self.exec_kind(
            host,
            connection,
            &script(&id, &token, &body),
            None,
            None,
            &options,
            "job_status",
            |r, out, broker| read_report(r, out, |s| broker.redact(s), &id, &token),
        )
    }
    /// Sends TERM to the job's processes and waits up to ten seconds for them to end; cancelling again sends
    /// KILL. Only the job's own processes are signalled, never others by name.
    pub fn job_cancel(
        &self,
        host: &str,
        connection: Option<&str>,
        id: &str,
        options: &ExecutionOptions,
    ) -> Result<OperationResult> {
        let id = job_id(id)?;
        let token = Uuid::new_v4().simple().to_string();
        let body = format!(
            r#"{MISSING}pids=$(procs "$id")
if [ -n "$pids" ]; then
  sig=TERM; [ -f "$j/cancelled" ] && sig=KILL
  [ -f "$j/exit" ] || : > "$j/cancelled"
  # sudo hides its environment, so its processes are only reached through the runner's group.
  pid=$(cat "$j/pid")
  case " $(echo $pids) " in *" $pid "*) kill -s "$sig" -- "-$pid" 2>/dev/null ;; esac
  kill -s "$sig" $pids 2>/dev/null
  i=0; while [ $i -lt 10 ] && [ -n "$(procs "$id")" ]; do sleep 1; i=$((i+1)); done
fi
report tail 0
"#
        );
        self.exec_kind(
            host,
            connection,
            &script(&id, &token, &body),
            None,
            None,
            options,
            "job_cancel",
            |r, out, broker| read_report(r, out, |s| broker.redact(s), &id, &token),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> OperationResult {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "operation_id": Uuid::nil(), "host_id": Uuid::nil(), "connection_id": Uuid::nil(),
            "kind": "job_status", "started_at": "2026-10-24T00:00:00Z", "duration_ms": 1, "prepare_ms": 0,
            "connect_ms": 1, "status": "succeeded", "phase": "completed", "exit_code": 0, "termination": null,
            "stdout": "", "stderr": "", "stdout_tail": "", "stderr_tail": "", "stdout_bytes": 0, "stderr_bytes": 0,
            "stdout_truncated": false, "stderr_truncated": false, "next_step": null, "transfer": null, "job": null
        }))
        .unwrap()
    }
    fn report(header: &str, log: &[u8]) -> Result<OperationResult> {
        let mut bytes = format!("\x1et:job {header}\x1f").into_bytes();
        bytes.extend_from_slice(log);
        let mut r = result();
        read_report(
            &mut r,
            Some(bytes),
            |s| *s = s.replace("secret", "[REDACTED]"),
            "id",
            "t",
        )?;
        Ok(r)
    }

    #[test]
    fn report_keeps_whole_characters_and_resumes_after_them() {
        // The excerpt starts one byte into "中" and stops one byte into the third character.
        let log = "中中中".as_bytes();
        let r = report("running - 9 1", &log[1..7]).unwrap();
        let job = r.job.unwrap();
        assert_eq!(r.stdout, "中");
        assert_eq!((job.output_offset, job.next_offset), (3, 6));
        assert_eq!(job.state, JobState::Running);
    }

    #[test]
    fn report_reads_exit_codes_and_redacts() {
        let r = report("cancelled 143 12 0", b"secret done\n").unwrap();
        let job = r.job.unwrap();
        assert_eq!(r.stdout, "[REDACTED] done\n");
        assert_eq!(
            (job.state, job.exit_code, job.next_offset),
            (JobState::Cancelled, Some(143), 12)
        );
    }

    #[test]
    fn missing_jobs_are_an_error() {
        assert_eq!(report("missing", b"").unwrap_err().code, "job_not_found");
    }
}
