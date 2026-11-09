//! Child processes for the probe: one-shot captures and long-lived stream-json sessions.
//! Plain `std::process` with reader threads; slice 4 builds the real adapter on tokio.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pf_chat::invocation::Executable;

/// A command for `exe` with `args`, a cleared environment set to `env`, and `cwd`.
/// `hidden` adds `CREATE_NO_WINDOW` on Windows (chat children; not the login).
pub fn command(
    exe: &Executable,
    args: &[String],
    env: &[(OsString, OsString)],
    cwd: &Path,
    hidden: bool,
) -> Command {
    let (program, prefix) = exe.program_and_prefix();
    let mut c = Command::new(program);
    c.args(prefix)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd);
    #[cfg(windows)]
    if hidden {
        use std::os::windows::process::CommandExt;
        c.creation_flags(pf_chat::invocation::CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = hidden;
    c
}

/// The result of a one-shot command.
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: String,
    pub timed_out: bool,
}

fn wait_timeout(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(25)),
            _ => return None,
        }
    }
}

/// Runs `cmd` to completion (killed after `timeout`). Stderr is read and dropped: it may
/// hold account details, and the probe never prints it.
pub fn capture(mut cmd: Command, timeout: Duration) -> io::Result<Captured> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let out_t = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut buf);
        }
        buf
    });
    let err_t = thread::spawn(move || {
        let mut sink = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_end(&mut sink);
        }
    });
    let status = wait_timeout(&mut child, timeout);
    let timed_out = status.is_none();
    if timed_out {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = out_t.join().unwrap_or_default();
    let _ = err_t.join();
    Ok(Captured {
        code: status.and_then(|s| s.code()),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        timed_out,
    })
}

/// What [`Session::next`] got.
pub enum Next {
    Line(String),
    Eof,
    Timeout,
}

/// What a kill did (FR-PRM-27 cancel; §10 job object in slice 4).
#[derive(Debug, Clone, Copy)]
pub struct KillReport {
    /// When stdout reached EOF after the kill. `None`: still open after 5 s, so something
    /// else (a grandchild) holds the pipe.
    pub eof_after: Option<Duration>,
    pub exited: bool,
}

/// A long-lived stream-json child: stdin stays open across turns (FR-PRM-35).
pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Option<String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Session {
    pub fn spawn(mut cmd: Command) -> io::Result<Self> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf)
                            .trim_end_matches(['\r', '\n'])
                            .to_owned();
                        if tx.send(Some(line)).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = tx.send(None);
        });
        let stderr = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut e) = child.stderr.take() {
            let sink = Arc::clone(&stderr);
            thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = e.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut s) = sink.lock() {
                        s.extend_from_slice(&chunk[..n]);
                    }
                }
            });
        }
        Ok(Self {
            child,
            stdin,
            rx,
            stderr,
        })
    }

    /// Writes one JSON line to stdin.
    pub fn send(&mut self, line: &str) -> io::Result<()> {
        let s = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("stdin closed"))?;
        s.write_all(line.as_bytes())?;
        s.write_all(b"\n")?;
        s.flush()
    }

    pub fn next(&self, timeout: Duration) -> Next {
        match self.rx.recv_timeout(timeout) {
            Ok(Some(l)) => Next::Line(l),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => Next::Eof,
            Err(RecvTimeoutError::Timeout) => Next::Timeout,
        }
    }

    /// Kills the child (as Stop does) and watches stdout close.
    pub fn kill(&mut self) -> KillReport {
        let start = Instant::now();
        let _ = self.child.kill();
        let limit = Duration::from_secs(5);
        let mut eof_after = None;
        while start.elapsed() < limit {
            match self.next(limit.saturating_sub(start.elapsed())) {
                Next::Line(_) => {}
                Next::Eof => {
                    eof_after = Some(start.elapsed());
                    break;
                }
                Next::Timeout => break,
            }
        }
        let exited = wait_timeout(&mut self.child, Duration::from_secs(5)).is_some();
        KillReport { eof_after, exited }
    }

    /// Closes stdin (as the window closing does) and waits for the exit. Lines that still
    /// arrive are returned.
    pub fn close(&mut self, timeout: Duration) -> (Option<ExitStatus>, Vec<String>) {
        self.stdin = None;
        let start = Instant::now();
        let mut trailing = Vec::new();
        while start.elapsed() < timeout {
            match self.next(timeout.saturating_sub(start.elapsed())) {
                Next::Line(l) => trailing.push(l),
                Next::Eof | Next::Timeout => break,
            }
        }
        let status = wait_timeout(&mut self.child, timeout.saturating_sub(start.elapsed()));
        if status.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        (status, trailing)
    }

    /// Everything the child wrote to stderr so far (scrub before showing it).
    pub fn stderr_text(&self) -> String {
        self.stderr
            .lock()
            .map(|s| String::from_utf8_lossy(&s).into_owned())
            .unwrap_or_default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
