//! PRJ's store API (R4): the Rust calls other features make to read and write project state.
//! Each call is one commit and fails fast with `read_only`, `locked` or `conflict` (FR-PRJ-41,
//! 21, 35). User-facing job and candidate commands are RGN's (`rgn_*`); they call these.
//!
//! Write orders follow [manifest-schema §3](../../../../docs/architecture/manifest-schema.md).

use std::collections::HashSet;
use std::sync::Arc;

use image::RgbImage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ulid::Ulid;

use super::{CommitPlan, OpenProject, commit, push_revision, write_job};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::job::{Iteration, JOB_SCHEMA_VERSION, JobFile, JobInputs, JobSeed, JobState};
use crate::manifest::*;
use crate::naming;
use crate::time::Timestamp;

fn unknown(field: &str, what: &str) -> CoreError {
    CoreError::new(ErrorCode::UnknownId, format!("unknown {what}")).with_field(field)
}

// ---------------------------------------------------------------- reads

/// `read_project` (R6): what EXP, RGN and EDT read. The **current** layer stack.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectView {
    pub project_id: Ulid,
    pub kind: Kind,
    /// `null` for a project made by Generate new (FR-GEN-02).
    pub original: Option<Original>,
    /// `layers[0].id`; `null` with no image (FR-GEN-03).
    pub original_image_id: Option<Ulid>,
    pub layers: Vec<Layer>,
    pub head_revision: Ulid,
    pub draft: Option<Draft>,
    pub export: Option<Value>,
    pub read_only: bool,
}

pub fn read_project(p: &OpenProject) -> ProjectView {
    let m = &p.manifest;
    ProjectView {
        project_id: m.project_id,
        kind: m.kind,
        original: m.original.clone(),
        original_image_id: m.original_image_id(),
        layers: m.layers.clone(),
        head_revision: m.head_revision,
        draft: m.draft.clone(),
        export: m.export.clone(),
        read_only: p.read_only.is_some(),
    }
}

/// `decoded_original` (FR-PRJ-42, R11): the original as RGB8, or `original_unavailable` while
/// the background decode runs (it starts on the first call).
pub fn decoded_original(ctx: &Context, p: &OpenProject) -> CoreResult<Arc<RgbImage>> {
    p.decoded.get(ctx)
}

/// A job file by id.
pub fn job(p: &OpenProject, job_id: Ulid) -> CoreResult<&JobFile> {
    p.jobs.get(&job_id).ok_or_else(|| unknown("job_id", "job"))
}

// ---------------------------------------------------------------- trash helpers

/// Records a trash item; the file moves follow the commit (schema §4.8). Returns its id.
pub(crate) fn trash_files(
    ctx: &Context,
    m: &mut Manifest,
    kind: TrashKind,
    files: Vec<String>,
    reason: &str,
) -> Ulid {
    let id = ctx.new_id();
    m.trash.push(TrashItem {
        id,
        kind,
        files,
        trashed_at: ctx.now(),
        reason: reason.into(),
        extra: Extra::new(),
    });
    id
}

/// Takes a candidate's file back out of its trash item (star, re-pick); the file moves back
/// after the commit.
fn untrash_candidate(m: &mut Manifest, idx: usize) {
    let (Some(tid), file) = (m.candidates[idx].trash_id, m.candidates[idx].file.clone()) else {
        return;
    };
    if let Some(pos) = m.trash.iter().position(|t| t.id == tid) {
        m.trash[pos].files.retain(|f| *f != file);
        if m.trash[pos].files.is_empty() {
            m.trash.remove(pos);
        }
    }
    m.candidates[idx].trash_id = None;
}

/// True if a trashed candidate's file can still be restored (not purged).
fn restorable(p: &OpenProject, c: &Candidate) -> bool {
    match c
        .trash_id
        .and_then(|t| p.manifest.trash.iter().find(|i| i.id == t))
    {
        Some(item) => p.path(&item.trashed_path(&c.file)).is_file() || p.path(&c.file).is_file(),
        None => c.trash_id.is_none() && p.path(&c.file).is_file(),
    }
}

/// #56: candidates `idx` become `discarded` (star cleared); the files not in the trash yet go
/// there as one item. Returns how many files moved.
fn discard_indices(ctx: &Context, m: &mut Manifest, idx: &[usize], reason: &str) -> usize {
    let mut files = vec![];
    for &i in idx {
        let c = &mut m.candidates[i];
        c.status = CandidateStatus::Discarded;
        c.starred = false;
        if c.trash_id.is_none() {
            files.push((i, c.file.clone()));
        }
    }
    if files.is_empty() {
        return 0;
    }
    let n = files.len();
    let names = files.iter().map(|(_, f)| f.clone()).collect();
    let tid = trash_files(ctx, m, TrashKind::Candidates, names, reason);
    for (i, _) in files {
        m.candidates[i].trash_id = Some(tid);
    }
    n
}

