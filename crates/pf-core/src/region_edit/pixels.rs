//! What RGN reads from the open asset project: its size, the current layer stack over PRJ's
//! decoded original (FR-RGN-04, R11, R45) and stored masks (FR-EDT-28, PRJ `put_mask`).
//!
//! RGN codes against [`PixelSource`]. [`OpenProjectSource`] reads PRJ's `decoded_original`
//! handle and the manifest's mask records; [`MemorySource`] is the test double.

use std::path::PathBuf;
use std::sync::Arc;

use image::{RgbImage, RgbaImage};
use pf_imaging::{Layer as ImgLayer, LayerStack, Placement};
use ulid::Ulid;

use super::fit::RectPx;
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::manifest::{Layer, LayerKind};
use crate::project::{OpenProject, store};

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

/// Reads a patch layer's PNG as RGBA8.
pub fn decode_layer(path: &std::path::Path, id: Ulid) -> CoreResult<Arc<RgbaImage>> {
    let img = image::open(path)
        .map_err(|e| CoreError::new(ErrorCode::Corrupt, format!("decoding layer {id}: {e}")))?
        .to_rgba8();
    Ok(Arc::new(img))
}

/// [`PixelSource`] over the open project: PRJ's decoded original (R45) and mask store.
#[derive(Debug)]
pub struct OpenProjectSource {
    project_id: Ulid,
    size: (u32, u32),
    read_only: bool,
    /// `decoded_original` as it was when the source was taken (starts the decode, R11).
    original: CoreResult<Arc<RgbImage>>,
    project_dir: PathBuf,
    layers: Vec<Layer>,
    /// `(id, inner rect, project path)` of every stored mask.
    masks: Vec<(Ulid, RectPx, String)>,
}

impl OpenProjectSource {
    pub fn new(ctx: &Context, p: &OpenProject) -> Self {
        let rect = |r: &crate::manifest::Rect| RectPx {
            x: r.x.max(0) as u32,
            y: r.y.max(0) as u32,
            w: r.w,
            h: r.h,
        };
        OpenProjectSource {
            project_id: p.manifest.project_id,
            size: (p.manifest.original.width, p.manifest.original.height),
            read_only: p.read_only.is_some(),
            original: store::decoded_original(ctx, p),
            project_dir: p.dir.clone(),
            layers: p.manifest.layers.clone(),
            masks: p
                .manifest
                .masks
                .iter()
                .map(|m| (m.id, rect(&m.rect), m.file.clone()))
                .collect(),
        }
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
        let original = self.original.clone()?;
        if original.dimensions() != self.size {
            return Err(CoreError::new(
                ErrorCode::Corrupt,
                "the original's size changed since the project was opened",
            ));
        }
        let mut patches = Vec::new();
        for l in self.layers.iter().filter(|l| l.kind == LayerKind::Patch) {
            let Some(file) = &l.file else { continue };
            patches.push((l.clone(), decode_layer(&self.project_dir.join(file), l.id)?));
        }
        stack_from_layers(original, &patches)
    }

    fn mask(&self, id: Ulid) -> CoreResult<MaskRecord> {
        let (_, rect, file) = self
            .masks
            .iter()
            .find(|m| m.0 == id)
            .ok_or_else(|| CoreError::new(ErrorCode::UnknownId, format!("no mask {id}")))?;
        let png = std::fs::read(self.project_dir.join(file))
            .map_err(|e| CoreError::io(format!("reading mask {id}"), &e))?;
        Ok(MaskRecord {
            id,
            rect: *rect,
            png,
        })
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
