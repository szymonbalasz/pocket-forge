//! The process seam (§13): every Claude Code process goes through a [`Spawner`], so the
//! adapter's parser and lifecycle run against recorded stdout in tests ([`crate::replay`])
//! and no test or CI run ever starts the CLI.
//!
//! [`OsSpawner`] is the real one: `std::process` with reader threads (core commands are
//! synchronous), `CREATE_NO_WINDOW` for chat children, and on Windows one Job Object per
//! child so a kill or an app crash ends the whole process tree (§10, R75).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::invocation::{ExeKind, Executable};
use crate::stream::MAX_LINE;

/// The last bytes of stderr kept for an error's detail (FR-PRM-37).
pub const STDERR_TAIL: usize = 2048;

/// How long a one-shot command's stdout may still drain after the child ended, even at
/// the deadline ([`OsSpawner::run`]).
const READ_GRACE: Duration = Duration::from_secs(2);

/// Stdout lines buffered between the reader thread and the turn (backpressure: a child
/// that floods stdout waits instead of filling memory; review-sec 7).
const LINE_QUEUE: usize = 16;

/// Reads `r` line by line (without the line end) into `emit` until EOF, an error, or `emit`
/// returns `false`. A line keeps at most [`MAX_LINE`] + 1 bytes and the rest up to its
/// newline is skipped, so an endless line can't fill memory; the turn sees it longer than
/// [`MAX_LINE`] and ends with `stream_malformed` (review-sec 7).
pub fn read_lines(r: impl Read, mut emit: impl FnMut(String) -> bool) {
    let mut r = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        let (used, done) = match r.fill_buf() {
            Ok([]) | Err(_) => break,
            Ok(chunk) => {
                let end = chunk.iter().position(|&b| b == b'\n');
                let take = end.map_or(chunk.len(), |i| i + 1);
                let room = (MAX_LINE + 1).saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..take.min(room)]);
                (take, end.is_some())
            }
        };
        r.consume(used);
        if done {
            if !emit(line_of(&buf)) {
                return;
            }
            buf.clear();
        }
    }
    if !buf.is_empty() {
        emit(line_of(&buf));
    }
}

fn line_of(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf)
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

/// One process to start. `args` are the CLI's own arguments; an npm shim
/// ([`ExeKind::CmdShim`]) is started through std's batch-file handling (see `os_command`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub kind: ExeKind,
    pub args: Vec<String>,
    /// The whole environment (the spawner clears the inherited one).
    pub env: Vec<(OsString, OsString)>,
    pub cwd: PathBuf,
    /// `CREATE_NO_WINDOW` (chat children and detection; never the login console).
    pub hidden: bool,
}

impl CommandSpec {
    pub fn new(
        exe: &Executable,
        args: Vec<String>,
        env: Vec<(OsString, OsString)>,
        cwd: &Path,
        hidden: bool,
    ) -> Self {
        Self {
            program: exe.path.clone(),
            kind: exe.kind,
            args,
            env,
            cwd: cwd.to_owned(),
            hidden,
        }
    }

    /// The value of `name` in `env` (case-insensitive, as Windows compares).
    pub fn env_var(&self, name: &str) -> Option<&OsString> {
        self.env
            .iter()
            .find(|(k, _)| k.to_string_lossy().eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }
}

/// A finished one-shot command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: String,
    pub timed_out: bool,
}

/// What [`ChildProcess::next_line`] got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Line(String),
    Eof,
    Timeout,
}

/// A child that has exited (`code` is `None` when it was killed or ended by a signal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exited {
    pub code: Option<i32>,
}

/// Kills a child (and on Windows its whole job) from any thread: Stop (FR-PRM-27).
pub trait Kill: Send + Sync {
    fn kill(&self);
}

