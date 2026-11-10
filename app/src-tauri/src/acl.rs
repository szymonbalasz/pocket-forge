//! Tests for the per-window command ACL (R96): every app command is declared in
//! `app_commands.rs` (build.rs turns it into `allow-<command>` permissions), and the main
//! window, the only one since the chat became a panel (#61), is granted all of them. Tauri
//! itself fails the build on a permission that names no declared command, and refuses at run
//! time any app command a window's capabilities don't grant.

use std::collections::BTreeSet;

use serde_json::Value;

include!("../app_commands.rs");

/// The Tauri-layer commands next to the core ones (`commands::handler`).
const APP_LEVEL: [&str; 7] = [
    "app_ready",
    "prm_chat_login",
    "app_close_guard",
    "app_exit",
    "app_close_unguard",
    "app_close_ack",
    "app_close_asking",
];

fn capability(file: &str) -> Value {
    let path = format!("{}/capabilities/{file}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The app commands a capability grants (`allow-<command>`, plugin and core entries skipped).
fn granted(file: &str) -> BTreeSet<String> {
    let all: BTreeSet<String> = APP_COMMANDS.iter().map(|c| c.replace('_', "-")).collect();
    capability(file)["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .filter_map(Value::as_str)
        .filter(|p| !p.contains(':'))
        .map(|p| {
            let cmd = p
                .strip_prefix("allow-")
                .unwrap_or_else(|| panic!("{file}: {p}"));
            assert!(all.contains(cmd), "{file}: {p} isn't an app command");
            cmd.replace('-', "_")
        })
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn r96_every_registered_command_is_declared() {
    let mut want: Vec<&str> = crate::commands::NAMES.to_vec();
    want.extend(APP_LEVEL);
    assert_eq!(
        APP_COMMANDS,
        want.as_slice(),
        "app_commands.rs must list the commands of src/commands.rs, in order"
    );
}

#[test]
fn r96_main_window_is_granted_every_app_command() {
    assert_eq!(granted("default.json"), set(APP_COMMANDS));
}

/// #61: the prompt chat is a panel in the main window; no other window has a capability.
#[test]
fn fr_prm_40_only_the_main_window_has_a_capability() {
    let dir = format!("{}/capabilities", env!("CARGO_MANIFEST_DIR"));
    let files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["default.json"]);
    assert_eq!(
        capability("default.json")["windows"],
        serde_json::json!(["main"])
    );
}
