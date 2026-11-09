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
        .min_inner_size(MIN.0, MIN.1);
    let (w, h) = initial_size(rect);
    b = b.inner_size(w, h);
    if let Some(r) = rect
        && on_a_monitor(&app, r)
    {
        b = b.position(f64::from(r.x), f64::from(r.y));
    }
    let w = b
        .build()
        .map_err(|e| CoreError::internal(format!("can't open the chat window: {e}")))?;
    let handle = app.clone();
    let win = w.clone();
    w.on_window_event(move |ev| match ev {
        WindowEvent::CloseRequested { .. } => save_rect(&handle, &win),
        // FR-PRM-35: closing the window ends the backend session; the thread is kept.
        WindowEvent::Destroyed => handle.state::<Core>().chat.end_session(),
        _ => {}
    });
    let _ = w.set_focus();
    Ok(())
}

/// FR-PRM-21: the remembered size (never below the minimum), or the first-run size.
fn initial_size(rect: Option<ChatWindowRect>) -> (f64, f64) {
    rect.map_or(FIRST, |r| {
        (f64::from(r.w).max(MIN.0), f64::from(r.h).max(MIN.1))
    })
}

/// A monitor's logical rect: x, y, width, height.
type Monitor = (f64, f64, f64, f64);

/// A remembered position is used only while some monitor still shows its top-left corner,
/// with at least 40 logical px of the window inside it.
fn shows_corner(r: ChatWindowRect, monitors: &[Monitor]) -> bool {
    let (x, y) = (f64::from(r.x), f64::from(r.y));
    monitors
        .iter()
        .any(|&(mx, my, mw, mh)| x >= mx && y >= my && x < mx + mw - 40.0 && y < my + mh - 40.0)
}

fn on_a_monitor(app: &AppHandle, r: ChatWindowRect) -> bool {
    let monitors: Vec<Monitor> = app
        .available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    let s = m.scale_factor();
                    let p = m.position().to_logical::<f64>(s);
                    let z = m.size().to_logical::<f64>(s);
                    (p.x, p.y, z.width, z.height)
                })
                .collect()
        })
        .unwrap_or_default();
    shows_corner(r, &monitors)
}

/// The rect to remember from the window's logical position and inner size (rounded, never
/// below the minimum).
fn rect_to_save(x: f64, y: f64, w: f64, h: f64) -> ChatWindowRect {
    ChatWindowRect {
        x: x.round() as i32,
        y: y.round() as i32,
        w: (w.round() as u32).max(MIN.0 as u32),
        h: (h.round() as u32).max(MIN.1 as u32),
    }
}

/// Stores the window's logical rect through `prm_chat_settings_put`.
fn save_rect(app: &AppHandle, w: &WebviewWindow) {
    let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.inner_size(), w.scale_factor())
    else {
        return;
    };
    let pos = pos.to_logical::<f64>(scale);
    let size = size.to_logical::<f64>(scale);
    let rect = rect_to_save(pos.x, pos.y, size.width, size.height);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: u32, h: u32) -> ChatWindowRect {
        ChatWindowRect { x, y, w, h }
    }

    #[test]
    fn fr_prm_21_label_matches_the_capability_file() {
        assert_eq!(LABEL, "prompt-chat");
        let cap: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/prompt-chat.json")).unwrap();
        assert_eq!(cap["windows"], serde_json::json!([LABEL]));
    }

    #[test]
    fn fr_prm_21_first_run_and_remembered_size_never_below_360_by_480() {
        assert_eq!(initial_size(None), (420.0, 640.0));
        assert_eq!(initial_size(Some(rect(10, 20, 800, 900))), (800.0, 900.0));
        assert_eq!(initial_size(Some(rect(10, 20, 100, 100))), (360.0, 480.0));
    }

    #[test]
    fn fr_prm_21_a_remembered_position_needs_a_monitor_showing_its_corner() {
        let two = [(0.0, 0.0, 1920.0, 1080.0), (1920.0, 0.0, 1280.0, 1024.0)];
        assert!(shows_corner(rect(100, 100, 420, 640), &two));
        assert!(
            shows_corner(rect(2000, 50, 420, 640), &two),
            "second monitor"
        );
        assert!(
            !shows_corner(rect(-500, 100, 420, 640), &two),
            "off to the left"
        );
        assert!(
            !shows_corner(rect(3190, 100, 420, 640), &two),
            "under 40 px left"
        );
        assert!(
            !shows_corner(rect(100, 1050, 420, 640), &two),
            "below the bottom"
        );
        assert!(!shows_corner(rect(100, 100, 420, 640), &[]), "no monitors");
    }

    #[test]
    fn fr_prm_21_the_saved_rect_is_rounded_and_never_below_the_minimum() {
        assert_eq!(
            rect_to_save(10.4, -20.6, 500.5, 700.2),
            rect(10, -21, 501, 700)
        );
        assert_eq!(rect_to_save(0.0, 0.0, 200.0, 300.0), rect(0, 0, 360, 480));
    }
}