/// R9 as amended by #56: a resolved job whose candidates are all discarded (or invalid, or a job
/// with none) sends its region and mask PNGs to the trash. A candidate counts as kept while it is
/// accepted, starred, or `new` / `dismissed` and still in `results/`; one trashed before #56
/// (dismissed, `trash_id` set) doesn't.
fn trash_inputs_if_all_discarded(ctx: &Context, m: &mut Manifest, job: &JobFile) {
    if job.resolved_at.is_none() {
        return;
    }
    let kept = m.candidates.iter().any(|c| {
        c.job_id == job.id
            && (c.starred
                || c.status == CandidateStatus::Accepted
                || (matches!(c.status, CandidateStatus::New | CandidateStatus::Dismissed)
                    && c.trash_id.is_none()))
    });
    let inputs: Vec<String> = job.input_files();
    let already = m
        .trash
        .iter()
        .any(|t| t.files.iter().any(|f| inputs.contains(f)));
    if !kept && !already && !inputs.is_empty() {
        trash_files(
            ctx,
            m,
            TrashKind::JobInputs,
            inputs,
            "every candidate discarded",
        );
    }
}

/// R7: masks no draft, layer, job or revision references go to the trash at the next commit.
pub(crate) fn trash_superseded_masks(
    ctx: &Context,
    p: &OpenProject,
    m: &mut Manifest,
    plan: &CommitPlan,
) {
    if p.refs.incomplete {
        // R102: a mask only an unreadable revision or job uses would look superseded.
        return;
    }
    let mut used: HashSet<Ulid> = p.refs.masks.clone();
    used.extend(m.draft.as_ref().and_then(|d| d.mask_id));
    used.extend(m.layers.iter().filter_map(|l| l.mask_id));
    used.extend(p.jobs.values().filter_map(|j| j.mask_id));
    used.extend(
        plan.jobs_before
            .iter()
            .chain(&plan.jobs_after)
            .filter_map(|j| j.mask_id),
    );
    for rev in &plan.revisions {
        used.extend(rev.layers.iter().filter_map(|l| l.mask_id));
        used.extend(rev.draft.as_ref().and_then(|d| d.mask_id));
    }
    let (keep, drop): (Vec<Mask>, Vec<Mask>) =
        m.masks.drain(..).partition(|k| used.contains(&k.id));
    m.masks = keep;
    if !drop.is_empty() {
        let files = drop.into_iter().map(|k| k.file).collect();
        trash_files(ctx, m, TrashKind::Mask, files, "superseded");
    }
}

// ---------------------------------------------------------------- masks

/// `put_mask` (FR-PRJ-19, FR-EDT-28): writes an 8-bit grey mask PNG (255 = editable, inner-rect
/// size) and points the draft at it, in one commit. `grey` is `rect.w × rect.h` bytes.
pub fn put_mask(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    rect: Rect,
    grey: Vec<u8>,
) -> CoreResult<Mask> {
    p.require_writable()?;
    p.require_image()?;
    let (w, h) = (rect.w, rect.h);
    let (iw, ih) = p.manifest.image_size().unwrap_or((0, 0));
    if w == 0
        || h == 0
        || rect.x < 0
        || rect.y < 0
        || rect.x + i64::from(w) > i64::from(iw)
        || rect.y + i64::from(h) > i64::from(ih)
    {
        return Err(CoreError::new(
            ErrorCode::InvalidRect,
            "the mask rect must lie inside the image",
        )
        .with_field("rect"));
    }
    if grey.len() != w as usize * h as usize {
        return Err(CoreError::new(
            ErrorCode::SizeMismatch,
            format!("a {w}×{h} mask is {} bytes, not {}", w * h, grey.len()),
        ));
    }
    let img = image::GrayImage::from_raw(w, h, grey)
        .ok_or_else(|| CoreError::internal("mask buffer size"))?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| CoreError::internal(format!("encoding the mask: {e}")))?;
    let id = ctx.new_id();
    let mask = Mask {
        id,
        file: mask_file(id),
        rect,
        created_at: ctx.now(),
        extra: Extra::new(),
    };
    let record = mask.clone();
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        plan.files.push((record.file.clone(), png));
        m.masks.push(record);
        let draft = m.draft.get_or_insert_with(|| Draft::empty(ctx.now()));
        draft.mask_id = Some(id);
        draft.updated_at = ctx.now();
        Ok(())
    })?;
    Ok(mask)
}

// ---------------------------------------------------------------- jobs

