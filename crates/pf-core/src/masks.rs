//! EDT mask hand-off (FR-EDT-18, 28, 29; R1, R7): the UI's inner-rect mask → PRJ `put_mask`, and
//! back. Bytes are 8-bit, w·h, row-major, 255 = editable (ADR 0001). Over IPC they travel as
//! base64 (`data_b64`): a JSON number array would be ~4× larger at 4096².
//!
//! [`read_mask`] is the store-side read other features (RGN's `PixelSource::mask`) can share.

use serde::Serialize;
use ulid::Ulid;

use crate::core::Core;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::manifest::Rect;
use crate::project::OpenProject;

/// FR-EDT-14 / OQ-CNV-04: a mask is at most 4096 px per side.
pub const MASK_MAX_SIDE: u32 = 4096;

/// `edt_mask_submit` result (`MaskRef`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MaskRef {
    pub mask_id: Ulid,
    pub rect: Rect,
}

/// A stored mask: its rect (the region's inner rect) and its w·h bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMask {
    pub rect: Rect,
    pub bytes: Vec<u8>,
}

fn invalid_rect(msg: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidRect, msg).with_field("rect")
}

/// The size rules that don't need the image: 1 ≤ w, h ≤ 4096 (`invalid_rect`).
pub fn check_rect_size(rect: &Rect) -> CoreResult<()> {
    if rect.w == 0 || rect.h == 0 {
        return Err(invalid_rect("the mask rect is empty"));
    }
    if rect.w > MASK_MAX_SIDE || rect.h > MASK_MAX_SIDE {
        return Err(invalid_rect("a mask is at most 4096 px per side"));
    }
    Ok(())
}

fn open_project(
    project: Option<&mut OpenProject>,
    project_id: Ulid,
) -> CoreResult<&mut OpenProject> {
    let p =
        project.ok_or_else(|| CoreError::new(ErrorCode::NoProject, "no asset project is open"))?;
    if p.manifest.project_id != project_id {
        return Err(
            CoreError::new(ErrorCode::UnknownId, "that project isn't open")
                .with_field("project_id"),
        );
    }
    Ok(p)
}

/// `edt_mask_submit`: validates, then PRJ `put_mask` writes the PNG and points the draft at it in
/// one commit (R7). A read-only project fails fast with `read_only` before the body is decoded
/// (FR-EDT-29). `locked` / `conflict` / `io` come from the store; the UI keeps the mask and retries,
/// and PRJ keeps a `locked` one waiting so closing asks first (R108).
pub fn submit(core: &Core, project_id: Ulid, rect: Rect, data_b64: &str) -> CoreResult<MaskRef> {
    check_rect_size(&rect)?;
    {
        let mut state = core.state();
        open_project(state.project.as_mut(), project_id)?.require_writable()?;
    }
    // Decoded outside the state lock (16 MiB at 4096²).
    let bytes = crate::b64::decode(data_b64)
        .ok_or_else(|| CoreError::invalid("data_b64", "the mask body is not base64"))?;
    let expected = rect.w as usize * rect.h as usize;
    if bytes.len() != expected {
        return Err(CoreError::new(
            ErrorCode::SizeMismatch,
            format!(
                "a {}×{} mask is {expected} bytes, not {}",
                rect.w,
                rect.h,
                bytes.len()
            ),
        )
        .with_field("data_b64"));
    }
    let mut state = core.state();
    let install_id = state.settings.install_id;
    let p = open_project(state.project.as_mut(), project_id)?;
    let mask = crate::project::submit_mask(&core.ctx, install_id, p, rect, bytes)?;
    Ok(MaskRef {
        mask_id: mask.id,
        rect: mask.rect,
    })
}

/// Reads a stored mask by id: `unknown_id` if the manifest has no such mask, `not_found` if its
/// file is gone, `corrupt` if it isn't an 8-bit grey PNG of exactly the rect's size.
pub fn read_mask(p: &OpenProject, mask_id: Ulid) -> CoreResult<StoredMask> {
    let record = p
        .manifest
        .masks
        .iter()
        .find(|m| m.id == mask_id)
        .ok_or_else(|| {
            CoreError::new(ErrorCode::UnknownId, "unknown mask").with_field("mask_id")
        })?;
    let path = p.path(&record.file);
    let file = std::fs::read(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            CoreError::new(ErrorCode::NotFound, "the mask file is missing")
        }
        _ => CoreError::io("reading the mask", &e),
    })?;
    let corrupt = |why: String| CoreError::new(ErrorCode::Corrupt, why).with_field("mask_id");
    let img = image::load_from_memory_with_format(&file, image::ImageFormat::Png)
        .map_err(|e| corrupt(format!("the mask can't be decoded: {e}")))?;
    let image::DynamicImage::ImageLuma8(grey) = img else {
        return Err(corrupt("the mask is not 8-bit grey".into()));
    };
    let rect = record.rect;
    if grey.dimensions() != (rect.w, rect.h) {
        return Err(corrupt(format!(
            "the mask is {}×{}, its rect {}×{}",
            grey.width(),
            grey.height(),
            rect.w,
            rect.h
        )));
    }
    Ok(StoredMask {
        rect,
        bytes: grey.into_raw(),
    })
}

/// `edt_mask_read` result: the rect and the w·h bytes as base64.
#[derive(Debug, Clone, Serialize)]
pub struct MaskReadOutput {
    pub mask_id: Ulid,
    pub rect: Rect,
    pub data_b64: String,
}

/// `edt_mask_read`: byte-identical to what was submitted (FR-EDT-18). Works read-only.
pub fn read(core: &Core, project_id: Ulid, mask_id: Ulid) -> CoreResult<MaskReadOutput> {
    let stored = {
        let mut state = core.state();
        let p = open_project(state.project.as_mut(), project_id)?;
        read_mask(p, mask_id)?
    };
    Ok(MaskReadOutput {
        mask_id,
        rect: stored.rect,
        data_b64: crate::b64::encode(&stored.bytes),
    })
}

#[cfg(test)]
mod tests;
