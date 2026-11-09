//! `DecodedOriginal` (FR-PRJ-42, R11): `layers[0]`'s file as RGB8 (ADR 0004), owned by PRJ: the
//! original, or a picked base image (FR-GEN-33), its alpha over the flatten colour (FR-GEN-12).
//! Decoded lazily on the first RGN or EXP use or by `project_prefetch_original`, in the
//! background, and released when the project closes. Until it's ready, callers get
//! `original_unavailable`; a project with no image gets `no_image` (FR-GEN-03).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::RgbImage;
use serde_json::json;
use ulid::Ulid;

use crate::context::{Context, EventSink};
use crate::error::{CoreError, CoreResult, ErrorCode};

#[derive(Debug, Clone)]
enum State {
    Idle,
    Decoding,
    Ready(Arc<RgbImage>),
    Failed(CoreError),
}

/// The handle an open project holds. Cloning shares the same decode.
#[derive(Debug, Clone)]
pub struct DecodedOriginal {
    project_id: Ulid,
    /// `None` in a project with no image (FR-GEN-03).
    path: Option<PathBuf>,
    state: Arc<Mutex<State>>,
}

/// `no_image` (FR-GEN-03): region edit and export need an image.
pub fn no_image() -> CoreError {
    CoreError::new(
        ErrorCode::NoImage,
        "No image yet. Generate one in New image.",
    )
}

impl DecodedOriginal {
    pub fn new(project_id: Ulid, path: PathBuf) -> Self {
        DecodedOriginal {
            project_id,
            path: Some(path),
            state: Arc::new(Mutex::new(State::Idle)),
        }
    }

    /// The handle of a project with no image: every use fails with `no_image`.
    pub fn none(project_id: Ulid) -> Self {
        DecodedOriginal {
            project_id,
            path: None,
            state: Arc::new(Mutex::new(State::Idle)),
        }
    }

    /// The decoded file, if the project has an image.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A handle whose decode never finishes (EXP's `original_unavailable` test, FR-EXP-28).
    #[cfg(test)]
    pub(crate) fn decoding_for_tests(project_id: Ulid) -> Self {
        let d = DecodedOriginal::new(project_id, PathBuf::new());
        *d.lock() = State::Decoding;
        d
    }

    /// The project this original belongs to.
    pub fn project_id(&self) -> Ulid {
        self.project_id
    }

    /// True once the pixels are in memory.
    pub fn is_ready(&self) -> bool {
        matches!(*self.lock(), State::Ready(_))
    }

    /// The pixels, or `original_unavailable` while the decode runs (starting it if needed), or
    /// the decode's error (`corrupt`, `io`, …). A failed decode isn't run again until
    /// [`release`](Self::release) (project close).
    pub fn get(&self, ctx: &Context) -> CoreResult<Arc<RgbImage>> {
        if self.path.is_none() {
            return Err(no_image());
        }
        self.start(ctx);
        match &*self.lock() {
            State::Ready(img) => Ok(img.clone()),
            State::Failed(e) => Err(e.clone()),
            State::Idle | State::Decoding => Err(CoreError::new(
                ErrorCode::OriginalUnavailable,
                "the original is still being decoded",
            )),
        }
    }

    /// Starts the decode unless it's running, done or failed (`project_prefetch_original`).
    /// A failed decode keeps its error until [`release`](Self::release); re-running it on every
    /// use would hide the error and redo a full 16k decode each time (review #10). Emits
    /// `project:original-ready` when the pixels are in memory.
    pub fn start(&self, ctx: &Context) {
        if self.path.is_none() {
            return;
        }
        {
            let mut s = self.lock();
            if !matches!(*s, State::Idle) {
                return;
            }
            *s = State::Decoding;
        }
        let job = self.clone();
        let events = ctx.events.clone();
        if ctx.inline_background {
            job.run(&*events);
        } else {
            let spawned = std::thread::Builder::new()
                .name("pf-decode-original".into())
                .spawn(move || job.run(&*events));
            if let Err(e) = spawned {
                *self.lock() =
                    State::Failed(CoreError::internal(format!("can't start the decode: {e}")));
            }
        }
    }

    fn run(&self, events: &dyn EventSink) {
        let Some(path) = &self.path else { return };
        let result = pf_imaging::decode_rgb8(path);
        let mut s = self.lock();
        // A close drops the handle; a late result for a released handle is simply discarded.
        match result {
            Ok(img) => {
                *s = State::Ready(Arc::new(img));
                drop(s);
                events.emit(
                    "project:original-ready",
                    json!({ "project_id": self.project_id }),
                );
            }
            Err(e) => {
                let err = match e {
                    pf_imaging::ProbeError::Io(io)
                        if io.kind() != std::io::ErrorKind::UnexpectedEof =>
                    {
                        CoreError::io("reading the original", &io)
                    }
                    other => CoreError::new(
                        ErrorCode::Corrupt,
                        format!("the original can't be decoded: {other}"),
                    ),
                };
                tracing::warn!(error = %err, "decoding the original failed");
                let code = err.code;
                *s = State::Failed(err);
                drop(s);
                // R104: callers waiting for `project:original-ready` learn it won't come.
                events.emit(
                    "project:original-failed",
                    json!({ "project_id": self.project_id, "code": code }),
                );
            }
        }
    }

