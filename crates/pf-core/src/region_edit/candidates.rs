//! RGN slices 4–5 commands (spec §9): candidate preview, accept (and re-pick), star, discard,
//! re-tune and the iteration tracker's list. Storage, statuses and the trash are PRJ's store
//! calls (R4); RGN bakes the patch ([`super::blend`]).

use std::sync::Arc;

use image::RgbImage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use super::blend::{self, Baked, BlendInput, MAX_FEATHER};
use super::fit::{RectPx, RegionFit};
use super::jobs::{is_active, kept};
use super::pixels::PixelSource;
use super::runner::{emit_state, shown_cost};
use super::{pixel_source, prepare};
use crate::commands::{NoInput, Ok_};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::job::{JobFile, JobState};
use crate::manifest::{CandidateStatus, LayerKind, Rect};
use crate::project::store::{self, LayerCommit, NewPatch};
use crate::tiles;

fn no_project() -> CoreError {
    CoreError::new(ErrorCode::NoProject, "no asset project is open")
}

fn invalid_candidate(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidCandidate, message).with_field("candidate_id")
}

fn check_blend(b: &BlendInput) -> CoreResult<()> {
    if b.feather_px > MAX_FEATHER {
        return Err(CoreError::invalid(
            "blend.feather_px",
            format!("the feather is 0 to {MAX_FEATHER} px"),
        ));
    }
    Ok(())
}

fn rect_of(v: &Value) -> CoreResult<RectPx> {
    serde_json::from_value(v.clone())
        .map_err(|_| CoreError::new(ErrorCode::Corrupt, "the job's region is unreadable"))
}

/// The job's region at the outer rect's own resolution (no fit scale): the bake works there.
fn outer_fit(job: &JobFile) -> CoreResult<RegionFit> {
    let inner = rect_of(&job.region["inner"])?;
    let outer = rect_of(&job.region["outer"])?;
    Ok(RegionFit {
        inner,
        outer,
        margin: job.region["margin"].as_u64().unwrap_or(0) as u32,
        sent_w: outer.w,
        sent_h: outer.h,
        scale: 1.0,
    })
}

fn imaging(e: impl std::fmt::Display) -> CoreError {
    CoreError::internal(format!("image processing: {e}"))
}

/// Everything a bake needs, taken under the lock; the pixel work runs without it.
struct BakeJob {
    project_id: Ulid,
    job: JobFile,
    candidate_path: std::path::PathBuf,
    src: Arc<dyn PixelSource>,
}

fn bake_job(core: &Core, job_id: Ulid, candidate_id: Ulid) -> CoreResult<BakeJob> {
    let state = core.state();
    let p = state.project.as_ref().ok_or_else(no_project)?;
    let job = store::job(p, job_id)?.clone();
    let cand = p
        .manifest
        .candidates
        .iter()
        .find(|c| c.id == candidate_id)
        .ok_or_else(|| {
            CoreError::new(ErrorCode::UnknownId, "unknown candidate").with_field("candidate_id")
        })?;
    if cand.job_id != job_id {
        return Err(invalid_candidate("the candidate belongs to another job"));
    }
    if cand.status == CandidateStatus::Invalid {
        return Err(invalid_candidate("the candidate won't decode"));
    }
    let candidate_path = p.resolve_image(candidate_id)?.path;
    let src = pixel_source(&core.ctx, &state)?;
    Ok(BakeJob {
        project_id: p.manifest.project_id,
        job,
        candidate_path,
        src,
    })
}

/// FR-RGN-25/26: the candidate resized to the outer rect (#40, also an odd aspect), the current
/// stack's pixels there, the canonical mask padded to it, then [`blend::bake`].
fn bake(b: &BakeJob, blend: BlendInput) -> CoreResult<Baked> {
    let fit = outer_fit(&b.job)?;
    let bytes = std::fs::read(&b.candidate_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CoreError::new(
                ErrorCode::SourceMissing,
                "the candidate's file is gone (purged from the trash?)",
            )
        } else {
            CoreError::io("reading the candidate", &e)
        }
    })?;
    let cand = image::load_from_memory(&bytes)
        .map_err(|e| invalid_candidate(format!("the candidate won't decode: {e}")))?
        .to_rgb8();
    let (ow, oh) = (fit.outer.w, fit.outer.h);
    let cand: RgbImage = if cand.dimensions() == (ow, oh) {
        cand
    } else {
        pf_imaging::resize_lanczos3(&cand, ow, oh).map_err(imaging)?
    };
    let base = prepare::sent_region(&*b.src, &fit)?;
    let mask_id = b
        .job
        .mask_id
        .ok_or_else(|| CoreError::new(ErrorCode::SourceMissing, "the job has no mask"))?;
    let mask = b.src.mask(mask_id).map_err(|e| {
        if e.code == ErrorCode::UnknownId {
            CoreError::new(ErrorCode::SourceMissing, "the job's mask is gone")
        } else {
            e
        }
    })?;
    let mask = prepare::sent_mask(&mask, &fit)?;
    Ok(blend::bake(&base, &cand, &mask, fit.outer, blend))
}

