//! Tests for the per-window command ACL (R96): every app command is declared in
//! `app_commands.rs` (build.rs turns it into `allow-<command>` permissions), the main window is
//! granted all of them, and the prompt chat window only what it calls. Tauri itself fails the
//! build on a permission that names no declared command, and refuses at run time any app
//! command a window's capabilities don't grant.

use std::collections::BTreeSet;

use serde_json::Value;

include!("../app_commands.rs");

/// The Tauri-layer commands next to the core ones (`commands::handler`).
const APP_LEVEL: [&str; 7] = [
    "app_ready",
    "prm_window_open",
    "prm_chat_login",
    "app_close_guard",
    "app_exit",
    "app_close_unguard",
    "app_close_ack",
];

/// What `ChatWindow.tsx` calls (R96). Settings, logout, provider, library and export are not
/// among them; turning the chat on from the chat window goes through the main window.
const CHAT_WINDOW: [&str; 9] = [
    "prm_chat_status",
    "prm_chat_consent",
    "prm_chat_thread_get",
    "prm_chat_preview",
    "prm_chat_send",
    "prm_chat_cancel",
    "prm_chat_insert",
    "prm_chat_login",
    "project_get",
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

#[test]
fn fr_prm_40_chat_window_is_granted_only_what_it_calls() {
    let chat = granted("prompt-chat.json");
    assert_eq!(chat, set(&CHAT_WINDOW));
    for refused in [
        "prm_chat_settings_put",
        "prm_chat_logout",
        "prm_context_publish",
        "provider_credentials_set",
        "library_set_roots",
        "export_start",
    ] {
        assert!(!chat.contains(refused), "{refused}");
    }
    assert_eq!(
        capability("prompt-chat.json")["windows"],
        serde_json::json!(["prompt-chat"])
    );
}
