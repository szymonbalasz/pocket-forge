//! The thin Tauri layer (AGENTS §9): command registration, the `pf-tiles` URI scheme, the
//! window and event forwarding. No domain logic: everything is a `pf-core` command.

mod commands;

use pf_core::context::EventSink;
use pf_core::{Context, Core};
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager};

/// Forwards core events to the webview.
struct TauriEvents(AppHandle);

impl EventSink for TauriEvents {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if let Err(e) = self.0.emit(event, payload) {
            tracing::warn!(event, error = %e, "event not delivered");
        }
    }
}

/// `tauri:smoke` steps. Debug builds only: none of this is compiled into release (R46).
#[cfg(debug_assertions)]
mod smoke {
    use std::sync::atomic::{AtomicBool, Ordering};

    static TILE_ASKED: AtomicBool = AtomicBool::new(false);

    pub fn enabled() -> bool {
        std::env::var("PF_SMOKE").is_ok_and(|v| v == "1")
    }

    /// `PF_SMOKE_TILE_IMAGE`, handed out once, after the shell reported ok (BL-07).
    pub fn tile_image(ok: bool) -> Option<String> {
        if !ok || TILE_ASKED.swap(true, Ordering::SeqCst) {
            return None;
        }
        std::env::var("PF_SMOKE_TILE_IMAGE")
            .ok()
            .filter(|p| !p.is_empty())
    }
}

/// The UI calls this once it has rendered and reached the core. In debug builds with
/// `PF_SMOKE=1` it ends the process (exit 0 if `ok`), which is how `tauri:smoke` knows the
/// window loaded. With `PF_SMOKE_TILE_IMAGE` set as well, the first `ok` report instead returns
/// that image: the UI opens it through the core, fetches tile 0/0_0 through the `pf-tiles`
/// scheme and reports again (R46). Otherwise it only logs and returns null.
#[tauri::command]
fn app_ready(app: AppHandle, ok: bool, detail: String) -> Option<String> {
    tracing::info!(ok, %detail, "UI ready");
    #[cfg(debug_assertions)]
    if smoke::enabled() {
        if let Some(image) = smoke::tile_image(ok) {
            println!("PF_SMOKE shell ok {detail}");
            return Some(image);
        }
        println!("PF_SMOKE {} {detail}", if ok { "ok" } else { "failed" });
        app.exit(if ok { 0 } else { 1 });
    }
    #[cfg(not(debug_assertions))]
    let _ = app;
    None
}

fn scheme_response(core: &Core, path: &str) -> Response<Vec<u8>> {
    let res = core.serve_scheme(path);
    let mut b = Response::builder()
        .status(StatusCode::from_u16(res.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    if let Some(ct) = res.content_type {
        b = b.header(header::CONTENT_TYPE, ct);
    }
    if res.cache_immutable {
        b = b.header(header::CACHE_CONTROL, "max-age=31536000, immutable");
    }
    b.body(res.body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    if cfg!(debug_assertions) && std::env::var("PF_SMOKE").is_ok_and(|v| v == "1") {
        // Never hang CI: give up if the UI doesn't report within 90 s.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(90));
            eprintln!("PF_SMOKE timeout: the UI never called app_ready");
            std::process::exit(3);
        });
    }

    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("pf-tiles", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_owned();
            tauri::async_runtime::spawn_blocking(move || {
                let core = app.state::<Core>();
                responder.respond(scheme_response(&core, &path));
            });
        })
        .invoke_handler(commands::handler())
        .setup(|app| {
            let mut ctx = Context::from_env()?;
            ctx.events = std::sync::Arc::new(TauriEvents(app.handle().clone()));
            tracing::info!(?ctx, "starting");
            let core = Core::new(ctx).map_err(|e| e.to_string())?;
            app.manage(core);
            Ok(())
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("Pocket Forge failed to start: {e}");
            std::process::exit(1);
        });
}