/// A long-lived stream-json child: stdin stays open across turns (FR-PRM-35).
pub trait ChildProcess: Send {
    /// Writes one line (the trailing newline is added).
    /// A write that hasn't finished within `timeout` (a child that stopped reading stdin,
    /// with more queued than the pipe holds) returns `ErrorKind::TimedOut` (FR-PRM-36).
    fn send_line(&mut self, line: &str, timeout: Duration) -> io::Result<()>;
    /// The next stdout line, waiting at most `timeout`.
    fn next_line(&mut self, timeout: Duration) -> Next;
    fn close_stdin(&mut self);
    /// Waits at most `timeout` for the exit; `None` while it still runs.
    fn wait(&mut self, timeout: Duration) -> Option<Exited>;
    fn killer(&self) -> Arc<dyn Kill>;
    /// The last [`STDERR_TAIL`] bytes of stderr, raw (scrub before showing).
    fn stderr_tail(&self) -> String;
}

/// Starts processes. Tests use [`crate::replay::ReplaySpawner`]; slice 5's consent test
/// uses one that panics.
pub trait Spawner: Send + Sync {
    /// The file-system check behind executable resolution (§10).
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    /// Runs a one-shot command (stdin closed, stderr dropped unread), killed after `timeout`.
    fn run(&self, spec: &CommandSpec, timeout: Duration) -> io::Result<Captured>;
    fn spawn(&self, spec: &CommandSpec) -> io::Result<Box<dyn ChildProcess>>;
    /// Opens a visible console running `spec` and returns at once (the login, FR-PRM-45).
    fn open_console(&self, spec: &CommandSpec) -> io::Result<()>;
}

// ---------------------------------------------------------------- the real spawner

/// The real spawner.
#[derive(Debug, Default)]
pub struct OsSpawner;

/// Windows `CREATE_NEW_CONSOLE`: the login runs in its own visible console.
pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

fn os_command(spec: &CommandSpec, creation_flags: u32) -> Command {
    // An npm shim (`.cmd`) goes through std's batch-file handling: it runs the system
    // `cmd.exe /e:ON /v:OFF /d /c` and escapes `%`, `!`, `^`, `&` and quotes so `cmd`
    // can't expand or split an argument (CVE-2024-24576). A hand-built `cmd /s /c` line
    // let `cmd` expand `%VAR%` inside a data-dir path. Only fixed arguments ever go
    // here; std refuses arguments with line breaks (§10).
    let mut c = Command::new(&spec.program);
    c.args(&spec.args);
    c.env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&spec.cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if creation_flags != 0 {
            c.creation_flags(creation_flags);
        }
    }
    #[cfg(not(windows))]
    let _ = creation_flags;
    c
}

fn hidden_flags(spec: &CommandSpec) -> u32 {
    if spec.hidden {
        crate::invocation::CREATE_NO_WINDOW
    } else {
        0
    }
}

fn wait_child(child: &Mutex<Child>, timeout: Duration) -> Option<Exited> {
    let start = Instant::now();
    loop {
        let polled = child.lock().ok().map(|mut c| c.try_wait());
        match polled {
            Some(Ok(Some(status))) => {
                return Some(Exited {
                    code: status.code(),
                });
            }
            Some(Ok(None)) if start.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(20));
            }
            Some(Err(_)) | None => return Some(Exited { code: None }),
            Some(Ok(None)) => return None,
        }
    }
}

