use crate::{Error, Result};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

#[derive(Debug)]
pub struct Capture {
    pub bytes: Vec<u8>,
    pub total: u64,
    pub started: bool,
    pub remote_exit: Option<i32>,
    pub io_error: bool,
}
pub struct ProcessResult {
    pub code: Option<i32>,
    pub stdout: Capture,
    pub stderr: Capture,
    pub termination: Option<String>,
}
fn capture(mut stream: impl Read, limit: usize, token: Option<String>) -> Capture {
    let mut out = Capture {
        bytes: Vec::new(),
        total: 0,
        started: false,
        remote_exit: None,
        io_error: false,
    };
    let mut buf = [0u8; 8192];
    let mut tail = Vec::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                out.total += n as u64;
                out.bytes
                    .extend_from_slice(&buf[..n.min(limit.saturating_sub(out.bytes.len()))]);
                if let Some(t) = &token {
                    tail.extend_from_slice(&buf[..n]);
                    let s = String::from_utf8_lossy(&tail);
                    if s.contains(&format!("\x1e{t}:start\x1f")) {
                        out.started = true;
                    }
                    if let Some((_, after)) = s.split_once(&format!("\x1e{t}:exit:")) {
                        if let Some((code, _)) = after.split_once('\x1f') {
                            out.remote_exit = code.parse().ok();
                        }
                    }
                    if tail.len() > 256 {
                        tail.drain(..tail.len() - 256);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                out.io_error = true;
                break;
            }
        }
    }
    out
}
pub fn run(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
    limit: usize,
    token: Option<String>,
) -> Result<ProcessResult> {
    command
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| Error::new("process_start_failed", format!("无法启动 SSH 工具：{e}")))?;
    let pid = child.id();
    let writer = input.map(|mut data| {
        let mut stdin = child.stdin.take().expect("piped stdin");
        thread::spawn(move || {
            let _ = stdin.write_all(&data);
            data.zeroize();
        })
    });
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || capture(stdout, limit, None));
    let err = thread::spawn(move || capture(stderr, limit, token));
    let start = Instant::now();
    let mut termination = None;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() >= timeout {
            termination = Some(
                if cancel.load(Ordering::Relaxed) {
                    "cancelled"
                } else {
                    "timeout"
                }
                .into(),
            );
            // A running ProxyJump creates nested ssh processes; killing only the direct child leaves pipes open.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(15));
    };
    // ssh can exit while a local helper still holds its pipes; reap the whole per-operation process group.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    Ok(ProcessResult {
        code: status.code(),
        stdout: out
            .join()
            .map_err(|_| Error::new("output_read_failed", "读取标准输出失败"))?,
        stderr: err
            .join()
            .map_err(|_| Error::new("output_read_failed", "读取错误输出失败"))?,
        termination,
    })
}
