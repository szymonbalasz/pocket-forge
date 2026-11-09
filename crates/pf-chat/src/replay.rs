//! Scripted processes (§13): a [`ScriptedChild`] plays stdout lines, delays, stalls and exits
//! after each stdin line, on a [`Clock`]. [`ReplaySpawner`] serves them to the Claude Code
//! adapter, so its parser and lifecycle are tested against the committed recordings without
//! the CLI. The fake backend uses the same child in memory (it starts no process).

use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::clock::Clock;
use crate::process::{Captured, ChildProcess, CommandSpec, Exited, Kill, Next, Spawner};

/// One scripted step after a stdin line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Line(String),
    /// Nothing for this long (on the clock).
    Delay(Duration),
    /// Nothing, ever (until killed).
    Stall,
    /// As a turn's first step: the child never reads that stdin line, so the write blocks
    /// (review-code 5).
    StdinStall,
    /// The process exits with this code; stdout closes.
    Exit(Option<i32>),
}

/// Steps per stdin line: `turns[i]` plays after the `i`-th line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    pub turns: Vec<Vec<Step>>,
    pub stderr: String,
}

impl Script {
    /// One turn per text, each text's non-empty lines in order.
    pub fn recordings(texts: &[&str]) -> Self {
        Self {
            turns: texts.iter().map(|t| lines(t)).collect(),
            stderr: String::new(),
        }
    }

    pub fn with_stderr(mut self, text: &str) -> Self {
        self.stderr = text.to_owned();
        self
    }
}

/// The non-empty lines of a recording as steps.
pub fn lines(text: &str) -> Vec<Step> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Step::Line(l.to_owned()))
        .collect()
}

#[derive(Debug, Default)]
struct Killed(AtomicBool);

impl Kill for Killed {
    fn kill(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// The step size for waits, so a kill is noticed promptly on the real clock.
const NAP: Duration = Duration::from_millis(50);

/// A scripted stream-json child.
pub struct ScriptedChild {
    turns: VecDeque<Vec<Step>>,
    pending: VecDeque<Step>,
    clock: Arc<dyn Clock>,
    killed: Arc<Killed>,
    stdin_open: bool,
    exit: Option<Exited>,
    stderr: String,
    stdin_log: Arc<Mutex<Vec<String>>>,
}

impl ScriptedChild {
    pub fn new(script: Script, clock: Arc<dyn Clock>) -> Self {
        Self {
            turns: script.turns.into(),
            pending: VecDeque::new(),
            clock,
            killed: Arc::default(),
            stdin_open: true,
            exit: None,
            stderr: script.stderr,
            stdin_log: Arc::default(),
        }
    }

    /// Every stdin line written so far (shared with the spawner's log).
    pub fn stdin_log(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.stdin_log)
    }

    pub fn was_killed(&self) -> bool {
        self.killed.0.load(Ordering::SeqCst)
    }

    fn is_killed(&self) -> bool {
        self.was_killed()
    }

    /// Sleeps `d` on the clock in small steps; false if killed meanwhile.
    fn nap(&self, mut d: Duration) -> bool {
        while !d.is_zero() {
            if self.is_killed() {
                return false;
            }
            let s = d.min(NAP);
            self.clock.sleep(s);
            d -= s;
        }
        !self.is_killed()
    }
}

impl ChildProcess for ScriptedChild {
    fn send_line(&mut self, line: &str, timeout: Duration) -> io::Result<()> {
        if self.is_killed() || self.exit.is_some() || !self.stdin_open {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "child gone"));
        }
        if self.turns.front().and_then(|t| t.first()) == Some(&Step::StdinStall) {
            // The write never completes: wait out the timeout on the clock.
            return if self.nap(timeout) {
                Err(io::Error::new(io::ErrorKind::TimedOut, "stdin not read"))
            } else {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "child killed"))
            };
        }
        if let Ok(mut log) = self.stdin_log.lock() {
            log.push(line.to_owned());
        }
        let steps = self.turns.pop_front().unwrap_or_default();
        self.pending.extend(steps);
        Ok(())
    }

    fn next_line(&mut self, timeout: Duration) -> Next {
        let mut left = timeout;
        loop {
            if self.is_killed() {
                self.exit.get_or_insert(Exited { code: None });
                return Next::Eof;
            }
            if self.exit.is_some() {
                return Next::Eof;
            }
            match self.pending.front_mut() {
                None if !self.stdin_open => {
                    self.exit = Some(Exited { code: Some(0) });
                    return Next::Eof;
                }
                None | Some(Step::Stall | Step::StdinStall) => {
                    self.nap(left);
                    return if self.is_killed() {
                        Next::Eof
                    } else {
                        Next::Timeout
                    };
                }
                Some(Step::Line(_)) => {
                    if let Some(Step::Line(l)) = self.pending.pop_front() {
                        return Next::Line(l);
                    }
                }
                Some(Step::Delay(d)) => {
                    if *d <= left {
                        let d = *d;
                        self.pending.pop_front();
                        self.nap(d);
                        left -= d;
                    } else {
                        *d -= left;
                        self.nap(left);
                        return if self.is_killed() {
                            Next::Eof
                        } else {
                            Next::Timeout
                        };
                    }
                }
                Some(Step::Exit(code)) => {
                    self.exit = Some(Exited { code: *code });
                    self.pending.clear();
                    self.turns.clear();
                    return Next::Eof;
                }
            }
        }
    }

    fn close_stdin(&mut self) {
        self.stdin_open = false;
    }

    fn wait(&mut self, timeout: Duration) -> Option<Exited> {
        if let Some(e) = self.exit {
            return Some(e);
        }
        if self.is_killed() {
            return Some(Exited { code: None });
        }
        if !self.stdin_open && self.pending.is_empty() {
            self.exit = Some(Exited { code: Some(0) });
            return self.exit;
        }
        self.nap(timeout);
        self.is_killed().then_some(Exited { code: None })
    }

    fn killer(&self) -> Arc<dyn Kill> {
        self.killed.clone()
    }

    fn stderr_tail(&self) -> String {
        self.stderr.clone()
    }
}

