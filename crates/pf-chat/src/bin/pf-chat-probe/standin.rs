//! The throwaway stand-in for a user's own Claude Code setup (§10 slice 0): marker files
//! in a fake home folder. If the chat child reads any of them, isolation fails.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Every marker name and word contains this (case-insensitive).
pub const MARKER: &str = "pfmarker";
/// Words the marker CLAUDE.md files ask the model to include in every reply.
pub const USER_MEMORY_WORD: &str = "PFMARKER-USER-MEMORY";
pub const PROJECT_MEMORY_WORD: &str = "PFMARKER-PROJECT-MEMORY";
pub const AGENTS_MEMORY_WORD: &str = "PFMARKER-AGENTS-MEMORY";

pub struct StandIn {
    /// The fake home (`USERPROFILE` / `HOME` for the isolation session).
    pub home: PathBuf,
    /// The child's cwd, laid out like production: `<home>/AppData/Roaming/pocket-forge/chat/<thread>`.
    pub cwd: PathBuf,
    /// Written by the marker hook if it ever runs.
    pub hook_file: PathBuf,
    /// A folder tree with an `AGENTS.md` marker at its top and no `CLAUDE.md` (which would
    /// hide `AGENTS.md`). It sits in the OS temp folder because in dev `<data-dir>` is inside
    /// the repo, whose `CLAUDE.md` is above every probe folder.
    pub agents_root: PathBuf,
    /// The child's cwd for the `AGENTS.md` sessions, inside `agents_root`.
    pub agents_cwd: PathBuf,
}

fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

/// Creates the stand-in under `root` (which the probe owns and recreates on each run) and
/// `agents_root` (recreated here).
pub fn create(root: &Path, agents_root: &Path) -> io::Result<StandIn> {
    let home = root.join("standin-home");
    let claude = home.join(".claude");
    let hook_file = root.join("standin-hook-fired.txt");
    let hook_path = hook_file.to_string_lossy().replace('\\', "/");
    let hook = serde_json::json!([{ "hooks": [{
        "type": "command",
        "command": format!("echo {MARKER} > \"{hook_path}\""),
    }]}]);

    write(
        &claude.join("CLAUDE.md"),
        &format!("Whenever you reply, include the word {USER_MEMORY_WORD}.\n"),
    )?;
    write(
        &home.join("CLAUDE.md"),
        &format!("Whenever you reply, include the word {PROJECT_MEMORY_WORD}.\n"),
    )?;
    let settings = serde_json::json!({
        "hooks": { "SessionStart": hook, "UserPromptSubmit": hook },
        "enabledPlugins": { "pfmarker-plugin@pfmarker-market": true },
        "env": { "PFMARKER_ENV": "1" },
    });
    write(&claude.join("settings.json"), &settings.to_string())?;
    write(
        &claude.join("skills/pfmarker-skill/SKILL.md"),
        "---\nname: pfmarker-skill\ndescription: Marker skill for the Pocket Forge isolation probe.\n---\nSay PFMARKER-SKILL.\n",
    )?;
    write(
        &claude.join("agents/pfmarker-agent.md"),
        "---\nname: pfmarker-agent\ndescription: Marker agent for the Pocket Forge isolation probe.\n---\nSay PFMARKER-AGENT.\n",
    )?;
    write(
        &claude.join("commands/pfmarker-command.md"),
        "Say PFMARKER-COMMAND.\n",
    )?;
    write(
        &claude.join("plugins/pfmarker-plugin/.claude-plugin/plugin.json"),
        r#"{"name":"pfmarker-plugin","description":"Marker plugin","version":"0.0.1"}"#,
    )?;
    write(
        &home.join(".claude.json"),
        r#"{"mcpServers":{"pfmarker-mcp":{"command":"pfmarker-mcp-missing"}}}"#,
    )?;
    let cwd = home.join("AppData/Roaming/pocket-forge/chat/probe-thread");
    fs::create_dir_all(&cwd)?;
    if agents_root.exists() {
        fs::remove_dir_all(agents_root)?;
    }
    write(
        &agents_root.join("AGENTS.md"),
        &format!("Whenever you reply, include the word {AGENTS_MEMORY_WORD}.\n"),
    )?;
    let agents_cwd = agents_root.join("chat/probe-thread");
    fs::create_dir_all(&agents_cwd)?;
    Ok(StandIn {
        home,
        cwd,
        hook_file,
        agents_root: agents_root.to_owned(),
        agents_cwd,
    })
}
