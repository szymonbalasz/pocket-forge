//! The thin Tauri layer (AGENTS §9): command registration, the `pf-tiles` URI scheme, the
//! window and event forwarding. No domain logic: everything is a `pf-core` command.

#[cfg(test)]
mod acl;
mod chat_window;
mod commands;
#[cfg(debug_assertions)]
mod smoke_export;

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
/// scheme and reports again (R46). With `PF_SMOKE_EXPORT_IMAGE` set too, the final `ok` report
/// first runs the export step (`smoke_export`) on a worker thread, then exits. Otherwise it only
/// logs and returns null.
#[tauri::command]
fn app_ready(app: AppHandle, ok: bool, detail: String) -> Option<String> {
    tracing::info!(ok, %detail, "UI ready");
    #[cfg(debug_assertions)]
    if smoke::enabled() {
        if let Some(image) = smoke::tile_image(ok) {
            println!("PF_SMOKE shell ok {detail}");
            return Some(image);
        }
        if ok && let Some(image) = smoke_export::image() {
            std::thread::spawn(move || {
                let result = smoke_export::run(&app.state::<Core>(), &image);
                match &result {
                    Ok(d) => println!("PF_SMOKE export ok {d}"),
                    Err(e) => println!("PF_SMOKE export failed {e}"),
                }
                println!(
                    "PF_SMOKE {} {detail}",
                    if result.is_ok() { "ok" } else { "failed" }
                );
                app.exit(if result.is_ok() { 0 } else { 1 });
            });
            return None;
        }
        println!("PF_SMOKE {} {detail}", if ok { "ok" } else { "failed" });
        app.exit(if ok { 0 } else { 1 });
    }
    #[cfg(not(debug_assertions))]
    let _ = app;
    None
}

/// R101: the main window's X (or quitting the app) is a project close. Once the UI said it runs
/// the close guards (`app_close_guard`), a close request is held and handed to it as
/// `app:close-requested`; the UI closes the project through its guards and then calls
/// `app_exit`. Before that (the UI never loaded), X closes as it always did. R109: the UI acks
/// each request (`app_close_ack`); one not acked within `ACK_WITHIN` (a crashed or unmounted UI)
/// closes the app anyway, and the UI un-guards when its listener goes (`app_close_unguard`).
mod quit {
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
    use std::time::Duration;

    /// R109: how long a held close waits for the UI to say it got it.
    pub const ACK_WITHIN: Duration = Duration::from_secs(3);

    pub struct Quit {
        /// Guards installed minus removed. Signed: a removal that arrives before its install
        /// (React's dev double mount) still nets out.
        guards: AtomicI64,
        exiting: AtomicBool,
        /// Close requests sent to the UI, and the latest one it acked.
        asked: AtomicU64,
        acked: AtomicU64,
    }

    pub static QUIT: Quit = Quit::new();

    impl Quit {
        pub const fn new() -> Self {
            Self {
                guards: AtomicI64::new(0),
                exiting: AtomicBool::new(false),
                asked: AtomicU64::new(0),
                acked: AtomicU64::new(0),
            }
        }

        pub fn guard(&self) {
            self.guards.fetch_add(1, Ordering::SeqCst);
        }

        pub fn unguard(&self) {
            self.guards.fetch_sub(1, Ordering::SeqCst);
        }

        /// The app is exiting from here on: a close is never held again.
        pub fn exiting(&self) {
            self.exiting.store(true, Ordering::SeqCst);
        }

        /// True if a close request must wait for the UI.
        pub fn hold(&self) -> bool {
            self.guards.load(Ordering::SeqCst) > 0 && !self.exiting.load(Ordering::SeqCst)
        }

        /// A close request goes to the UI: its number.
        pub fn ask(&self) -> u64 {
            self.asked.fetch_add(1, Ordering::SeqCst) + 1
        }

        /// The UI got the close requests sent so far.
        pub fn ack(&self) {
            let asked = self.asked.load(Ordering::SeqCst);
            self.acked.fetch_max(asked, Ordering::SeqCst);
        }

