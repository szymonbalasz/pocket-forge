//! How the Claude Code child is started (§10, FR-PRM-34, FR-PRM-45).
//!
//! No user text is ever put in an argument: messages go over stdin as stream-json. The
//! only text argument is the fixed system prompt (Appendix B).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The isolated config folder variable (FR-PRM-45).
pub const ENV_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";

/// The folder name of the isolated config inside `<data-dir>` (FR-PRM-45).
pub const CONFIG_DIR_NAME: &str = "claude-config";

/// Extra variables set on every chat child and detection command (§10).
pub const EXTRA_ENV: [(&str, &str); 2] = [
    ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
    ("DISABLE_AUTOUPDATER", "1"),
];

/// Windows `CREATE_NO_WINDOW`: chat children run without a console window (§10).
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `<data-dir>/claude-config/` (FR-PRM-45).
pub fn config_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(CONFIG_DIR_NAME)
}

/// The chat child's arguments, exactly §10's list (snapshot-tested).
pub fn chat_args(system_prompt: &str) -> Vec<String> {
    [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--system-prompt",
        system_prompt,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// `--version`, for detection (FR-PRM-33).
pub fn version_args() -> Vec<String> {
    vec!["--version".into()]
}

/// `auth status --json`, for detection (FR-PRM-33).
pub fn auth_status_args() -> Vec<String> {
    ["auth", "status", "--json"].map(String::from).to_vec()
}

/// `auth login` / `auth logout` (FR-PRM-45), run with the same environment.
pub fn auth_login_args() -> Vec<String> {
    ["auth", "login"].map(String::from).to_vec()
}

/// See [`auth_login_args`].
pub fn auth_logout_args() -> Vec<String> {
    ["auth", "logout"].map(String::from).to_vec()
}

/// The child's environment: `parent` minus every `PF_*` variable (and any existing
/// `CLAUDE_CONFIG_DIR`), plus `CLAUDE_CONFIG_DIR=<config_dir>` and [`EXTRA_ENV`]
/// (FR-PRM-34, FR-PRM-45). Names compare case-insensitively, as Windows does. Use it with
/// `Command::env_clear()`.
pub fn child_env<I>(parent: I, config_dir: &Path) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let ours: Vec<&str> = std::iter::once(ENV_CONFIG_DIR)
        .chain(EXTRA_ENV.iter().map(|(k, _)| *k))
        .collect();
    let mut env: Vec<(OsString, OsString)> = parent
        .into_iter()
        .filter(|(k, _)| {
            let up = k.to_string_lossy().to_uppercase();
            !up.starts_with("PF_") && !ours.contains(&up.as_str())
        })
        .collect();
    env.push((ENV_CONFIG_DIR.into(), config_dir.as_os_str().to_owned()));
    for (k, v) in EXTRA_ENV {
        env.push((k.into(), v.into()));
    }
    env
}

/// How the executable is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExeKind {
    /// A native binary (`claude.exe`, or `claude` off Windows).
    Native,
    /// The npm shim `claude.cmd`, run through `cmd /d /c` with fixed arguments only.
    CmdShim,
}

/// Where the executable was found (§10's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExeSource {
    /// `prompt_chat.claude_path` (or the probe's `--claude`).
    Override,
    /// `claude.exe` on `PATH`.
    Path,
    /// `<home>/.local/bin/claude.exe` (native installer).
    NativeInstaller,
    /// `claude.cmd` on `PATH` (npm).
    NpmShim,
}

/// A resolved Claude Code executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executable {
    pub path: PathBuf,
    pub kind: ExeKind,
    pub source: ExeSource,
}

impl Executable {
    /// The program to spawn and the arguments that go before the CLI's own.
    pub fn program_and_prefix(&self) -> (OsString, Vec<OsString>) {
        match self.kind {
            ExeKind::Native => (self.path.as_os_str().to_owned(), Vec::new()),
            ExeKind::CmdShim => (
                "cmd".into(),
                vec!["/d".into(), "/c".into(), self.path.as_os_str().to_owned()],
            ),
        }
    }
}

