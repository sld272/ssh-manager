use crate::{Error, Result};
use std::{
    collections::VecDeque,
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
    /// The first bytes of the stream; `tail` keeps the last ones after them, so a long stream still shows how it ended.
    pub bytes: Vec<u8>,
    pub tail: VecDeque<u8>,
    pub total: u64,
    /// Time from launching ssh until the remote shell printed its start record.
    pub started: Option<Duration>,
    pub remote_exit: Option<i32>,
    pub io_error: bool,
}
impl Capture {
    /// The whole stream, when the head and tail together still hold every byte of it.
    pub fn whole(&self) -> Option<Vec<u8>> {
        (self.total == (self.bytes.len() + self.tail.len()) as u64).then(|| {
            let mut v = self.bytes.clone();
            v.extend(&self.tail);
            v
        })
    }
}
pub struct ProcessResult {
    pub code: Option<i32>,
    pub stdout: Capture,
    pub stderr: Capture,
    pub termination: Option<String>,
}
/// What a process reads on stdin.
pub enum Input {
    /// Zeroized once written, since it can be a secret for the credential helper.
    Bytes(Vec<u8>),
    /// Streamed, so a large upload is never held in memory.
    File(std::fs::File),
}
fn capture(
    mut stream: impl Read,
    head: usize,
    tail: usize,
    token: Option<String>,
    spawned: Instant,
) -> Capture {
    let mut out = Capture {
        bytes: Vec::new(),
        tail: VecDeque::new(),
        total: 0,
        started: None,
        remote_exit: None,
        io_error: false,
    };
    let mut buf = [0u8; 8192];
    let mut window = Vec::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                out.total += n as u64;
                let kept = n.min(head.saturating_sub(out.bytes.len()));
                out.bytes.extend_from_slice(&buf[..kept]);
                out.tail.extend(&buf[kept..n]);
                if out.tail.len() > tail {
                    out.tail.drain(..out.tail.len() - tail);
                }
                if let Some(t) = &token {
                    window.extend_from_slice(&buf[..n]);
                    let s = String::from_utf8_lossy(&window);
                    if out.started.is_none() && s.contains(&format!("\x1e{t}:start\x1f")) {
                        out.started = Some(spawned.elapsed());
                    }
                    if let Some((_, after)) = s.split_once(&format!("\x1e{t}:exit:"))
                        && let Some((code, _)) = after.split_once('\x1f')
                    {
                        out.remote_exit = code.parse().ok();
                    }
                    if window.len() > 256 {
                        window.drain(..window.len() - 256);
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
/// Keeps the first `head` bytes of each stream, and the last `tail` bytes after those.
pub fn run(
    mut command: Command,
    input: Option<Input>,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
    head: usize,
    tail: usize,
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
    let start = Instant::now();
    let writer = input.map(|input| {
        let mut stdin = child.stdin.take().expect("piped stdin");
        thread::spawn(move || match input {
            Input::Bytes(mut data) => {
                let _ = stdin.write_all(&data);
                data.zeroize();
            }
            Input::File(mut file) => {
                let _ = std::io::copy(&mut file, &mut stdin);
            }
        })
    });
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || capture(stdout, head, tail, None, start));
    let err = thread::spawn(move || capture(stderr, head, tail, token, start));
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