/// What RGN knows when it creates a job (schema §4.6). The id, times and state come from PRJ.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct NewJob {
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub capability_snapshot: Value,
    #[serde(default)]
    pub final_prompt: String,
    #[serde(default)]
    pub controls: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    pub candidate_count: u32,
    #[serde(default)]
    pub seed_requested: Option<i64>,
    #[serde(default)]
    pub region: Value,
    #[serde(default)]
    pub fit: Value,
    #[serde(default)]
    pub mask_id: Option<Ulid>,
    #[serde(default)]
    pub estimate: Value,
    #[serde(default)]
    pub provider: Value,
    /// `references[]` (#51, FR-RGN-43): written with `requested`, never changed.
    #[serde(default)]
    pub references: Vec<Value>,
    /// Fields RGN adds to the job file (kept verbatim, written after the known ones).
    #[serde(default)]
    pub extra: Extra,
}

/// `record_job` (R2): `region.png` and `mask.png` as sent (after fit, before polarity
/// conversion) → `job.json` in state `requested` (no handle) → the manifest index row.
pub fn record_job(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    new: NewJob,
    region_png: Vec<u8>,
    mask_png: Vec<u8>,
) -> CoreResult<JobFile> {
    record(ctx, install_id, p, new, Some((region_png, mask_png)))
}

/// `record_job` for a `generate` job (FR-GEN-22): `job.json` only, no input PNGs, `inputs`
/// null.
pub fn record_job_without_inputs(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    new: NewJob,
) -> CoreResult<JobFile> {
    record(ctx, install_id, p, new, None)
}

fn record(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    new: NewJob,
    inputs: Option<(Vec<u8>, Vec<u8>)>,
) -> CoreResult<JobFile> {
    p.require_writable()?;
    if let Some(mid) = new.mask_id
        && !p.manifest.masks.iter().any(|k| k.id == mid)
        && !p.refs.masks.contains(&mid)
    {
        return Err(unknown("mask_id", "mask"));
    }
    let now = ctx.now();
    let id = ctx.new_id();
    let job = JobFile {
        schema_version: JOB_SCHEMA_VERSION,
        id,
        created_at: now,
        updated_at: now,
        state: JobState::Requested,
        state_times: [(JobState::Requested.as_str().to_owned(), now)].into(),
        resolved_at: None,
        operation: new.operation,
        model_id: new.model_id,
        capability_snapshot: new.capability_snapshot,
        final_prompt: new.final_prompt,
        controls: new.controls,
        candidate_count: new.candidate_count,
        seed: JobSeed {
            requested: new.seed_requested,
            returned: None,
        },
        region: new.region,
        fit: new.fit,
        mask_id: new.mask_id,
        // R97: a region edit job remembers the base image it was built on.
        base_layer_id: inputs.as_ref().and_then(|_| p.manifest.original_image_id()),
        inputs: inputs.as_ref().map(|_| JobInputs::default()),
        references: new.references,
        estimate: new.estimate,
        cost: Value::Null,
        provider: new.provider,
        iteration: None,
        candidate_ids: vec![],
        error: None,
        extra: new.extra,
    };
    let record = job.clone();
    commit(ctx, install_id, p, |_, _, m, plan| {
        if let Some((region_png, mask_png)) = inputs {
            let files = record.input_files();
            plan.files.push((files[0].clone(), region_png));
            plan.files.push((files[1].clone(), mask_png));
        }
        m.jobs.push(JobIndex {
            id: record.id,
            state: record.state,
            iteration: None,
            extra: Extra::new(),
        });
        plan.jobs_before.push(record);
        Ok(())
    })?;
    Ok(job)
}

/// A job update from RGN's runner. `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct JobUpdate {
    #[serde(default)]
    pub state: Option<JobState>,
    /// Replaces `provider{}` verbatim.
    #[serde(default)]
    pub provider: Option<Value>,
    #[serde(default)]
    pub seed_returned: Option<i64>,
    #[serde(default)]
    pub estimate: Option<Value>,
    #[serde(default)]
    pub cost: Option<Value>,
    /// `Some(Value::Null)` clears it.
    #[serde(default)]
    pub error: Option<Value>,
}