    /// Drops the pixels (project close). The next use decodes again.
    pub fn release(&self) {
        *self.lock() = State::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RecordedEvents;
    use crate::time::Timestamp;

    #[test]
    fn fr_prj_42_decodes_once_emits_ready_and_releases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.png");
        image::RgbImage::from_pixel(8, 4, image::Rgb([1, 2, 3]))
            .save(&path)
            .unwrap();
        let mut ctx = Context::deterministic(dir.path().into(), Timestamp(0));
        let events = Arc::new(RecordedEvents::default());
        ctx.events = events.clone();
        let d = DecodedOriginal::new(Ulid::from_parts(0, 1), path);
        assert!(!d.is_ready());
        let img = d.get(&ctx).unwrap();
        assert_eq!(
            (img.width(), img.height(), img.get_pixel(0, 0).0),
            (8, 4, [1, 2, 3])
        );
        assert_eq!(events.names(), ["project:original-ready"]);
        d.get(&ctx).unwrap();
        assert_eq!(events.names().len(), 1, "decoded once");
        d.release();
        assert!(!d.is_ready());
    }

    #[test]
    fn fr_prj_42_unavailable_until_the_background_decode_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.png");
        image::RgbImage::new(4, 4).save(&path).unwrap();
        let mut ctx = Context::deterministic(dir.path().into(), Timestamp(0));
        ctx.inline_background = false;
        let d = DecodedOriginal::new(Ulid::from_parts(0, 1), path);
        // Marked as decoding by hand: callers get `original_unavailable`, never a partial image.
        *d.lock() = State::Decoding;
        assert_eq!(
            d.get(&ctx).unwrap_err().code,
            ErrorCode::OriginalUnavailable
        );
        *d.lock() = State::Idle;
        let r = d.get(&ctx);
        assert!(
            matches!(&r, Err(e) if e.code == ErrorCode::OriginalUnavailable) || r.is_ok(),
            "{r:?}"
        );
        for _ in 0..500 {
            if d.is_ready() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("decode never finished");
    }

    #[test]
    fn fr_gen_03_no_image_handle_fails_with_no_image() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::deterministic(dir.path().into(), Timestamp(0));
        let d = DecodedOriginal::none(Ulid::from_parts(0, 1));
        assert_eq!(d.get(&ctx).unwrap_err().code, ErrorCode::NoImage);
        assert!(!d.is_ready() && d.path().is_none());
    }

    /// FR-GEN-12, FR-GEN-33: a base image with alpha decodes over the flatten colour.
    #[test]
    fn fr_gen_12_decoded_layer_0_flattens_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("001 token 01.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 0]))
            .save(&path)
            .unwrap();
        let ctx = Context::deterministic(dir.path().into(), Timestamp(0));
        let d = DecodedOriginal::new(Ulid::from_parts(0, 1), path);
        assert_eq!(
            d.get(&ctx).unwrap().get_pixel(2, 2).0,
            pf_imaging::FLATTEN_BG
        );
    }

    /// Review #10, R104: on the production background thread, a failed decode emits
    /// `project:original-failed` and stays `Failed`: the next `get()` reports `corrupt` and
    /// doesn't start the decode again.
    #[test]
    fn fr_prj_42_background_decode_failure_is_reported_and_not_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.png");
        std::fs::write(&path, truncated_png()).unwrap();
        let mut ctx = Context::deterministic(dir.path().into(), Timestamp(0));
        ctx.inline_background = false;
        let events = Arc::new(RecordedEvents::default());
        ctx.events = events.clone();
        let d = DecodedOriginal::new(Ulid::from_parts(0, 1), path);
        let first = d.get(&ctx);
        assert!(first.is_err(), "{first:?}");
        let mut settled = false;
        for _ in 0..500 {
            // The event follows the state change: wait for it.
            if !events.names().is_empty() {
                settled = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(settled, "the background decode never failed");
        for _ in 0..3 {
            assert_eq!(d.get(&ctx).unwrap_err().code, ErrorCode::Corrupt);
            assert!(
                matches!(*d.lock(), State::Failed(_)),
                "get() must not start the decode again"
            );
        }
        // R104: one `project:original-failed {project_id, code}`, not re-sent by later gets.
        assert_eq!(events.names(), ["project:original-failed"]);
        assert_eq!(
            events.0.lock().unwrap()[0].1,
            json!({ "project_id": Ulid::from_parts(0, 1), "code": "corrupt" })
        );
        // A close releases the handle; the next use decodes again.
        d.release();
        assert!(matches!(*d.lock(), State::Idle));
    }

    fn truncated_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbImage::new(64, 64)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes.truncate(bytes.len() / 2);
        bytes
    }

    #[test]
    fn fr_prj_42_corrupt_original_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.png");
        let mut bytes = Vec::new();
        image::RgbImage::new(64, 64)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes.truncate(bytes.len() / 2);
        std::fs::write(&path, bytes).unwrap();
        let ctx = Context::deterministic(dir.path().into(), Timestamp(0));
        let d = DecodedOriginal::new(Ulid::from_parts(0, 1), path);
        assert_eq!(d.get(&ctx).unwrap_err().code, ErrorCode::Corrupt);
    }
}