impl Spawner for OsSpawner {
    /// Everything is bounded by `timeout` (plus [`READ_GRACE`] to drain stdout): the Job
    /// Object is required (R75), and once the child exits or times out the whole tree is
    /// ended, so a grandchild still holding stdout (the npm shim's `node`, a daemon) can't
    /// hang detection (review-code 6, FR-PRM-33). Stdout keeps at most [`MAX_LINE`] bytes.
    fn run(&self, spec: &CommandSpec, timeout: Duration) -> io::Result<Captured> {
        let deadline = Instant::now() + timeout;
        let mut cmd = os_command(spec, hidden_flags(spec));
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        #[cfg(windows)]
        let job = match crate::job::JobObject::new().and_then(|j| j.assign(&child).map(|()| j)) {
            Ok(j) => j,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let out = child.stdout.take();
        let err = child.stderr.take();
        let (out_tx, out_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(o) = out {
                let _ = o.take(MAX_LINE as u64).read_to_end(&mut buf);
            }
            let _ = out_tx.send(buf);
        });
        // Stderr may hold account details: read and dropped (FR-PRM-33). Never joined.
        thread::spawn(move || {
            if let Some(mut e) = err {
                let _ = io::copy(&mut e, &mut io::sink());
            }
        });
        let child = Mutex::new(child);
        let status = wait_child(&child, timeout);
        let mut timed_out = status.is_none();
        #[cfg(windows)]
        job.terminate();
        if timed_out && let Ok(mut c) = child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .max(READ_GRACE);
        let stdout = out_rx.recv_timeout(wait).unwrap_or_else(|_| {
            timed_out = true;
            Vec::new()
        });
        Ok(Captured {
            code: status.and_then(|s| s.code),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            timed_out,
        })
    }

    fn spawn(&self, spec: &CommandSpec) -> io::Result<Box<dyn ChildProcess>> {
        Ok(Box::new(OsChild::spawn(spec)?))
    }

    fn open_console(&self, spec: &CommandSpec) -> io::Result<()> {
        if !cfg!(windows) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a login console can only be opened on Windows; run the shown command",
            ));
        }
        // Detached: dropping a std `Child` neither waits for nor kills it.
        os_command(spec, CREATE_NEW_CONSOLE).spawn().map(drop)
    }
}

struct OsKill {
    child: Arc<Mutex<Child>>,
    #[cfg(windows)]
    job: Option<Arc<crate::job::JobObject>>,
}

impl Kill for OsKill {
    fn kill(&self) {
        #[cfg(windows)]
        if let Some(j) = &self.job {
            j.terminate();
        }
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }
}

/// The child's stdin, written by its own thread so a write that blocks (the child stopped
/// reading, the pipe is full) can be given up on after a timeout (review-code 5, FR-PRM-36).
/// The blocked thread ends when the child is killed and the pipe breaks. Dropping the
/// writer closes stdin once no write is in flight.
struct StdinWriter {
    tx: mpsc::Sender<Vec<u8>>,
    ack: Receiver<io::Result<()>>,
    /// A write timed out and may still be blocked.
    stuck: bool,
}

impl StdinWriter {
    fn new(mut stdin: ChildStdin) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let (ack_tx, ack) = mpsc::channel();
        thread::spawn(move || {
            for buf in rx {
                let r = stdin.write_all(&buf).and_then(|()| stdin.flush());
                let failed = r.is_err();
                if ack_tx.send(r).is_err() || failed {
                    break;
                }
            }
        });
        Self {
            tx,
            ack,
            stuck: false,
        }
    }

    fn send(&mut self, line: &str, timeout: Duration) -> io::Result<()> {
        if self.stuck {
            match self.ack.try_recv() {
                Ok(_) => self.stuck = false,
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "an earlier stdin write is still blocked",
                    ));
                }
            }
        }
        let mut buf = Vec::with_capacity(line.len() + 1);
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
        self.tx
            .send(buf)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "stdin closed"))?;
        match self.ack.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => {
                self.stuck = true;
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the child didn't read its stdin in time",
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "stdin closed"))
            }
        }
    }
}

/// A real stream-json child.
pub struct OsChild {
    child: Arc<Mutex<Child>>,
    stdin: Option<StdinWriter>,
    rx: Receiver<Option<String>>,
    eof: bool,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    killer: Arc<OsKill>,
}