/// `update_job` (R2, schema §3): one transition or data update, written to `job.json` only (the
/// commit point for job state). `fetched` and `closed` are set by `add_candidates`, accept and
/// `resolve_job`.
///
/// Refused with `read_only` or `conflict` while the project can't be written; RGN's runner then
/// spools it (FR-PRJ-40).
pub fn update_job(
    ctx: &Context,
    p: &mut OpenProject,
    job_id: Ulid,
    update: JobUpdate,
) -> CoreResult<JobFile> {
    let mut next = job(p, job_id)?.clone();
    if let Some(state) = update.state {
        if matches!(state, JobState::Fetched | JobState::Closed) && state != next.state {
            return Err(CoreError::invalid(
                "state",
                "fetched and closed are set by add_candidates, accept and resolve_job",
            ));
        }
        next.transition(state, ctx.now())
            .map_err(|e| CoreError::invalid("state", e))?;
    } else {
        next.updated_at = ctx.now();
    }
    if let Some(provider) = update.provider {
        next.provider = provider;
    }
    if let Some(seed) = update.seed_returned {
        next.seed.returned = Some(seed);
    }
    if let Some(estimate) = update.estimate {
        next.estimate = estimate;
    }
    if let Some(cost) = update.cost {
        next.cost = cost;
    }
    if let Some(error) = update.error {
        next.error = (!error.is_null()).then_some(error);
    }
    if let Some(e) = blocked(p) {
        return Err(e);
    }
    // R102: refused (and spooled by the runner) once the folder moved, never written to a
    // recreated old path.
    super::require_folder(p)?;
    write_job(ctx, p, &next)?;
    set_job(p, next.clone());
    Ok(next)
}

fn set_job(p: &mut OpenProject, job: JobFile) {
    if let Some(row) = p.manifest.jobs.iter_mut().find(|r| r.id == job.id) {
        row.state = job.state;
        row.iteration = job.iteration.as_ref().map(|i| i.n);
    }
    p.jobs.insert(job.id, job);
}

/// Why the project can't be written right now, if it can't (read-only or in conflict).
fn blocked(p: &OpenProject) -> Option<CoreError> {
    if let Err(e) = p.require_writable() {
        return Some(e);
    }
    p.conflict.is_some().then(super::conflict::conflict_error)
}

/// One returned candidate, untouched (R10).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewCandidate {
    pub bytes: Vec<u8>,
    pub seed: Option<i64>,
    pub provider_meta: Value,
    /// RGN's decode check failed (FR-RGN-24); a header that doesn't probe is invalid too.
    pub invalid: bool,
    /// Fields RGN adds to the candidate record (e.g. `aspect_warning`, FR-RGN-24).
    pub extra: Extra,
}

/// The next iteration folder for `prompt` (FR-RGN-39): `n` = 1 + the highest in the project,
/// `results/NNN <slug>`, ` (2)` if that folder is taken.
fn next_iteration(p: &OpenProject, prompt: &str, empty_slug: &str) -> Iteration {
    let n = p
        .jobs
        .values()
        .filter_map(|j| j.iteration.as_ref().map(|i| i.n))
        .chain(p.manifest.jobs.iter().filter_map(|j| j.iteration))
        .max()
        .unwrap_or(0)
        + 1;
    let base = naming::iteration_dir_name(n, &naming::iteration_slug_or(prompt, empty_slug));
    let name = naming::first_free(&base, |c| {
        std::fs::symlink_metadata(p.path(&format!("{RESULTS_DIR}/{c}"))).is_ok()
    });
    Iteration {
        n,
        dir: format!("{RESULTS_DIR}/{name}"),
    }
}

/// `detail.reason` of `add_candidates`' `conflict` when a candidate file already belongs to a
/// candidate record: retrying never helps (R102), unlike a sync conflict.
pub const FILE_TAKEN: &str = "file_taken";

