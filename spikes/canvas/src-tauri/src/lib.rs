//! Throwaway Tauri shell for the canvas spike.
//! Tiles reach the webview two ways, to compare them:
//!  - `tiles://` custom URI scheme (http://tiles.localhost/<size>/<level>/<c>_<r>.jpeg on Windows), bytes streamed as-is;
//!  - IPC commands `tile_b64` (base64 string in JSON) and `tile_raw` (binary IPC response).
//! Tiles are pre-cut by scripts/gen-fixtures.mjs; the imaging spike covers a real Rust tile cutter.
use base64::Engine;
use std::{fs, path::PathBuf};
use tauri::{
    http::{header, Response, StatusCode},
    AppHandle, WebviewUrl, WebviewWindowBuilder,
};

fn fixtures() -> PathBuf {
    std::env::var("SPIKE_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../public/fixtures"))
}

fn tile_path(size: u32, level: u32, col: u32, row: u32) -> PathBuf {
    fixtures().join(format!("tiles-{size}/{level}/{col}_{row}.jpeg"))
}

#[tauri::command]
fn manifest(size: u32) -> Result<String, String> {
    fs::read_to_string(fixtures().join(format!("tiles-{size}/manifest.json"))).map_err(|e| e.to_string())
}

#[tauri::command]
async fn tile_b64(size: u32, level: u32, col: u32, row: u32) -> Result<String, String> {
    let bytes = fs::read(tile_path(size, level, col, row)).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
async fn tile_raw(size: u32, level: u32, col: u32, row: u32) -> Result<tauri::ipc::Response, String> {
    let bytes = fs::read(tile_path(size, level, col, row)).map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Query string for the page (e.g. `autorun=tauri&lib=konva`), from SPIKE_QUERY.
#[tauri::command]
fn autorun() -> String {
    std::env::var("SPIKE_QUERY").unwrap_or_default()
}

/// The page posts its results here; written to results/raw/tauri-<lib>.json, then the app exits.
#[tauri::command]
fn report(app: AppHandle, json: String) {
    let lib = serde_json::from_str::<serde_json::Value>(&json)
        .ok()
        .and_then(|v| v.get("lib").and_then(|l| l.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".into());
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../results/raw");
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join(format!("tauri-{lib}.json")), &json);
    println!("SPIKE_REPORT written for {lib}");
    app.exit(0);
}

pub fn run() {
    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("tiles", |_ctx, req, responder| {
            // Path: /<size>/<level>/<c>_<r>.jpeg or /<size>/manifest.json
            let path = req.uri().path().trim_start_matches('/').to_string();
            tauri::async_runtime::spawn_blocking(move || {
                let mut parts = path.splitn(2, '/');
                let (size, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let ok = !path.contains("..") && size.chars().all(|c| c.is_ascii_digit());
                let file = fixtures().join(format!("tiles-{size}")).join(rest);
                let res = match (ok, fs::read(&file)) {
                    (true, Ok(bytes)) => Response::builder()
                        .header(header::CONTENT_TYPE, if rest.ends_with(".json") { "application/json" } else { "image/jpeg" })
                        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                        .header(header::CACHE_CONTROL, "max-age=3600")
                        .body(bytes),
                    _ => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()),
                };
                responder.respond(res.unwrap());
            });
        })
        .invoke_handler(tauri::generate_handler![manifest, tile_b64, tile_raw, autorun, report])
        .setup(|app| {
            #[allow(unused_mut)]
            let mut b = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Pocket Forge canvas spike")
                .inner_size(1640.0, 980.0);
            #[cfg(windows)]
            {
                // Tauri's defaults plus optional extras, e.g. "--force_high_performance_gpu --disable-gpu-vsync".
                let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
                if let Ok(extra) = std::env::var("SPIKE_BROWSER_ARGS") {
                    args.push(' ');
                    args.push_str(&extra);
                }
                b = b.additional_browser_args(&args);
            }
            b.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
