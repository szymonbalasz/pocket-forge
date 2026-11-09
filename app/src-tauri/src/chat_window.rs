//! The prompt chat pop-out window (PRM slice 6, FR-PRM-21, R91): label `prompt-chat`, created
//! at runtime from Rust (not declared in `tauri.conf.json`); its capability grants
//! `core:default` only. The two Tauri-layer commands of FR-PRM-40 live here: opening the window
//! and opening the login console. No domain logic: the rect and the session end go through
//! `pf-core`.

use pf_core::Core;
use pf_core::error::CoreError;
use pf_core::prompt::chat::{ChatSettingsInput, PrmChatSettingsPut};
use pf_core::settings::ChatWindowRect;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// The window label (R91; the capability file names it).
pub const LABEL: &str = "prompt-chat";
/// FR-PRM-21: the minimum size, and the first-run size.
const MIN: (f64, f64) = (360.0, 480.0);
const FIRST: (f64, f64) = (420.0, 640.0);

/// `prm_window_open` (FR-PRM-21): opens the chat window, or focuses it when it is open. The
/// size and position are remembered per machine (`prompt_chat.window`).
#[tauri::command(async)]
pub fn prm_window_open(app: AppHandle) -> Result<(), CoreError> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let rect = app.state::<Core>().state().settings.prompt_chat.window;
    let mut b = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Prompt chat · Pocket Forge")
        .min_inner_size(MIN.0, MIN.1)
        .inner_size(FIRST.0, FIRST.1);
    if let Some(r) = rect {
        b = b.inner_size(f64::from(r.w).max(MIN.0), f64::from(r.h).max(MIN.1));
        if on_a_monitor(&app, r) {
            b = b.position(f64::from(r.x), f64::from(r.y));
        }
    }
    let w = b
        .build()
        .map_err(|e| CoreError::internal(format!("can't open the chat window: {e}")))?;
    let handle = app.clone();
    let win = w.clone();
    // These run on the main thread: neither waits for a process (code review 14). Saving the
    // rect alone detects nothing, and the session ends on its own thread.
    w.on_window_event(move |ev| match ev {
        WindowEvent::CloseRequested { .. } => save_rect(&handle, &win),
        // FR-PRM-35: closing the window ends the backend session; the thread is kept.
        WindowEvent::Destroyed => handle.state::<Core>().chat.end_session_in_background(),
        _ => {}
    });
    let _ = w.set_focus();
    Ok(())
}

/// A remembered position is used only while some monitor still shows its top-left corner.
fn on_a_monitor(app: &AppHandle, r: ChatWindowRect) -> bool {
    app.available_monitors().is_ok_and(|ms| {
        ms.iter().any(|m| {
            let s = m.scale_factor();
            let p = m.position().to_logical::<f64>(s);
            let z = m.size().to_logical::<f64>(s);
            let (x, y) = (f64::from(r.x), f64::from(r.y));
            x >= p.x && y >= p.y && x < p.x + z.width - 40.0 && y < p.y + z.height - 40.0
        })
    })
}

/// Stores the window's logical rect through `prm_chat_settings_put`.
fn save_rect(app: &AppHandle, w: &WebviewWindow) {
    let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.inner_size(), w.scale_factor())
    else {
        return;
    };
    let pos = pos.to_logical::<f64>(scale);
    let size = size.to_logical::<f64>(scale);
    let rect = ChatWindowRect {
        x: pos.x.round() as i32,
        y: pos.y.round() as i32,
        w: (size.width.round() as u32).max(MIN.0 as u32),
        h: (size.height.round() as u32).max(MIN.1 as u32),
    };
    let input = ChatSettingsInput {
        enabled: None,
        claude_path: None,
        window: Some(Some(rect)),
    };
    if let Err(e) = app.state::<Core>().run::<PrmChatSettingsPut>(input) {
        tracing::warn!(error = %e, "the chat window's size and position weren't saved");
    }
}

/// `prm_chat_login` (FR-PRM-40/45): opens a console window running `claude auth login` for the
/// chat's own config folder. Needs consent.
#[tauri::command(async)]
pub fn prm_chat_login(core: tauri::State<'_, Core>) -> Result<(), CoreError> {
    pf_core::prompt::chat::open_login_console(&core)
}

/// The main window closed: the chat window goes too (the app then exits).
pub fn close_with_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.close();
    }
}