/// `add_candidates` (FR-PRJ-30, schema §3): `job.json` with `iteration` set → the candidate files
/// in `results/NNN <slug>/NN.<ext>` → the manifest → `job.json` `fetched`. A re-fetch after a
/// crash reuses the job's folder and replaces only files no candidate record references.
///
/// Refused with `read_only` or `conflict` while the project can't be written; RGN's runner then
/// spools it (FR-PRJ-40).
pub fn add_candidates(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
    candidates: Vec<NewCandidate>,
) -> CoreResult<Vec<Candidate>> {
    if let Some(e) = blocked(p) {
        return Err(e);
    }
    super::require_folder(p)?;
    let mut job = job(p, job_id)?.clone();
    if job.state != JobState::Succeeded {
        return Err(CoreError::invalid(
            "job_id",
            format!(
                "candidates are added to a succeeded job, not a {} one",
                job.state.as_str()
            ),
        ));
    }
    if candidates.is_empty() {
        return Err(CoreError::invalid("candidates", "no candidates"));
    }
    if job.iteration.is_none() {
        // FR-GEN-24: an empty slug is `new` for a New image job, `edit` otherwise.
        let empty_slug = if job.is_generate() { "new" } else { "edit" };
        job.iteration = Some(next_iteration(p, job.typed_prompt(), empty_slug));
        job.updated_at = ctx.now();
        write_job(ctx, p, &job)?;
        p.jobs.insert(job_id, job.clone());
    }
    let dir = job
        .iteration
        .as_ref()
        .map(|i| i.dir.clone())
        .unwrap_or_default();
    let n = job.iteration.as_ref().map(|i| i.n);
    let referenced: HashSet<String> = p
        .manifest
        .candidates
        .iter()
        .map(|c| c.file.clone())
        .collect();
    let mut records = vec![];
    let mut files = vec![];
    for (index, c) in candidates.into_iter().enumerate() {
        let probe = pf_imaging::probe_bytes(&c.bytes).ok();
        let ext = pf_imaging::sniff(&c.bytes).map_or("bin", |f| f.extension());
        let file = format!("{dir}/{:02}.{ext}", index + 1);
        if referenced.contains(&file) {
            return Err(CoreError::new(
                ErrorCode::Conflict,
                format!("{file} already belongs to a candidate"),
            )
            .with_detail(serde_json::json!({ "reason": FILE_TAKEN })));
        }
        records.push(Candidate {
            id: ctx.new_id(),
            job_id,
            index: index as u32,
            file: file.clone(),
            format: probe.map_or_else(
                || {
                    pf_imaging::sniff(&c.bytes)
                        .map_or("unknown", |f| f.as_str())
                        .into()
                },
                |i| i.format.as_str().into(),
            ),
            width: probe.map_or(0, |i| i.width),
            height: probe.map_or(0, |i| i.height),
            seed: c.seed,
            provider_meta: c.provider_meta,
            status: if c.invalid || probe.is_none() {
                CandidateStatus::Invalid
            } else {
                CandidateStatus::New
            },
            starred: false,
            trash_id: None,
            purged_at: None,
            extra: c.extra,
        });
        files.push((file, c.bytes));
    }
    let out = records.clone();
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        plan.files = files;
        let mut done = job.clone();
        done.candidate_ids = records.iter().map(|c| c.id).collect();
        done.transition(JobState::Fetched, ctx.now())
            .map_err(CoreError::internal)?;
        if let Some(row) = m.jobs.iter_mut().find(|r| r.id == job_id) {
            row.state = JobState::Fetched;
            row.iteration = n;
        }
        m.candidates.extend(records);
        plan.jobs_after.push(done);
        Ok(())
    })?;
    Ok(out)
}

/// How a job was resolved (FR-RGN-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveReason {
    Dismiss,
    NextSubmit,
}

/// `resolve_job` (FR-RGN-28, R9, #56): dismiss, or the next submit. Remaining `new` candidates
/// become `dismissed` and stay in `results/`; only a job whose candidates are all discarded (or
/// that has none) trashes its inputs. A `fetched` job becomes `closed`; a terminal job only gets
/// `resolved_at`. Resolving twice is a no-op. Both reasons do the same since #56.
pub fn resolve_job(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
    _reason: ResolveReason,
) -> CoreResult<JobFile> {
    p.require_writable()?;
    let current = job(p, job_id)?.clone();
    if current.resolved_at.is_some() {
        return Ok(current);
    }
    if !(current.state == JobState::Fetched || current.state.is_terminal()) {
        return Err(CoreError::invalid(
            "job_id",
            format!(
                "a {} job can't be resolved; cancel it first",
                current.state.as_str()
            ),
        ));
    }
    let mut next = current.clone();
    if next.state == JobState::Fetched {
        next.transition(JobState::Closed, ctx.now())
            .map_err(CoreError::internal)?;
    }
    next.resolved_at = Some(ctx.now());
    let done = next.clone();
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        for c in m.candidates.iter_mut().filter(|c| c.job_id == job_id) {
            if c.status == CandidateStatus::New {
                c.status = CandidateStatus::Dismissed;
            }
        }
        trash_inputs_if_all_discarded(ctx, m, &done);
        if let Some(row) = m.jobs.iter_mut().find(|r| r.id == job_id) {
            row.state = done.state;
        }
        plan.jobs_after.push(done);
        Ok(())
    })?;
    Ok(next)
}

// ---------------------------------------------------------------- candidates

fn candidate_index(m: &Manifest, candidate_id: Ulid) -> CoreResult<usize> {
    m.candidates
        .iter()
        .position(|c| c.id == candidate_id)
        .ok_or_else(|| unknown("candidate_id", "candidate"))
}

/// `set_star` (FR-RGN-28, #56): a favourite marker. Starring a trashed one brings its file back;
/// unstarring leaves the file where it is.
pub fn set_star(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    candidate_id: Ulid,
    starred: bool,
) -> CoreResult<Candidate> {
    p.require_writable()?;
    let idx = candidate_index(&p.manifest, candidate_id)?;
    let c = &p.manifest.candidates[idx];
    if starred && c.trash_id.is_some() && !restorable(p, c) {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            "the candidate's file was purged from the trash",
        ));
    }
    commit(ctx, install_id, p, |_, _, m, _| {
        m.candidates[idx].starred = starred;
        if starred {
            untrash_candidate(m, idx);
        }
        Ok(())
    })?;
    Ok(p.manifest.candidates[idx].clone())
}