impl OsChild {
    fn spawn(spec: &CommandSpec) -> io::Result<Self> {
        let mut cmd = os_command(spec, hidden_flags(spec));
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        #[cfg(windows)]
        let job = match crate::job::JobObject::new().and_then(|j| j.assign(&child).map(|()| j)) {
            Ok(j) => Some(Arc::new(j)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let stdin = child.stdin.take().map(StdinWriter::new);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let (tx, rx) = mpsc::sync_channel(LINE_QUEUE);
        thread::spawn(move || {
            read_lines(stdout, |line| tx.send(Some(line)).is_ok());
            let _ = tx.send(None);
        });
        let stderr = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL)));
        if let Some(mut e) = child.stderr.take() {
            let sink = Arc::clone(&stderr);
            thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = e.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut s) = sink.lock() {
                        s.extend(&chunk[..n]);
                        let over = s.len().saturating_sub(STDERR_TAIL);
                        s.drain(..over);
                    }
                }
            });
        }
        let child = Arc::new(Mutex::new(child));
        let killer = Arc::new(OsKill {
            child: Arc::clone(&child),
            #[cfg(windows)]
            job,
        });
        Ok(Self {
            child,
            stdin,
            rx,
            eof: false,
            stderr,
            killer,
        })
    }
}

impl ChildProcess for OsChild {
    fn send_line(&mut self, line: &str, timeout: Duration) -> io::Result<()> {
        let s = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdin closed"))?;
        s.send(line, timeout)
    }

    fn next_line(&mut self, timeout: Duration) -> Next {
        if self.eof {
            return Next::Eof;
        }
        match self.rx.recv_timeout(timeout) {
            Ok(Some(l)) => Next::Line(l),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                self.eof = true;
                Next::Eof
            }
            Err(RecvTimeoutError::Timeout) => Next::Timeout,
        }
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    fn wait(&mut self, timeout: Duration) -> Option<Exited> {
        wait_child(&self.child, timeout)
    }

    fn killer(&self) -> Arc<dyn Kill> {
        self.killer.clone()
    }

    fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|s| String::from_utf8_lossy(&s.iter().copied().collect::<Vec<_>>()).into_owned())
            .unwrap_or_default()
    }
}

