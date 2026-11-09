// R96: the app commands are declared, so Tauri's ACL checks them per window (capabilities/).
include!("app_commands.rs");

fn main() {
    println!("cargo:rerun-if-changed=app_commands.rs");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