/// `discard_candidate` (FR-RGN-28, #56): `discarded`, its file to the trash. An explicit discard
/// also clears the star. An accepted candidate can't be discarded. A resolved job whose last kept
/// candidate this was also trashes its inputs.
pub fn discard_candidate(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    candidate_id: Ulid,
) -> CoreResult<Candidate> {
    p.require_writable()?;
    let idx = candidate_index(&p.manifest, candidate_id)?;
    if p.manifest.candidates[idx].status == CandidateStatus::Accepted {
        return Err(CoreError::invalid(
            "candidate_id",
            "an accepted candidate can't be discarded",
        ));
    }
    let job = p.jobs.get(&p.manifest.candidates[idx].job_id).cloned();
    commit(ctx, install_id, p, |ctx, _, m, _| {
        discard_indices(ctx, m, &[idx], "discarded");
        if let Some(job) = &job {
            trash_inputs_if_all_discarded(ctx, m, job);
        }
        Ok(())
    })?;
    Ok(p.manifest.candidates[idx].clone())
}

/// `discard_rest` (#56): one commit that discards every candidate of the job except the accepted
/// and starred ones; their files go to the trash as one item. Returns how many files moved (0, and
/// no commit, when there is nothing to discard).
pub fn discard_rest(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
) -> CoreResult<usize> {
    p.require_writable()?;
    let job = job(p, job_id)?.clone();
    let idx: Vec<usize> = (0..p.manifest.candidates.len())
        .filter(|&i| {
            let c = &p.manifest.candidates[i];
            c.job_id == job_id
                && c.status != CandidateStatus::Accepted
                && !c.starred
                && !(c.status == CandidateStatus::Discarded && c.trash_id.is_some())
        })
        .collect();
    if idx.is_empty() {
        return Ok(0);
    }
    let mut moved = 0;
    commit(ctx, install_id, p, |ctx, _, m, _| {
        moved = discard_indices(ctx, m, &idx, "discarded the rest");
        trash_inputs_if_all_discarded(ctx, m, &job);
        Ok(())
    })?;
    Ok(moved)
}

/// What a `discard_all` binned, so its Undo puts every candidate back as it was (#61).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardAll {
    pub job_id: Ulid,
    /// The one trash item holding the candidate files.
    pub item_id: Ulid,
    /// Each binned candidate's status and star before the discard.
    pub prior: Vec<(Ulid, CandidateStatus, bool)>,
}

/// `discard_all` (#61, FR-RGN-46): one commit that discards every candidate of the job still in
/// `results/`, starred ones included (their star is cleared, #57), never the accepted one; the
/// files go to the trash as one item, and the job's inputs follow when nothing is kept (#56).
/// `None`, and no commit, when there is nothing to discard.
pub fn discard_all(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
) -> CoreResult<Option<DiscardAll>> {
    p.require_writable()?;
    let job = job(p, job_id)?.clone();
    let idx: Vec<usize> = (0..p.manifest.candidates.len())
        .filter(|&i| discardable(&p.manifest.candidates[i], job_id))
        .collect();
    if idx.is_empty() {
        return Ok(None);
    }
    let prior = idx
        .iter()
        .map(|&i| {
            let c = &p.manifest.candidates[i];
            (c.id, c.status, c.starred)
        })
        .collect();
    let first = idx[0];
    commit(ctx, install_id, p, |ctx, _, m, _| {
        discard_indices(ctx, m, &idx, "discarded all");
        trash_inputs_if_all_discarded(ctx, m, &job);
        Ok(())
    })?;
    let item_id = p.manifest.candidates[first]
        .trash_id
        .ok_or_else(|| CoreError::internal("discarded without a trash item"))?;
    Ok(Some(DiscardAll {
        job_id,
        item_id,
        prior,
    }))
}

/// A candidate of `job_id` that Discard all bins: not accepted, and its file not in the trash.
pub fn discardable(c: &Candidate, job_id: Ulid) -> bool {
    c.job_id == job_id && c.status != CandidateStatus::Accepted && c.trash_id.is_none()
}

