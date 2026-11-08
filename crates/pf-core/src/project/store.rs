//! PRJ's store API (R4): the Rust calls other features make to read and write project state.
//! Each call is one commit and fails fast with `read_only`, `locked` or `conflict` (FR-PRJ-41,
//! 21, 35). User-facing job and candidate commands are RGN's (`rgn_*`); they call these.
//!
//! Write orders follow [manifest-schema §3](../../../../docs/architecture/manifest-schema.md).

use std::collections::HashSet;
use std::sync::Arc;

use image::RgbImage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ulid::Ulid;

use super::{CommitPlan, OpenProject, commit, emit_save_failed, push_revision, spool, write_job};
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
    pub original: Original,
    pub original_image_id: Ulid,
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
        original_image_id: m.original_image_id().unwrap_or(m.project_id),
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

/// Unstarred candidates of `job_id` in `statuses` that aren't in the trash yet → one trash item.
fn trash_unkept(ctx: &Context, m: &mut Manifest, job_id: Ulid, reason: &str) {
    let idx: Vec<usize> = (0..m.candidates.len())
        .filter(|&i| {
            let c = &m.candidates[i];
            c.job_id == job_id
                && !c.starred
                && c.trash_id.is_none()
                && matches!(
                    c.status,
                    CandidateStatus::Dismissed
                        | CandidateStatus::Discarded
                        | CandidateStatus::Invalid
                )
        })
        .collect();
    if idx.is_empty() {
        return;
    }
    let files = idx.iter().map(|&i| m.candidates[i].file.clone()).collect();
    let tid = trash_files(ctx, m, TrashKind::Candidates, files, reason);
    for i in idx {
        m.candidates[i].trash_id = Some(tid);
    }
}