fn encode_rgba(img: &image::RgbaImage) -> CoreResult<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| CoreError::internal(format!("encoding the patch: {e}")))?;
    Ok(out)
}

fn manifest_rect(r: RectPx) -> Rect {
    Rect {
        x: i64::from(r.x),
        y: i64::from(r.y),
        w: r.w,
        h: r.h,
    }
}

// ---------------------------------------------------------------- rgn_candidate_preview

/// `rgn_candidate_preview` (FR-RGN-25, R16, R44): the baked preview under a transient key.
pub struct RgnCandidatePreview;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateBlendInput {
    pub job_id: Ulid,
    pub candidate_id: Ulid,
    #[serde(default)]
    pub blend: BlendInput,
}

impl Validate for CandidateBlendInput {
    fn validate(&self) -> CoreResult<()> {
        check_blend(&self.blend)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewOutput {
    pub key: String,
    pub url: String,
    /// Where the preview sits, image px.
    pub rect: RectPx,
    /// `colour_match_skipped` (FR-RGN-26).
    pub notices: Vec<String>,
}

impl Command for RgnCandidatePreview {
    const NAME: &'static str = "rgn_candidate_preview";
    type Input = CandidateBlendInput;
    type Output = PreviewOutput;

    fn run(core: &Core, input: CandidateBlendInput) -> CoreResult<PreviewOutput> {
        let b = bake_job(core, input.job_id, input.candidate_id)?;
        let baked = bake(&b, input.blend)?;
        let png = encode_rgba(&baked.rgba)?;
        let reg = tiles::register_transient_image(core, b.project_id, png)?;
        Ok(PreviewOutput {
            key: reg.key,
            url: reg.url,
            rect: baked.rect,
            notices: baked.notices,
        })
    }
}

// ---------------------------------------------------------------- rgn_candidate_accept

/// `rgn_candidate_accept` (FR-RGN-27/28/36): one PRJ commit with the baked patch layer, the
/// statuses, the trash and an `accept` revision. Also a re-pick (R32).
pub struct RgnCandidateAccept;

impl Command for RgnCandidateAccept {
    const NAME: &'static str = "rgn_candidate_accept";
    type Input = CandidateBlendInput;
    type Output = LayerCommit;

    fn run(core: &Core, input: CandidateBlendInput) -> CoreResult<LayerCommit> {
        {
            let state = core.state();
            let p = state.project.as_ref().ok_or_else(no_project)?;
            p.require_writable()?;
        }
        let b = bake_job(core, input.job_id, input.candidate_id)?;
        let baked = bake(&b, input.blend)?;
        let patch = NewPatch {
            png: encode_rgba(&baked.rgba)?,
            rect: Some(manifest_rect(baked.rect)),
            region: b.job.region.clone(),
            blend: baked.blend,
        };
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        if p.manifest.project_id != b.project_id {
            return Err(no_project());
        }
        let out = store::accept_candidate(
            &core.ctx,
            install_id,
            p,
            input.job_id,
            input.candidate_id,
            patch,
            None,
        )?;
        if let Ok(j) = store::job(p, input.job_id) {
            emit_state(&core.ctx, j, None);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- rgn_candidate_star / _discard

/// `rgn_candidate_star` (FR-RGN-28).
pub struct RgnCandidateStar;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StarInput {
    pub job_id: Ulid,
    pub candidate_id: Ulid,
    pub starred: bool,
}

impl Validate for StarInput {}

/// The candidate must exist and belong to the job.
fn check_owner(core: &Core, job_id: Ulid, candidate_id: Ulid) -> CoreResult<()> {
    let state = core.state();
    let p = state.project.as_ref().ok_or_else(no_project)?;
    let c = p
        .manifest
        .candidates
        .iter()
        .find(|c| c.id == candidate_id)
        .ok_or_else(|| {
            CoreError::new(ErrorCode::UnknownId, "unknown candidate").with_field("candidate_id")
        })?;
    if c.job_id != job_id {
        return Err(invalid_candidate("the candidate belongs to another job"));
    }
    Ok(())
}

impl Command for RgnCandidateStar {
    const NAME: &'static str = "rgn_candidate_star";
    type Input = StarInput;
    type Output = Ok_;

    fn run(core: &Core, input: StarInput) -> CoreResult<Ok_> {
        check_owner(core, input.job_id, input.candidate_id)?;
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        store::set_star(&core.ctx, install_id, p, input.candidate_id, input.starred)?;
        Ok(Ok_ {})
    }
}

/// `rgn_candidate_discard` (FR-RGN-28): `discarded`, its file to the trash.
pub struct RgnCandidateDiscard;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateInput {
    pub job_id: Ulid,
    pub candidate_id: Ulid,
}

impl Validate for CandidateInput {}

impl Command for RgnCandidateDiscard {
    const NAME: &'static str = "rgn_candidate_discard";
    type Input = CandidateInput;
    type Output = Ok_;

    fn run(core: &Core, input: CandidateInput) -> CoreResult<Ok_> {
        check_owner(core, input.job_id, input.candidate_id)?;
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        store::discard_candidate(&core.ctx, install_id, p, input.candidate_id)?;
        Ok(Ok_ {})
    }
}

// ---------------------------------------------------------------- rgn_patch_retune

/// `rgn_patch_retune` (FR-RGN-29, R3): re-bakes from the untouched candidate into a new layer
/// id and file, plus a `retune` revision.
pub struct RgnPatchRetune;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetuneInput {
    pub layer_id: Ulid,
    pub blend: BlendInput,
}

impl Validate for RetuneInput {
    fn validate(&self) -> CoreResult<()> {
        check_blend(&self.blend)
    }
}

impl Command for RgnPatchRetune {
    const NAME: &'static str = "rgn_patch_retune";
    type Input = RetuneInput;
    type Output = LayerCommit;

    fn run(core: &Core, input: RetuneInput) -> CoreResult<LayerCommit> {
        let (job_id, candidate_id) = {
            let state = core.state();
            let p = state.project.as_ref().ok_or_else(no_project)?;
            p.require_writable()?;
            let layer = p
                .manifest
                .layers
                .iter()
                .find(|l| l.id == input.layer_id && l.kind == LayerKind::Patch)
                .ok_or_else(|| {
                    CoreError::new(ErrorCode::UnknownId, "unknown patch layer")
                        .with_field("layer_id")
                })?;
            match (layer.job_id, layer.candidate_id) {
                (Some(j), Some(c)) => (j, c),
                _ => {
                    return Err(CoreError::new(
                        ErrorCode::SourceMissing,
                        "the layer doesn't name its job and candidate",
                    ));
                }
            }
        };
        let b = bake_job(core, job_id, candidate_id).map_err(|e| {
            if e.code == ErrorCode::UnknownId {
                CoreError::new(ErrorCode::SourceMissing, e.message)
            } else {
                e
            }
        })?;
        let baked = bake(&b, input.blend)?;
        let patch = NewPatch {
            png: encode_rgba(&baked.rgba)?,
            rect: Some(manifest_rect(baked.rect)),
            region: b.job.region.clone(),
            blend: baked.blend,
        };
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        store::replace_patch_layer(&core.ctx, install_id, p, input.layer_id, patch)
    }
}

// ---------------------------------------------------------------- rgn_iterations_list

/// `rgn_iterations_list` (FR-RGN-40): the project's iterations newest first, plus the newest
/// job before it has candidates while it's unresolved (the active job's strip, or a failed one
/// still offering Retry / Dismiss).
pub struct RgnIterationsList;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IterationRow {
    pub job_id: Ulid,
    pub n: Option<u32>,
    pub dir: Option<String>,
    pub slug: Option<String>,
    pub state: JobState,
    /// Candidates not in the trash (and not invalid).
    pub kept: usize,
    pub returned: usize,
    /// The actual cost when known, else the estimate.
    pub cost: Value,
    /// A layer from this job is in the current stack (the ✓).
    pub accepted_in_stack: bool,
    /// `references[]` count for the chip (FR-RGN-45).
    pub references: usize,
}

impl Command for RgnIterationsList {
    const NAME: &'static str = "rgn_iterations_list";
    type Input = NoInput;
    type Output = Vec<IterationRow>;

    fn run(core: &Core, _: NoInput) -> CoreResult<Vec<IterationRow>> {
        let state = core.state();
        let p = state.project.as_ref().ok_or_else(no_project)?;
        let row = |j: &JobFile| {
            let (n, dir, slug) = match &j.iteration {
                Some(it) => {
                    let name = it.dir.rsplit('/').next().unwrap_or(&it.dir);
                    let slug = name.split_once(' ').map_or("", |(_, s)| s);
                    (Some(it.n), Some(it.dir.clone()), Some(slug.to_owned()))
                }
                None => (None, None, None),
            };
            // While the runner drives it, its copy is newer than the store's.
            let live = state.rgn.runner.job(j.id).unwrap_or(j);
            IterationRow {
                job_id: j.id,
                n,
                dir,
                slug,
                state: live.state,
                kept: kept(p, j.id),
                returned: j.candidate_ids.len(),
                cost: shown_cost(live),
                accepted_in_stack: p.manifest.layers.iter().any(|l| l.job_id == Some(j.id)),
                references: j.references.len(),
            }
        };
        let mut rows: Vec<IterationRow> = p
            .jobs
            .values()
            .filter(|j| j.iteration.is_some())
            .map(row)
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.n));
        let newest = p.jobs.values().max_by_key(|j| (j.created_at, j.id));
        if let Some(j) = newest
            && j.iteration.is_none()
            && (is_active(j.state) || j.resolved_at.is_none())
        {
            rows.insert(0, row(j));
        }
        Ok(rows)
    }
}