impl Drop for OsChild {
    fn drop(&mut self) {
        if self.wait(Duration::ZERO).is_none() {
            self.killer.kill();
            let _ = self.wait(Duration::from_secs(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// review-sec 7 (R96): an endless stdout line keeps at most MAX_LINE + 1 bytes, the
    /// rest is skipped up to its newline, and the next line still arrives.
    #[test]
    fn prm_s10_read_lines_bounds_a_long_line() {
        let mut input = b"first\r\n".to_vec();
        input.extend(std::iter::repeat_n(b'x', 3 * MAX_LINE));
        input.extend_from_slice(b"\nnext\nlast without newline");
        let mut lines = Vec::new();
        read_lines(io::Cursor::new(input), |l| {
            lines.push(l);
            true
        });
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "first");
        assert_eq!(lines[1].len(), MAX_LINE + 1);
        assert_eq!(lines[2], "next");
        assert_eq!(lines[3], "last without newline");

        // A receiver that hangs up stops the reader.
        let mut n = 0;
        read_lines(io::Cursor::new(b"a\nb\nc\n".to_vec()), |_| {
            n += 1;
            false
        });
        assert_eq!(n, 1);
    }

    /// review-code 5: a child that never reads stdin; a write bigger than the pipe gives up
    /// after its timeout instead of blocking, and the kill still ends the child.
    #[cfg(windows)]
    #[test]
    fn prm_s10_os_child_stdin_write_is_bounded() {
        let exe = Executable {
            path: PathBuf::from("cmd.exe"),
            kind: ExeKind::Native,
            source: crate::invocation::ExeSource::Override,
        };
        let env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        let spec = CommandSpec::new(
            &exe,
            vec![
                "/d".into(),
                "/c".into(),
                "echo ready & ping -n 30 127.0.0.1 >nul".into(),
            ],
            env,
            &std::env::temp_dir(),
            true,
        );
        let mut child = OsSpawner
            .spawn(&spec)
            .unwrap_or_else(|e| panic!("spawn: {e}"));
        assert!(matches!(
            child.next_line(Duration::from_secs(10)),
            Next::Line(_)
        ));
        let big = "x".repeat(8 << 20);
        let start = Instant::now();
        let e = child
            .send_line(&big, Duration::from_millis(500))
            .expect_err("8 MiB can't fit in the pipe");
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(5));
        // A second write while the first is stuck fails at once.
        let e = child
            .send_line("{}", Duration::from_secs(5))
            .expect_err("still stuck");
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(5));
        child.killer().kill();
        assert!(child.wait(Duration::from_secs(5)).is_some());
    }

    fn cmd_spec(script: &str) -> CommandSpec {
        let exe = Executable {
            path: PathBuf::from("cmd.exe"),
            kind: ExeKind::Native,
            source: crate::invocation::ExeSource::Override,
        };
        let env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        CommandSpec::new(
            &exe,
            vec!["/d".into(), "/c".into(), script.into()],
            env,
            &std::env::temp_dir(),
            true,
        )
    }

    /// review-code 6 (FR-PRM-33, R75): a one-shot command whose grandchild keeps stdout open
    /// (like the npm shim's `node`) returns once the child exits; the tree is ended.
    #[cfg(windows)]
    #[test]
    fn prm_s10_run_is_not_held_by_a_grandchild() {
        let start = Instant::now();
        let out = OsSpawner
            .run(
                &cmd_spec("echo 2.1.319 & start /b ping -n 60 127.0.0.1"),
                Duration::from_secs(30),
            )
            .unwrap_or_else(|e| panic!("run: {e}"));
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
        assert!(!out.timed_out);
        assert_eq!(out.code, Some(0));
        assert!(out.stdout.contains("2.1.319"), "{}", out.stdout);
    }

    /// review-code 6: a hanging one-shot command is killed, tree included, at its timeout.
    #[cfg(windows)]
    #[test]
    fn prm_s10_run_times_out_and_ends_the_tree() {
        let start = Instant::now();
        let out = OsSpawner
            .run(
                &cmd_spec("echo partial & ping -n 60 127.0.0.1 >nul"),
                Duration::from_secs(1),
            )
            .unwrap_or_else(|e| panic!("run: {e}"));
        assert!(out.timed_out);
        assert!(
            start.elapsed() < Duration::from_secs(6),
            "{:?}",
            start.elapsed()
        );
    }

    /// The real spawner and the job object, on a harmless system command (never the CLI).
    #[cfg(windows)]
    #[test]
    fn prm_s10_os_child_kill_ends_the_process_tree() {
        let exe = Executable {
            path: PathBuf::from("cmd.exe"),
            kind: ExeKind::Native,
            source: crate::invocation::ExeSource::Override,
        };
        let env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        let cwd = std::env::temp_dir();
        // `cmd` starts `ping` as a grandchild that holds the stdout pipe.
        let spec = CommandSpec::new(
            &exe,
            vec![
                "/d".into(),
                "/c".into(),
                "echo ready & ping -n 30 127.0.0.1 >nul".into(),
            ],
            env,
            &cwd,
            true,
        );
        let mut child = OsSpawner
            .spawn(&spec)
            .unwrap_or_else(|e| panic!("spawn: {e}"));
        let first = child.next_line(Duration::from_secs(10));
        assert!(
            matches!(&first, Next::Line(l) if l.trim() == "ready"),
            "{first:?}"
        );
        let start = Instant::now();
        child.killer().kill();
        let mut eof = false;
        while start.elapsed() < Duration::from_secs(5) {
            if child.next_line(Duration::from_secs(1)) == Next::Eof {
                eof = true;
                break;
            }
        }
        assert!(eof, "stdout stayed open: a grandchild escaped the job");
        assert!(child.wait(Duration::from_secs(5)).is_some());
    }
}