        /// True if request `seq` is still unacked (and nothing else is exiting the app).
        pub fn unanswered(&self, seq: u64) -> bool {
            self.acked.load(Ordering::SeqCst) < seq && !self.exiting.load(Ordering::SeqCst)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::Quit;

        #[test]
        fn r101_a_close_waits_for_the_ui_only_while_it_guards_and_not_once_it_exits() {
            let q = Quit::new();
            assert!(!q.hold(), "no UI yet: X closes");
            q.guard();
            assert!(q.hold(), "the UI runs the close guards");
            q.exiting();
            assert!(!q.hold(), "the UI said exit");
        }

        #[test]
        fn r109_fr_prj_21_the_ui_unguards_when_its_listener_goes() {
            let q = Quit::new();
            q.guard();
            q.unguard();
            assert!(!q.hold(), "the UI unmounted (a crash): X closes");
            // React's dev double mount, the removal arriving first: still one guard.
            q.unguard();
            q.guard();
            q.guard();
            assert!(q.hold());
        }

        #[test]
        fn r109_fr_prj_21_a_close_the_ui_doesnt_ack_is_unanswered() {
            let q = Quit::new();
            q.guard();
            let first = q.ask();
            assert!(q.unanswered(first), "no ack yet: closes after ACK_WITHIN");
            q.ack();
            assert!(!q.unanswered(first), "acked: the UI's guards decide");
            let second = q.ask();
            assert!(q.unanswered(second), "every request needs its own ack");
            q.ack();
            assert!(!q.unanswered(second));
            let third = q.ask();
            q.exiting();
            assert!(!q.unanswered(third), "already exiting");
        }
    }
}

/// The UI's close guards are listening for `app:close-requested` (R101).
#[tauri::command]
fn app_close_guard() {
    quit::QUIT.guard();
}

/// The UI's close guards stopped listening (R109): X no longer waits for them.
#[tauri::command]
fn app_close_unguard() {
    quit::QUIT.unguard();
}

/// The UI got `app:close-requested` (R109): the held close is its guards' to decide.
#[tauri::command]
fn app_close_ack() {
    quit::QUIT.ack();
}

/// The UI closed the project (or none was open): the app exits (R101).
#[tauri::command]
fn app_exit(app: AppHandle) {
    exit_app(&app);
}

/// Ends the app.
fn exit_app(app: &AppHandle) {
    quit::QUIT.exiting();
    app.exit(0);
}

/// R109: a held close the UI didn't ack within `ACK_WITHIN` closes the app anyway.
fn close_unless_acked(app: AppHandle, seq: u64) {
    let spawned = std::thread::Builder::new()
        .name("pf-close-ack".into())
        .spawn(move || {
            std::thread::sleep(quit::ACK_WITHIN);
            if quit::QUIT.unanswered(seq) {
                tracing::warn!("the UI didn't answer the close request: closing anyway");
                exit_app(&app);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "no close timeout");
    }
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
        .plugin(tauri_plugin_dialog::init())
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
            // RGN's job runner polls on its own thread while the app runs (FR-RGN-35).
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("pf-rgn-runner".into())
                .spawn(move || {
                    let core = handle.state::<Core>();
                    pf_core::region_edit::runner::run_pump(&core)
                })?;
            // R101: X asks the UI's close guards first. PRM FR-PRM-21: the chat window closes
            // with the main window (its own X is not held).
            if let Some(main) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                main.on_window_event(move |ev| match ev {
                    tauri::WindowEvent::CloseRequested { api, .. } if quit::QUIT.hold() => {
                        api.prevent_close();
                        let seq = quit::QUIT.ask();
                        if let Err(e) = handle.emit_to("main", "app:close-requested", ()) {
                            tracing::warn!(error = %e, "close request not delivered");
                        }
                        close_unless_acked(handle.clone(), seq);
                    }
                    tauri::WindowEvent::Destroyed => chat_window::close_with_main(&handle),
                    _ => {}
                });
            }
            // FR-PRJ-21/40: retry saves that failed with `locked` and replay the spool.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(pf_core::project::SAVE_RETRY_EVERY);
                    pf_core::project::commands::retry_pending_saves(&handle.state::<Core>());
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("Pocket Forge failed to start: {e}");
            std::process::exit(1);
        });
}