/// The Undo of [`discard_all`] (#61): one commit that brings back every file still in its trash
/// item, plus the job's region/mask item if the discard trashed them (#56), and gives each
/// candidate its old status and star. A candidate taken out of the item since (starred from the
/// trash) is left as it is now. Returns how many candidates came back.
pub fn undo_discard_all(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    undo: &DiscardAll,
) -> CoreResult<usize> {
    p.require_writable()?;
    if !p.manifest.trash.iter().any(|t| t.id == undo.item_id) {
        return Err(CoreError::new(
            ErrorCode::UnknownId,
            "those results aren't in the trash any more",
        )
        .with_field("undo_id"));
    }
    let inputs = p
        .jobs
        .get(&undo.job_id)
        .map(JobFile::input_files)
        .unwrap_or_default();
    let mut items = vec![undo.item_id];
    items.extend(
        p.manifest
            .trash
            .iter()
            .filter(|t| {
                t.kind == TrashKind::JobInputs && t.files.iter().any(|f| inputs.contains(f))
            })
            .map(|t| t.id),
    );
    let back: Vec<(Ulid, CandidateStatus, bool)> = undo
        .prior
        .iter()
        .filter(|(id, _, _)| {
            p.manifest
                .candidates
                .iter()
                .any(|c| c.id == *id && c.trash_id == Some(undo.item_id))
        })
        .copied()
        .collect();
    let n = back.len();
    super::retention::restore_items(ctx, install_id, p, &items, |m| {
        for (id, status, starred) in &back {
            if let Some(c) = m.candidates.iter_mut().find(|c| c.id == *id) {
                c.status = *status;
                c.starred = *starred;
            }
        }
    })?;
    Ok(n)
}

/// A baked patch to place: the RGBA PNG (cropped to alpha > 0) and its record fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewPatch {
    pub png: Vec<u8>,
    /// Placement of the cropped PNG, image px.
    pub rect: Option<Rect>,
    /// `{inner, outer, margin}`.
    pub region: Value,
    /// `{feather_px, colour_match{…}}`.
    pub blend: Value,
}

/// What accept and re-tune return.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LayerCommit {
    pub layer_id: Ulid,
    pub revision_id: Ulid,
}

fn patch_layer(id: Ulid, patch: &NewPatch, job: &JobFile, candidate_id: Ulid) -> CoreResult<Layer> {
    let rect = patch
        .rect
        .ok_or_else(|| CoreError::invalid("rect", "a patch layer needs its rect"))?;
    if patch.png.is_empty() {
        return Err(CoreError::invalid("png", "the patch PNG is empty"));
    }
    let mut l = Layer::original(id, 0, 0);
    l.kind = LayerKind::Patch;
    l.file = Some(layer_file(id));
    l.cx = rect.x as f64 + f64::from(rect.w) / 2.0;
    l.cy = rect.y as f64 + f64::from(rect.h) / 2.0;
    l.job_id = Some(job.id);
    l.candidate_id = Some(candidate_id);
    l.mask_id = job.mask_id;
    l.region = Some(patch.region.clone());
    l.rect = Some(rect);
    l.blend = Some(patch.blend.clone());
    Ok(l)
}

/// R97: a region edit candidate is accepted only on the base image its job was built on (a
/// pick since then changed `layers[0]`); the candidate stays as it is. Jobs from before R97
/// carry no base and aren't checked.
pub fn check_same_base(p: &OpenProject, job: &JobFile) -> CoreResult<()> {
    match job.base_layer_id {
        Some(base) if p.manifest.original_image_id() != Some(base) => Err(CoreError::new(
            ErrorCode::InvalidCandidate,
            "that candidate was made on a different base image; it can't be accepted on this one",
        )
        .with_field("candidate_id")),
        _ => Ok(()),
    }
}

/// The newest job (by creation), which is the only one a re-pick can use (R32).
fn newest_job(p: &OpenProject) -> Option<&JobFile> {
    p.jobs.values().max_by_key(|j| (j.created_at, j.id))
}