/// Resolves the executable in §10's order: the override, else `claude.exe` on `PATH`, else
/// `<home>/.local/bin/claude.exe`, else `claude.cmd` on `PATH`. Off Windows the names are
/// `claude` and `<home>/.local/bin/claude`, and there is no shim step. `is_file` is the
/// file-system seam.
pub fn resolve_executable(
    override_path: Option<&Path>,
    path_var: Option<&OsStr>,
    home: Option<&Path>,
    windows: bool,
    is_file: &dyn Fn(&Path) -> bool,
) -> Option<Executable> {
    let kind_of = |p: &Path| {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if windows && (ext == "cmd" || ext == "bat") {
            ExeKind::CmdShim
        } else {
            ExeKind::Native
        }
    };
    if let Some(p) = override_path {
        return is_file(p).then(|| Executable {
            path: p.to_owned(),
            kind: kind_of(p),
            source: ExeSource::Override,
        });
    }
    let dirs: Vec<PathBuf> = path_var
        .map(|v| std::env::split_paths(v).collect())
        .unwrap_or_default();
    let on_path = |name: &str| {
        dirs.iter()
            .map(|d| d.join(name))
            .find(|p| is_file(p.as_path()))
    };
    let native_name = if windows { "claude.exe" } else { "claude" };
    if let Some(path) = on_path(native_name) {
        return Some(Executable {
            path,
            kind: ExeKind::Native,
            source: ExeSource::Path,
        });
    }
    if let Some(home) = home {
        let p = home.join(".local").join("bin").join(native_name);
        if is_file(&p) {
            return Some(Executable {
                path: p,
                kind: ExeKind::Native,
                source: ExeSource::NativeInstaller,
            });
        }
    }
    if windows && let Some(path) = on_path("claude.cmd") {
        return Some(Executable {
            path,
            kind: ExeKind::CmdShim,
            source: ExeSource::NpmShim,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prm_34_chat_args_snapshot() {
        let args = chat_args("SYS");
        let want = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--tools",
            "",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--system-prompt",
            "SYS",
        ];
        assert_eq!(args, want);
        // Never --bare, --resume, --model, --allowedTools or file paths (§10).
        for banned in [
            "--bare",
            "--resume",
            "--model",
            "--allowedTools",
            "--safe-mode",
        ] {
            assert!(!args.iter().any(|a| a == banned), "{banned}");
        }
    }

    #[test]
    fn fr_prm_45_child_env_sets_config_dir_and_drops_pf_vars() {
        let parent = [
            ("PATH", "/bin"),
            ("PF_DATA_DIR", "/x"),
            ("pf_fault", "crash"),
            ("PF_CHAT_REAL", "1"),
            ("CLAUDE_CONFIG_DIR", "/elsewhere/.claude"),
            ("DISABLE_AUTOUPDATER", "0"),
            ("ANTHROPIC_API_KEY", "kept"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let env = child_env(parent, Path::new("/data/claude-config"));
        let get = |k: &str| {
            env.iter()
                .filter(|(n, _)| n == k)
                .map(|(_, v)| v.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(get("PATH"), ["/bin"]);
        assert_eq!(get(ENV_CONFIG_DIR), ["/data/claude-config"]);
        assert_eq!(get("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"), ["1"]);
        assert_eq!(get("DISABLE_AUTOUPDATER"), ["1"]);
        // The API key stays: it overrides the plan login and detection reports it (§10).
        assert_eq!(get("ANTHROPIC_API_KEY"), ["kept"]);
        assert!(
            env.iter()
                .all(|(k, _)| !k.to_string_lossy().to_uppercase().starts_with("PF_"))
        );
    }

    fn fs(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| {
            let s = p.to_string_lossy().replace('\\', "/");
            files.contains(&s.as_str())
        }
    }

    #[test]
    fn fr_prm_33_executable_resolution_order() {
        let path = std::env::join_paths(["/a", "/b"]).ok();
        let path = path.as_deref();
        let home = Some(Path::new("/h"));

        // Override wins, and its kind follows the extension.
        let all = fs(&["/o/claude.cmd", "/a/claude.exe", "/h/.local/bin/claude.exe"]);
        let r = resolve_executable(Some(Path::new("/o/claude.cmd")), path, home, true, &all);
        assert_eq!(
            r.map(|e| (e.kind, e.source)),
            Some((ExeKind::CmdShim, ExeSource::Override))
        );
        // A missing override is not silently replaced.
        assert_eq!(
            resolve_executable(Some(Path::new("/nope")), path, home, true, &all),
            None
        );

        let r = resolve_executable(None, path, home, true, &all).map(|e| e.source);
        assert_eq!(r, Some(ExeSource::Path));

        let native = fs(&["/h/.local/bin/claude.exe", "/b/claude.cmd"]);
        let r = resolve_executable(None, path, home, true, &native).map(|e| e.source);
        assert_eq!(r, Some(ExeSource::NativeInstaller));

        let npm = fs(&["/b/claude.cmd"]);
        let r = resolve_executable(None, path, home, true, &npm);
        assert_eq!(
            r.as_ref().map(|e| (e.kind, e.source)),
            Some((ExeKind::CmdShim, ExeSource::NpmShim))
        );
        let (prog, prefix) = r.map(|e| e.program_and_prefix()).unwrap_or_default();
        assert_eq!(prog, "cmd");
        assert_eq!(prefix.len(), 3);

        // Off Windows: plain `claude`, no shim step.
        assert_eq!(resolve_executable(None, path, home, false, &npm), None);
        let unix = fs(&["/b/claude"]);
        let r = resolve_executable(None, path, home, false, &unix).map(|e| e.kind);
        assert_eq!(r, Some(ExeKind::Native));
    }
}