/// R9: a job that ends with nothing accepted or starred sends its inputs to the trash.
fn trash_inputs_if_unkept(ctx: &Context, m: &mut Manifest, job: &JobFile) {
    let kept = m
        .candidates
        .iter()
        .any(|c| c.job_id == job.id && (c.starred || c.status == CandidateStatus::Accepted));
    let inputs: Vec<String> = job.input_files().to_vec();
    let already = m
        .trash
        .iter()
        .any(|t| t.files.iter().any(|f| inputs.contains(f)));
    if !kept && !already {
        trash_files(
            ctx,
            m,
            TrashKind::JobInputs,
            inputs,
            "job ended with nothing kept",
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
    let (w, h) = (rect.w, rect.h);
    let o = &p.manifest.original;
    if w == 0
        || h == 0
        || rect.x < 0
        || rect.y < 0
        || rect.x + i64::from(w) > i64::from(o.width)
        || rect.y + i64::from(h) > i64::from(o.height)
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
        let draft = m.draft.get_or_insert_with(|| Draft {
            final_prompt: String::new(),
            model_id: None,
            controls: Default::default(),
            seed: None,
            region: None,
            mask_id: None,
            updated_at: ctx.now(),
            extra: Extra::new(),
        });
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
        inputs: JobInputs::default(),
        estimate: new.estimate,
        cost: Value::Null,
        provider: new.provider,
        iteration: None,
        candidate_ids: vec![],
        error: None,
        extra: Extra::new(),
    };
    let record = job.clone();
    commit(ctx, install_id, p, |_, _, m, plan| {
        let [region, mask] = record.input_files();
        plan.files.push((region, region_png));
        plan.files.push((mask, mask_png));
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
/// While the project can't be written (`read_only`, `conflict`, `locked` after 3 s), the update
/// goes to the spool (FR-PRJ-40) and is kept in memory, so the runner's next transition
/// validates; the call still fails with that code and `detail.spooled = true`.
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
    let written = match blocked(p) {
        Some(e) => Err(e),
        None => write_job(ctx, p, &next),
    };
    if let Err(e) = written {
        if !spool::spoolable(e.code) {
            return Err(e);
        }
        let seq = spool::spool_job(ctx, p, &next)?;
        if e.code == ErrorCode::Locked {
            emit_save_failed(ctx, ErrorCode::Locked, true);
        }
        set_job(p, next);
        return Err(spooled(e, json!({ "spool_seq": seq })));
    }
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

/// Marks an error as spooled: `detail.spooled = true` plus `extra`.
fn spooled(e: CoreError, extra: Value) -> CoreError {
    let mut detail = match e.detail.clone() {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    detail.insert("spooled".into(), Value::Bool(true));
    if let Value::Object(x) = extra {
        detail.extend(x);
    }
    let detail = Value::Object(detail);
    e.with_detail(detail)
}

/// Replays a spooled job file (FR-PRJ-40): written unless the file on disk is already as late.
pub(crate) fn apply_spooled_job(
    ctx: &Context,
    p: &mut OpenProject,
    spooled: JobFile,
) -> CoreResult<()> {
    let path = p.path(&format!("{}/{}", spooled.dir(), crate::job::JOB_FILE));
    let on_disk: Option<JobFile> = super::read_json(&path).ok();
    if on_disk
        .as_ref()
        .is_some_and(|d| !super::conflict::later(&spooled, d))
    {
        return Ok(());
    }
    if let Some(e) = blocked(p) {
        return Err(e);
    }
    write_job(ctx, p, &spooled)?;
    // Memory may already hold a later state from a write after the spool: keep the later one.
    let keep = match p.jobs.get(&spooled.id) {
        Some(mem) if super::conflict::later(mem, &spooled) => mem.clone(),
        _ => spooled,
    };
    set_job(p, keep);
    Ok(())
}

/// One returned candidate, untouched (R10).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewCandidate {
    pub bytes: Vec<u8>,
    pub seed: Option<i64>,
    pub provider_meta: Value,
    /// RGN's decode check failed (FR-RGN-24); a header that doesn't probe is invalid too.
    pub invalid: bool,
}

/// The next iteration folder for `prompt` (FR-RGN-39): `n` = 1 + the highest in the project,
/// `results/NNN <slug>`, ` (2)` if that folder is taken.
fn next_iteration(p: &OpenProject, prompt: &str) -> Iteration {
    let n = p
        .jobs
        .values()
        .filter_map(|j| j.iteration.as_ref().map(|i| i.n))
        .chain(p.manifest.jobs.iter().filter_map(|j| j.iteration))
        .max()
        .unwrap_or(0)
        + 1;
    let base = naming::iteration_dir_name(n, &naming::iteration_slug(prompt));
    let name = naming::first_free(&base, |c| {
        std::fs::symlink_metadata(p.path(&format!("{RESULTS_DIR}/{c}"))).is_ok()
    });
    Iteration {
        n,
        dir: format!("{RESULTS_DIR}/{name}"),
    }
}

/// `add_candidates` (FR-PRJ-30, schema §3): `job.json` with `iteration` set → the candidate files
/// in `results/NNN <slug>/NN.<ext>` → the manifest → `job.json` `fetched`. A re-fetch after a
/// crash reuses the job's folder and replaces only files no candidate record references.
///
/// While the project can't be written (`read_only`, `conflict`, `locked` after 3 s), the bytes go
/// to the spool (FR-PRJ-40) and are committed by its replay: the call fails with that code and
/// `detail{spooled: true, candidate_ids}` (the ids the replay will use).
pub fn add_candidates(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
    candidates: Vec<NewCandidate>,
) -> CoreResult<Vec<Candidate>> {
    let with_ids: Vec<(Ulid, NewCandidate)> =
        candidates.into_iter().map(|c| (ctx.new_id(), c)).collect();
    let ids: Vec<Ulid> = with_ids.iter().map(|(id, _)| *id).collect();
    let keep: Vec<(Ulid, NewCandidate)> = with_ids.clone();
    match add_candidates_with_ids(ctx, install_id, p, job_id, with_ids) {
        Err(e) if spool::spoolable(e.code) && !keep.is_empty() && p.jobs.contains_key(&job_id) => {
            let seq = spool::spool_candidates(ctx, p, job_id, &keep)?;
            if e.code == ErrorCode::Locked {
                emit_save_failed(ctx, ErrorCode::Locked, true);
            }
            Err(spooled(
                e,
                json!({ "spool_seq": seq, "candidate_ids": ids }),
            ))
        }
        other => other,
    }
}

/// `add_candidates` with the candidate ids given (the spool replay keeps the ids it spooled).
pub(crate) fn add_candidates_with_ids(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
    candidates: Vec<(Ulid, NewCandidate)>,
) -> CoreResult<Vec<Candidate>> {
    if let Some(e) = blocked(p) {
        return Err(e);
    }
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
        job.iteration = Some(next_iteration(p, &job.final_prompt));
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
    for (index, (cand_id, c)) in candidates.into_iter().enumerate() {
        let probe = pf_imaging::probe_bytes(&c.bytes).ok();
        let ext = pf_imaging::sniff(&c.bytes).map_or("bin", |f| f.extension());
        let file = format!("{dir}/{:02}.{ext}", index + 1);
        if referenced.contains(&file) {
            return Err(CoreError::new(
                ErrorCode::Conflict,
                format!("{file} already belongs to a candidate"),
            ));
        }
        records.push(Candidate {
            id: cand_id,
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
            extra: Extra::new(),
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

/// `resolve_job` (FR-RGN-28, R9): dismiss, or the next submit. Remaining `new` candidates become
/// `dismissed`; unstarred ones go to the trash; a job that ends with nothing accepted or starred
/// trashes its inputs too. A `fetched` job becomes `closed`; a terminal job only gets
/// `resolved_at`. Resolving twice is a no-op.
pub fn resolve_job(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
    reason: ResolveReason,
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
    let label = match reason {
        ResolveReason::Dismiss => "dismissed",
        ResolveReason::NextSubmit => "resolved by the next submit",
    };
    let done = next.clone();
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        for c in m.candidates.iter_mut().filter(|c| c.job_id == job_id) {
            if c.status == CandidateStatus::New {
                c.status = CandidateStatus::Dismissed;
            }
        }
        trash_unkept(ctx, m, job_id, label);
        trash_inputs_if_unkept(ctx, m, &done);
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

/// `set_star` (FR-RGN-28): starred candidates are kept forever. Starring a trashed one brings
/// its file back; unstarring a leftover of a resolved job sends it to the trash.
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
    let resolved = p
        .jobs
        .get(&c.job_id)
        .is_some_and(|j| j.resolved_at.is_some());
    commit(ctx, install_id, p, |ctx, _, m, _| {
        m.candidates[idx].starred = starred;
        if starred {
            untrash_candidate(m, idx);
        } else if resolved {
            let job_id = m.candidates[idx].job_id;
            trash_unkept(ctx, m, job_id, "unstarred");
        }
        Ok(())
    })?;
    Ok(p.manifest.candidates[idx].clone())
}

/// `discard_candidate` (FR-RGN-28): `discarded`, its file to the trash. An explicit discard also
/// clears the star. An accepted candidate can't be discarded.
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
    commit(ctx, install_id, p, |ctx, _, m, _| {
        let c = &mut m.candidates[idx];
        c.status = CandidateStatus::Discarded;
        c.starred = false;
        if c.trash_id.is_none() {
            let file = c.file.clone();
            let tid = trash_files(ctx, m, TrashKind::Candidates, vec![file], "discarded");
            m.candidates[idx].trash_id = Some(tid);
        }
        Ok(())
    })?;
    Ok(p.manifest.candidates[idx].clone())
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

/// The newest job (by creation), which is the only one a re-pick can use (R32).
fn newest_job(p: &OpenProject) -> Option<&JobFile> {
    p.jobs.values().max_by_key(|j| (j.created_at, j.id))
}

/// `accept_candidate` (FR-PRJ-31, FR-RGN-27, R4): one commit with the baked patch layer, the
/// candidate statuses, the `accept` revision and the rest of the job's unstarred candidates to
/// the trash; then `job.json` `closed`. Also a **re-pick** (R32, FR-RGN-36): a candidate of the
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
        let l: String = job.final_prompt.chars().take(40).collect();
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
            trash_unkept(ctx, m, job_id, "not accepted");
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

#[cfg(test)]
pub(crate) mod tests;
