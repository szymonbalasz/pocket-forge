//! What RGN reads from the open asset project: its size, the current layer stack over PRJ's
//! decoded original (FR-RGN-04, R11) and stored masks (FR-EDT-28).
//!
//! RGN codes against [`PixelSource`]. [`OpenProjectSource`] is the **interim** adapter over the
//! skeleton's `OpenProject`: it decodes the original itself on first use and has no mask store.
//! It is replaced by PRJ's `DecodedOriginal` and store API (`read_project`, masks) when those
//! land on `main`; nothing else in RGN changes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use image::{RgbImage, RgbaImage};
use pf_imaging::{Layer as ImgLayer, LayerStack, Placement};
use ulid::Ulid;

use super::fit::RectPx;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::manifest::{Layer, LayerKind};
use crate::project::OpenProject;

/// A stored mask: 8-bit grey, 255 = editable, at its region (inner) rect (schema §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskRecord {
    pub id: Ulid,
    /// The region the mask was painted for (inner rect, image px).
    pub rect: RectPx,
    /// The stored PNG as written by `put_mask`.
    pub png: Vec<u8>,
}

/// The project data RGN's commands need. Owned and `Send`, so a command can take it from the
/// core state and release the lock before doing pixel or provider work.
pub trait PixelSource: Send + Sync {
    fn project_id(&self) -> Ulid;
    /// The original's size (= the flatten's size).
    fn image_size(&self) -> (u32, u32);
    /// FR-RGN-38: a read-only project requests no estimates and submits nothing.
    fn read_only(&self) -> bool;
    /// The current layer stack over the decoded original. `original_unavailable` until PRJ's
    /// decoded original is ready (R11).
    fn layer_stack(&self) -> CoreResult<LayerStack>;
    /// A stored mask by id (`unknown_id` if there is none).
    fn mask(&self, id: Ulid) -> CoreResult<MaskRecord>;
}

pub fn original_unavailable() -> CoreError {
    CoreError::new(
        ErrorCode::OriginalUnavailable,
        "the original image is still loading; try again in a moment",
    )
}

/// Builds the pf-imaging stack from manifest layers (`[0]` is the original) and decoded patches.
pub fn stack_from_layers(
    original: Arc<RgbImage>,
    layers: &[(Layer, Arc<RgbaImage>)],
) -> CoreResult<LayerStack> {
    let img_layers: Vec<ImgLayer> = layers
        .iter()
        .map(|(l, image)| ImgLayer {
            image: image.clone(),
            placement: Placement {
                cx: l.cx,
                cy: l.cy,
                scale: l.scale,
                rotation: l.rotation,
            },
            opacity: l.opacity as f32,
            visible: l.visible,
        })
        .collect();
    LayerStack::new(original, &img_layers)
        .map_err(|e| CoreError::internal(format!("preparing the layer stack: {e}")))
}

/// The interim decoded-original cache: one project's original, kept until another project's
/// original is decoded. PRJ's `DecodedOriginal` replaces it (R11).
pub type OriginalCache = Arc<Mutex<Option<(Ulid, Arc<RgbImage>)>>>;

/// **Interim** [`PixelSource`] over the skeleton `OpenProject` (see the module docs).
#[derive(Debug)]
pub struct OpenProjectSource {
    project_id: Ulid,
    size: (u32, u32),
    read_only: bool,
    image_path: PathBuf,
    project_dir: PathBuf,
    layers: Vec<Layer>,
    cache: OriginalCache,
}

impl OpenProjectSource {
    pub fn new(p: &OpenProject, cache: OriginalCache) -> Self {
        OpenProjectSource {
            project_id: p.manifest.project_id,
            size: (p.manifest.original.width, p.manifest.original.height),
            read_only: p.read_only.is_some(),
            image_path: p.original_path(),
            project_dir: p.dir.clone(),
            layers: p.manifest.layers.clone(),
            cache,
        }
    }

    fn original(&self) -> CoreResult<Arc<RgbImage>> {
        let lock = || self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((id, img)) = lock().as_ref()
            && *id == self.project_id
        {
            return Ok(img.clone());
        }
        let img = image::open(&self.image_path)
            .map_err(|e| CoreError::new(ErrorCode::Corrupt, format!("decoding the original: {e}")))?
            .to_rgb8();
        if img.dimensions() != self.size {
            return Err(CoreError::new(
                ErrorCode::Corrupt,
                "the original's size changed since the project was opened",
            ));
        }
        let img = Arc::new(img);
        *lock() = Some((self.project_id, img.clone()));
        Ok(img)
    }
}

impl PixelSource for OpenProjectSource {
    fn project_id(&self) -> Ulid {
        self.project_id
    }

    fn image_size(&self) -> (u32, u32) {
        self.size
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn layer_stack(&self) -> CoreResult<LayerStack> {
        let original = self.original()?;
        let mut patches = Vec::new();
        for l in self.layers.iter().filter(|l| l.kind == LayerKind::Patch) {
            let Some(file) = &l.file else { continue };
            let path = self.project_dir.join(file);
            let img = image::open(&path)
                .map_err(|e| {
                    CoreError::new(ErrorCode::Corrupt, format!("decoding layer {}: {e}", l.id))
                })?
                .to_rgba8();
            patches.push((l.clone(), Arc::new(img)));
        }
        stack_from_layers(original, &patches)
    }

    fn mask(&self, id: Ulid) -> CoreResult<MaskRecord> {
        // The mask store (PRJ `put_mask`, EDT slice 5) isn't on main yet.
        Err(CoreError::new(
            ErrorCode::UnknownId,
            format!("no stored mask {id} (the mask store isn't built yet)"),
        ))
    }
}

/// An in-memory source for tests and fixtures.
#[derive(Debug, Clone)]
pub struct MemorySource {
    pub project_id: Ulid,
    pub size: (u32, u32),
    pub original: Option<Arc<RgbImage>>,
    pub patches: Vec<(Layer, Arc<RgbaImage>)>,
    pub masks: Vec<MaskRecord>,
    pub read_only: bool,
}

impl MemorySource {
    pub fn new(project_id: Ulid, original: RgbImage) -> Self {
        MemorySource {
            project_id,
            size: original.dimensions(),
            original: Some(Arc::new(original)),
            patches: vec![],
            masks: vec![],
            read_only: false,
        }
    }
}

impl PixelSource for MemorySource {
    fn project_id(&self) -> Ulid {
        self.project_id
    }

    fn image_size(&self) -> (u32, u32) {
        self.size
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn layer_stack(&self) -> CoreResult<LayerStack> {
        let original = self.original.clone().ok_or_else(original_unavailable)?;
        stack_from_layers(original, &self.patches)
    }

    fn mask(&self, id: Ulid) -> CoreResult<MaskRecord> {
        self.masks
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| CoreError::new(ErrorCode::UnknownId, format!("no mask {id}")))
    }
}
