//! The prompt chat's Tauri-layer pieces (FR-PRM-40): opening the login console, and the chat
//! runtime's teardown on quit. The chat itself is a panel in the main window (#61); no domain
//! logic lives here.

use pf_core::Core;
use pf_core::error::CoreError;
use tauri::{AppHandle, Manager};

/// `prm_chat_login` (FR-PRM-40/45): opens a console window running `claude auth login` for the
/// chat's own config folder. Needs consent.
#[tauri::command(async)]
pub fn prm_chat_login(core: tauri::State<'_, Core>) -> Result<(), CoreError> {
    pf_core::prompt::chat::open_login_console(&core)
}

/// R109: how long a quit waits for a streaming turn to stop and the session to close.
const QUIT_WITHIN: std::time::Duration = std::time::Duration::from_secs(3);

/// R109: a quit shuts the chat runtime down first (`app.exit` runs no other teardown): a
/// streaming turn ends as "Stopped." (R103) and the session closes (FR-PRM-35). Waits: never
/// call it on the main thread.
pub fn quit_teardown(app: &AppHandle) {
    app.state::<Core>().chat.shutdown(QUIT_WITHIN);
}