/// What the replay spawner saw.
#[derive(Debug, Default, Clone)]
pub struct SpawnLog {
    /// One-shot commands (`--version`, `auth status`, `auth logout`).
    pub runs: Vec<CommandSpec>,
    /// Long-lived children, with the stdin lines each one received.
    pub spawns: Vec<(CommandSpec, Arc<Mutex<Vec<String>>>)>,
    pub consoles: Vec<CommandSpec>,
}

impl SpawnLog {
    /// The stdin lines of the `i`-th spawned child.
    pub fn stdin(&self, i: usize) -> Vec<String> {
        self.spawns
            .get(i)
            .and_then(|(_, l)| l.lock().ok().map(|l| l.clone()))
            .unwrap_or_default()
    }
}

/// Serves scripted one-shot results and children. Files it reports as present are the only
/// executables that resolve.
pub struct ReplaySpawner {
    clock: Arc<dyn Clock>,
    files: HashSet<String>,
    runs: Mutex<Vec<(Vec<String>, Captured)>>,
    scripts: Mutex<VecDeque<Script>>,
    log: Mutex<SpawnLog>,
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

impl ReplaySpawner {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            files: HashSet::new(),
            runs: Mutex::default(),
            scripts: Mutex::default(),
            log: Mutex::default(),
        }
    }

    /// Makes `path` exist for executable resolution.
    pub fn with_file(mut self, path: impl AsRef<Path>) -> Self {
        self.files.insert(norm(path.as_ref()));
        self
    }

    /// A one-shot result for any command whose arguments end with `args_suffix`. Later
    /// registrations win.
    pub fn on_run(self, args_suffix: &[&str], code: Option<i32>, stdout: &str) -> Self {
        if let Ok(mut r) = self.runs.lock() {
            r.push((
                args_suffix.iter().map(|s| (*s).to_owned()).collect(),
                Captured {
                    code,
                    stdout: stdout.to_owned(),
                    timed_out: code.is_none(),
                },
            ));
        }
        self
    }

    /// The script for the next spawned child.
    pub fn push_script(&self, script: Script) {
        if let Ok(mut s) = self.scripts.lock() {
            s.push_back(script);
        }
    }

    pub fn log(&self) -> SpawnLog {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

impl Spawner for ReplaySpawner {
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains(&norm(path))
    }

    fn run(&self, spec: &CommandSpec, _timeout: Duration) -> io::Result<Captured> {
        if let Ok(mut l) = self.log.lock() {
            l.runs.push(spec.clone());
        }
        let runs = self.runs.lock().map_err(|_| io::Error::other("poisoned"))?;
        runs.iter()
            .rev()
            .find(|(suffix, _)| spec.args.ends_with(suffix))
            .map(|(_, c)| c.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no scripted result"))
    }

    fn spawn(&self, spec: &CommandSpec) -> io::Result<Box<dyn ChildProcess>> {
        let script = self
            .scripts
            .lock()
            .ok()
            .and_then(|mut s| s.pop_front())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no scripted child"))?;
        let child = ScriptedChild::new(script, Arc::clone(&self.clock));
        if let Ok(mut l) = self.log.lock() {
            l.spawns.push((spec.clone(), child.stdin_log()));
        }
        Ok(Box::new(child))
    }

    fn open_console(&self, spec: &CommandSpec) -> io::Result<()> {
        if let Ok(mut l) = self.log.lock() {
            l.consoles.push(spec.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;

    #[test]
    fn scripted_child_plays_steps_on_the_clock() {
        let clock = FakeClock::new();
        let script = Script {
            turns: vec![vec![
                Step::Line("a".into()),
                Step::Delay(Duration::from_secs(5)),
                Step::Line("b".into()),
                Step::Exit(Some(2)),
            ]],
            stderr: String::new(),
        };
        let mut c = ScriptedChild::new(script, Arc::new(clock.clone()));
        assert_eq!(c.next_line(Duration::from_secs(1)), Next::Timeout);
        assert!(c.send_line("{}", Duration::from_secs(1)).is_ok());
        assert_eq!(c.next_line(Duration::from_secs(1)), Next::Line("a".into()));
        assert_eq!(c.next_line(Duration::from_secs(3)), Next::Timeout);
        assert_eq!(c.next_line(Duration::from_secs(3)), Next::Line("b".into()));
        assert_eq!(clock.now(), Duration::from_secs(6));
        assert_eq!(c.next_line(Duration::from_secs(1)), Next::Eof);
        assert_eq!(c.wait(Duration::ZERO), Some(Exited { code: Some(2) }));
    }
}
