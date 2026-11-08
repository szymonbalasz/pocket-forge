//! `tauri:smoke` export step (export spec §13, slice 5). Debug builds only: never compiled into
//! release. With `PF_SMOKE_EXPORT_IMAGE` set, the smoke run creates a Maps project from that
//! image in `PF_LIBRARY_DIR`, exports it to PNG through the same core commands the UI calls, and
//! prints `PF_SMOKE export ok <file> <w>x<h> <bytes>`; `scripts/tauri-smoke.mjs` then decodes the
//! file and checks its size. No logic here beyond calling commands in order and waiting.

use std::path::Path;
use std::time::{Duration, Instant};

use pf_core::Core;
use serde_json::{Value, json};

/// The image to export, if this smoke run has an export step.
pub fn image() -> Option<String> {
    std::env::var("PF_SMOKE_EXPORT_IMAGE")
        .ok()
        .filter(|p| !p.trim().is_empty())
}

/// Polls `check` every 100 ms until it yields a value or `limit` passes.
fn wait<T>(
    what: &str,
    limit: Duration,
    mut check: impl FnMut() -> Result<Option<T>, String>,
) -> Result<T, String> {
    let start = Instant::now();
    loop {
        if let Some(v) = check()? {
            return Ok(v);
        }
        if start.elapsed() > limit {
            return Err(format!("timed out waiting for {what}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Runs the export step; returns `<file> <w>x<h> <bytes>`.
pub fn run(core: &Core, image: &str) -> Result<String, String> {
    let call = |name: &str, input: Value| {
        core.dispatch(name, input)
            .map_err(|e| format!("{name}: {e}"))
    };
    let created = call(
        "project_create",
        json!({ "section": "maps", "paths": [image] }),
    )?;
    let item = &created["items"][0];
    let project = item["path"]
        .as_str()
        .ok_or_else(|| format!("project_create: {item}"))?
        .to_owned();
    call("project_open", json!({ "path": project }))?;
    wait("the decoded original", Duration::from_secs(60), || {
        let r = call("project_prefetch_original", json!({}))?;
        Ok((r["ready"] == true).then_some(()))
    })?;
    let settings = json!({
        "grid": { "mode": "px" },
        "target_px_per_square": null,
        "format": "png",
    });
    let plan = call("export_plan", json!({ "settings": settings }))?;
    call(
        "export_start",
        json!({ "settings": settings, "replace": true }),
    )?;
    let last = wait("the export record", Duration::from_secs(120), || {
        let snap = call("project_get", json!({}))?;
        let last = &snap["export"]["last"];
        Ok(last.is_object().then(|| last.clone()))
    })?;
    if last["path"] != plan["path"] {
        return Err(format!(
            "exported {} but planned {}",
            last["path"], plan["path"]
        ));
    }
    let rel = last["path"].as_str().unwrap_or_default();
    let file = rel
        .split('/')
        .fold(Path::new(&project).to_path_buf(), |p, part| p.join(part));
    call("project_close", json!({}))?;
    Ok(format!(
        "{} {}x{} {}",
        file.display(),
        last["width"],
        last["height"],
        last["bytes"]
    ))
}
