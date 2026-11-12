//! Open in Explorer (FR-PRJ-59, #63): `reveal_in_explorer(path)` through the shell seam.

use std::path::Path;

use crate::context::Context;
use crate::error::{CoreError, CoreResult};
use crate::settings::Settings;

/// Opens Explorer at `path`, which must be inside a library root once `..` and links are
/// resolved (`outside_library`; a missing path → `not_found`). A folder opens itself; a file
/// opens its folder with the file selected. Only a failed spawn is an error (`io`). Nothing is
/// read, so it works on read-only, conflicted and unreadable projects alike.
pub fn reveal(ctx: &Context, settings: &Settings, path: &Path) -> CoreResult<()> {
    let real = super::resolve_in_library(ctx, settings, path)?;
    let select = !real.is_dir();
    ctx.shell.reveal(&real, select).map_err(|e| {
        CoreError::new(
            crate::error::ErrorCode::Io,
            format!("starting Explorer: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use serde_json::json;

    use crate::context::{Context, RecordingShell, Shell};
    use crate::core::Core;
    use crate::error::ErrorCode;
    use crate::time::Timestamp;

    struct T {
        _dir: tempfile::TempDir,
        core: Core,
        shell: Arc<RecordingShell>,
        lib: PathBuf,
        outside: PathBuf,
    }

    fn setup() -> T {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
        let lib = dir.path().join("lib");
        ctx.library_override = Some(lib.clone());
        let shell = Arc::new(RecordingShell::default());
        ctx.shell = shell.clone();
        let core = Core::new(ctx).unwrap();
        let outside = dir.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.png"), b"x").unwrap();
        T {
            _dir: dir,
            core,
            shell,
            lib,
            outside,
        }
    }

    fn real(p: &std::path::Path) -> String {
        crate::library::plain_path(std::fs::canonicalize(p).unwrap())
            .display()
            .to_string()
    }

    fn reveal(t: &T, path: impl AsRef<std::path::Path>) -> Result<(), crate::error::CoreError> {
        t.core
            .dispatch(
                "reveal_in_explorer",
                json!({ "path": path.as_ref().to_string_lossy() }),
            )
            .map(|_| ())
    }

    #[test]
    fn fr_prj_59_a_folder_opens_itself_and_a_file_is_selected() {
        let t = setup();
        let project = t.lib.join("Maps").join("Crypt");
        std::fs::create_dir_all(project.join("references")).unwrap();
        let file = project.join("references").join("statue.png");
        std::fs::write(&file, b"png").unwrap();
        reveal(&t, &project).unwrap();
        reveal(&t, &file).unwrap();
        assert_eq!(
            t.shell.calls(),
            vec![
                vec![real(&project)],
                vec![format!("/select,{}", real(&file))]
            ]
        );
    }

    #[test]
    fn fr_prj_59_explorer_args_are_select_for_a_file() {
        let p = std::path::Path::new("lib").join("a b.png");
        assert_eq!(
            crate::context::explorer_args(&p, true),
            vec![format!("/select,{}", p.display())]
        );
        assert_eq!(
            crate::context::explorer_args(&p, false),
            vec![p.display().to_string()]
        );
    }

    #[test]
    fn fr_prj_59_outside_the_library_and_dot_dot_escapes_are_refused() {
        let t = setup();
        let direct = reveal(&t, t.outside.join("secret.png")).unwrap_err();
        assert_eq!(direct.code, ErrorCode::OutsideLibrary);
        // `<lib>/Maps/../../elsewhere/secret.png` resolves outside the root.
        let escape = t
            .lib
            .join("Maps")
            .join("..")
            .join("..")
            .join("elsewhere")
            .join("secret.png");
        assert_eq!(
            reveal(&t, &escape).unwrap_err().code,
            ErrorCode::OutsideLibrary
        );
        assert!(t.shell.calls().is_empty());
    }

    #[test]
    fn fr_prj_59_a_link_out_of_the_root_is_outside_the_library() {
        let t = setup();
        let link = t.lib.join("Maps").join("sneaky");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&t.outside, &link);
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&t.outside, &link);
        if let Err(e) = made {
            // Windows without the symlink privilege (developer mode off): nothing to test here.
            eprintln!("skipped: can't create a symlink here: {e}");
            return;
        }
        assert_eq!(
            reveal(&t, link.join("secret.png")).unwrap_err().code,
            ErrorCode::OutsideLibrary
        );
        assert!(t.shell.calls().is_empty());
    }

    #[test]
    fn fr_prj_59_missing_path_is_not_found() {
        let t = setup();
        let err = reveal(&t, t.lib.join("Maps").join("Gone")).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(t.shell.calls().is_empty());
    }

    /// A shell whose launch fails (no Explorer) or whose Explorer exits non-zero.
    struct Failing(bool);

    impl Shell for Failing {
        fn reveal(&self, _: &std::path::Path, _: bool) -> std::io::Result<()> {
            if self.0 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "explorer.exe not found",
                ))
            } else {
                // Explorer often exits with 1 even when it opened: the seam doesn't wait.
                Ok(())
            }
        }
    }

    #[test]
    fn fr_prj_59_only_a_failed_spawn_is_an_error() {
        for (spawn_fails, want) in [(true, Some(ErrorCode::Io)), (false, None)] {
            let dir = tempfile::tempdir().unwrap();
            let mut ctx =
                Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
            let lib = dir.path().join("lib");
            ctx.library_override = Some(lib.clone());
            ctx.shell = Arc::new(Failing(spawn_fails));
            let core = Core::new(ctx).unwrap();
            let got = core
                .dispatch(
                    "reveal_in_explorer",
                    json!({ "path": lib.join("Maps").to_string_lossy() }),
                )
                .err()
                .map(|e| e.code);
            assert_eq!(got, want);
        }
    }
}