/// `accept_candidate` (FR-PRJ-31, FR-RGN-27, R4): one commit with the baked patch layer, the
/// candidate statuses (the other `new` ones `dismissed`, kept in `results/`, #56) and the
/// `accept` revision; then `job.json` `closed`. Also a **re-pick** (R32, FR-RGN-36): a candidate of the
/// newest, `closed` job whose layer is no longer in the stack; a trashed file is restored in the
/// same commit. Otherwise `repick_unavailable`.
pub fn accept_candidate(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
    candidate_id: Ulid,
    patch: NewPatch,
    label: Option<String>,
) -> CoreResult<LayerCommit> {
    p.require_writable()?;
    let job = job(p, job_id)?.clone();
    let idx = candidate_index(&p.manifest, candidate_id)?;
    let cand = p.manifest.candidates[idx].clone();
    if cand.job_id != job_id {
        return Err(CoreError::invalid(
            "candidate_id",
            "the candidate belongs to another job",
        ));
    }
    if cand.status == CandidateStatus::Invalid {
        return Err(CoreError::invalid(
            "candidate_id",
            "the candidate won't decode",
        ));
    }
    check_same_base(p, &job)?;
    let repick = match job.state {
        JobState::Fetched => false,
        JobState::Closed => {
            let newest = newest_job(p).map(|j| j.id) == Some(job_id);
            let in_stack = p.manifest.layers.iter().any(|l| l.job_id == Some(job_id));
            let had_accept = p
                .manifest
                .candidates
                .iter()
                .any(|c| c.job_id == job_id && c.status == CandidateStatus::Accepted);
            if !newest || in_stack || !had_accept || !restorable(p, &cand) {
                return Err(CoreError::new(
                    ErrorCode::RepickUnavailable,
                    "that candidate can't be accepted any more (a newer job exists, its patch is in the stack, or the file was purged)",
                ));
            }
            true
        }
        other => {
            return Err(CoreError::invalid(
                "job_id",
                format!("a {} job has nothing to accept", other.as_str()),
            ));
        }
    };
    let layer_id = ctx.new_id();
    let layer = patch_layer(layer_id, &patch, &job, candidate_id)?;
    let label = label.or_else(|| {
        let l: String = job.typed_prompt().chars().take(40).collect();
        (!l.is_empty()).then_some(l)
    });
    let now = ctx.now();
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        plan.files.push((layer_file(layer_id), patch.png));
        m.layers.push(layer);
        m.candidates[idx].status = CandidateStatus::Accepted;
        untrash_candidate(m, idx);
        if !repick {
            for c in m.candidates.iter_mut().filter(|c| c.job_id == job_id) {
                if c.status == CandidateStatus::New {
                    c.status = CandidateStatus::Dismissed;
                }
            }
            let mut closed = job.clone();
            closed
                .transition(JobState::Closed, now)
                .map_err(CoreError::internal)?;
            closed.resolved_at = Some(now);
            if let Some(row) = m.jobs.iter_mut().find(|r| r.id == job_id) {
                row.state = JobState::Closed;
            }
            plan.jobs_after.push(closed);
        }
        plan.revisions
            .push(push_revision(ctx, m, RevisionReason::Accept, label));
        Ok(())
    })?;
    Ok(LayerCommit {
        layer_id,
        revision_id: p.manifest.head_revision,
    })
}

/// `replace_patch_layer` (re-tune, FR-RGN-29, R3): a **new** layer id and file replace the layer
/// in the stack (`retune_of` = the old id), plus a `retune` revision. The old file stays,
/// referenced by its revision.
pub fn replace_patch_layer(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    layer_id: Ulid,
    patch: NewPatch,
) -> CoreResult<LayerCommit> {
    p.require_writable()?;
    let pos = p
        .manifest
        .layers
        .iter()
        .position(|l| l.id == layer_id && l.kind == LayerKind::Patch)
        .ok_or_else(|| unknown("layer_id", "patch layer"))?;
    let old = p.manifest.layers[pos].clone();
    let job_id = old
        .job_id
        .ok_or_else(|| CoreError::internal("a patch layer without a job"))?;
    let job = job(p, job_id)?.clone();
    let new_id = ctx.new_id();
    let mut layer = patch_layer(new_id, &patch, &job, old.candidate_id.unwrap_or(job_id))?;
    layer.mask_id = old.mask_id;
    layer.opacity = old.opacity;
    layer.visible = old.visible;
    layer.retune_of = Some(old.id);
    layer.extra = old.extra.clone();
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        plan.files.push((layer_file(new_id), patch.png));
        m.layers[pos] = layer;
        plan.revisions
            .push(push_revision(ctx, m, RevisionReason::Retune, None));
        Ok(())
    })?;
    Ok(LayerCommit {
        layer_id: new_id,
        revision_id: p.manifest.head_revision,
    })
}

// ---------------------------------------------------------------- export

/// Merges EXP's fields into the manifest `export` record (schema §4.10). EXP owns the meaning.
pub(crate) fn merge_export(m: &mut Manifest, fields: Map<String, Value>) {
    let mut record = match m.export.take() {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
    };
    for (k, v) in fields {
        record.insert(k, v);
    }
    m.export = Some(Value::Object(record));
}

/// `put_export` (R6, FR-EXP-25/27): commits export settings and/or `last`.
pub fn put_export(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    fields: Map<String, Value>,
) -> CoreResult<u64> {
    commit(ctx, install_id, p, |_, _, m, _| {
        merge_export(m, fields);
        Ok(())
    })
}

/// When the next trash purge happens (FR-PRJ-32, for disk use; slice 5 runs it).
pub fn next_purge_at(p: &OpenProject) -> Option<Timestamp> {
    p.manifest
        .trash
        .iter()
        .map(|t| {
            t.trashed_at
                .plus(std::time::Duration::from_secs(30 * 86_400))
        })
        .min()
}

// ---------------------------------------------------------------- pick (GEN)

mod pick;
pub(crate) use pick::statuses_follow_base;
pub use pick::{BaseCommit, pick_base, pick_label, working_name};

#[cfg(test)]
pub(crate) mod tests;
