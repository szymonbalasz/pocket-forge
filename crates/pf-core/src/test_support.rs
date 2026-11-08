//! Shared helpers for PRJ unit tests: a deterministic context with a library override, a fake
//! filesystem-attributes handle, generated images (KQ-22, no personal art).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub use crate::context::FsAttributes;
use crate::context::{Context, FakeFsAttributes, RecordedEvents};
use crate::create::{CreateItem, create_projects};
use crate::manifest::Kind;
use crate::project::{self, OpenProject};
use crate::settings::Settings;
use crate::time::Timestamp;

/// Start of the fake clock: 2027-01-15T08:00:00Z.
pub const T0: i64 = 1_800_000_000_000;

pub fn png_bytes(w: u32, h: u32, shade: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([shade, (x % 251) as u8, (y % 241) as u8])
    })
    .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
    .unwrap();
    out
}

pub fn rgba_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba(rgba))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

pub fn jpeg_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::from_pixel(w, h, image::Rgb([90, 60, 30]))
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    out
}

pub struct Env {
    pub dir: tempfile::TempDir,
    pub ctx: Context,
    pub settings: Settings,
    pub fs: Arc<FakeFsAttributes>,
    pub events: Arc<RecordedEvents>,
}

pub fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(T0));
    ctx.library_override = Some(dir.path().join("lib"));
    let fs = Arc::new(FakeFsAttributes::default());
    ctx.fs = Box::new(fs.clone());
    let events = Arc::new(RecordedEvents::default());
    ctx.events = events.clone();
    crate::library::prepare_override(&ctx).unwrap();
    std::fs::create_dir_all(dir.path().join("outside")).unwrap();
    std::fs::create_dir_all(&ctx.data_dir).unwrap();
    let settings = Settings::defaults(&ctx);
    Env {
        dir,
        ctx,
        settings,
        fs,
        events,
    }
}

impl Env {
    pub fn lib(&self) -> PathBuf {
        self.dir.path().join("lib")
    }

    /// Writes a file outside the library.
    pub fn outside(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = self.dir.path().join("outside").join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }

    pub fn create(&self, kind: Kind, paths: &[PathBuf]) -> Vec<CreateItem> {
        create_projects(
            &self.ctx,
            &self.settings,
            kind,
            paths,
            &AtomicBool::new(false),
            None,
        )
        .unwrap()
    }

    /// Creates a Maps project from a generated 40×30 PNG named `<name>.png`; returns its folder.
    pub fn project(&self, name: &str) -> PathBuf {
        // A shade per name, so two projects never share a hash (FR-PRJ-08 dedupe).
        let shade = name
            .bytes()
            .fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b));
        let src = self.outside(&format!("{name}.png"), &png_bytes(40, 30, shade));
        match &self.create(Kind::Map, &[src])[0] {
            CreateItem::Created { path, .. } => path.clone(),
            other => panic!("{other:?}"),
        }
    }

    pub fn open(&self, dir: &Path) -> OpenProject {
        project::open(&self.ctx, &self.settings, dir, None).unwrap()
    }

    pub fn install(&self) -> ulid::Ulid {
        self.settings.install_id
    }
}

/// Every file under `dir`, relative, `/` separators, sorted.
pub fn tree(dir: &Path) -> Vec<String> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.path().is_dir() {
            let sub = tree(&e.path());
            if sub.is_empty() {
                out.push(format!("{name}/"));
            }
            out.extend(sub.into_iter().map(|c| format!("{name}/{c}")));
        } else {
            out.push(name);
        }
    }
    out.sort();
    out
}
