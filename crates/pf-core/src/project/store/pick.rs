//! The **pick commit** (GEN FR-GEN-27 … 30, OQ-GEN-03/04): a generate candidate becomes the
//! project's base image in one PRJ commit. The candidate file is copied write-once to
//! `working/NNN <slug> NN.<ext>` (before the manifest, schema §3); the stack becomes
//! `[base layer]`; the candidate is `accepted`, its job `closed` and the job's other `new`
//! candidates `dismissed` (kept, #56); the previous base candidate goes back to `dismissed`
//! (R84); one `accept` revision labelled `New: <typed prompt>`.
//! The original file, if any, is untouched. Undo = `project_restore_revision(parent)` (FR-GEN-29).

use serde::Serialize;
use ulid::Ulid;

use super::{candidate_index, job, restorable, untrash_candidate};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::JobState;
use crate::manifest::*;
use crate::naming;
use crate::project::{OpenProject, commit, push_revision};

/// What a pick returns: `{layer_id, revision_id}` (GEN §9) plus what the UI's notice needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaseCommit {
    pub layer_id: Ulid,
    pub revision_id: Ulid,
    /// The base image's project path, `working/NNN <slug> NN.<ext>`.
    pub file: String,
    /// Patch layers of the previous stack, kept in history only (FR-GEN-28): "N patch layers
    /// kept in history (undo restores them)".
    pub patches_dropped: usize,
}

/// Longest part of the typed prompt in the revision label (FR-GEN-27).
const LABEL_PROMPT_CHARS: usize = 40;

/// The revision label of a pick: `New: ` + the first 40 chars of the typed prompt.
pub fn pick_label(typed_prompt: &str) -> String {
    let head: String = typed_prompt
        .trim()
        .chars()
        .take(LABEL_PROMPT_CHARS)
        .collect();
    format!("New: {}", head.trim_end())
}

/// `working/NNN <slug> NN.<ext>` for a candidate at `results/NNN <slug>/NN.<ext>`.
pub fn working_name(candidate_file: &str) -> String {
    let rel = candidate_file
        .strip_prefix(&format!("{RESULTS_DIR}/"))
        .unwrap_or(candidate_file);
    format!("{WORKING_DIR}/{}", rel.replace('/', " "))
}

fn invalid_candidate(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidCandidate, message).with_field("candidate_id")
}

/// Copies the candidate into `working/` write-once. A file already there with the same hash (a
/// crash between the copy and the manifest, or a re-pick) is reused; one with other bytes is
/// never overwritten: the copy takes a free ` (2)` name. Returns the project path and hash.
fn place_base_file(
    ctx: &Context,
    p: &OpenProject,
    source: &std::path::Path,
    wanted: &str,
) -> CoreResult<(String, String)> {
    let target = p.path(wanted);
    if target.is_file() {
        let src_hash =
            fsx::hash_file(source).map_err(|e| CoreError::io("reading the candidate", &e))?;
        let have = fsx::hash_file(&target).map_err(|e| CoreError::io("reading working/", &e))?;
        if have == src_hash {
            return Ok((wanted.to_owned(), have));
        }
    }
    let (stem, ext) = match wanted.rsplit_once('.') {
        Some((s, e)) if !s.ends_with('/') => (s.to_owned(), format!(".{e}")),
        _ => (wanted.to_owned(), String::new()),
    };
    let free = naming::first_free(&stem, |c| p.path(&format!("{c}{ext}")).exists());
    let rel = format!("{free}{ext}");
    let target = p.path(&rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::io(format!("creating {}", parent.display()), &e))?;
    }
    let (hash, _) = fsx::copy_atomic_new(ctx, source, &target)?;
    Ok((rel, hash))
}

/// R97: a new bottom image is never committed while a job runs (a region edit job would come
/// back for another base image). `job_active`, with the running job's id.
fn require_no_active_job(p: &OpenProject) -> CoreResult<()> {
    if let Some(active) = p
        .jobs
        .values()
        .find(|j| !j.state.is_terminal() && j.state != JobState::Fetched)
    {
        return Err(CoreError::new(
            ErrorCode::JobActive,
            "a job is running in this project; pick when it has finished",
        )
        .with_detail(serde_json::json!({ "job_id": active.id })));
    }
    Ok(())
}

/// `pick_base` (FR-GEN-27, 28, 30): the pick commit behind `gen_candidate_pick`. Any valid,
/// non-discarded candidate of a `fetched` job (the first pick: the job is closed) or of a
/// `closed` one (a re-pick from any iteration, no transition, no spend) can be picked; a file
/// moved to the trash by a sweep is restored in the same commit. Fails with `job_active` while any
/// job of the project is active (R97), `invalid_candidate`
/// (another job's, `invalid`, `discarded`), `not_found` (purged), `invalid_input` (a job with
/// no candidates to pick) and the commit errors (`read_only`, `locked`, `conflict`). The job's
/// `operation` is checked by the command (slice 4, `wrong_operation`).
pub fn pick_base(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    job_id: Ulid,
    candidate_id: Ulid,
) -> CoreResult<BaseCommit> {
    p.require_writable()?;
    require_no_active_job(p)?;
    let job = job(p, job_id)?.clone();
    let idx = candidate_index(&p.manifest, candidate_id)?;
    let cand = p.manifest.candidates[idx].clone();
    if cand.job_id != job_id {
        return Err(invalid_candidate("the candidate belongs to another job"));
    }
    match cand.status {
        CandidateStatus::Invalid => return Err(invalid_candidate("the candidate won't decode")),
        CandidateStatus::Discarded => {
            return Err(invalid_candidate("the candidate was discarded"));
        }
        _ => {}
    }
    let first = match job.state {
        JobState::Fetched => true,
        JobState::Closed => false,
        other => {
            return Err(CoreError::invalid(
                "job_id",
                format!("a {} job has nothing to pick", other.as_str()),
            ));
        }
    };
    if !restorable(p, &cand) {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            "the candidate's file was purged from the trash",
        ));
    }
    // The bytes now: in `results/`, or still in the trash until this commit moves them back.
    let source = match cand
        .trash_id
        .and_then(|t| p.manifest.trash.iter().find(|i| i.id == t))
    {
        Some(item) if p.path(&item.trashed_path(&cand.file)).is_file() => {
            p.path(&item.trashed_path(&cand.file))
        }
        _ => p.path(&cand.file),
    };
    crate::library::hydrate_if_needed(ctx, &source)?;
    let info = pf_imaging::probe_file(&source).map_err(crate::project::probe_error)?;
    // Write-once file first; a failure after this leaves only a harmless unreferenced copy (§8).
    let (file, content_hash) = place_base_file(ctx, p, &source, &working_name(&cand.file))?;
    let layer_id = ctx.new_id();
    let layer = Layer::base(
        layer_id,
        BaseImage {
            file: file.clone(),
            content_hash,
            width: info.width,
            height: info.height,
            job_id: Some(job_id),
            candidate_id: Some(candidate_id),
        },
    );
    let patches_dropped = p
        .manifest
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Patch)
        .count();
    let label = pick_label(job.typed_prompt());
    let now = ctx.now();
    // R84: only the current base candidate stays `accepted`; earlier picks (base candidates of
    // generate jobs) become ordinary kept results again, so they can be discarded (#56).
    let generate_jobs: std::collections::HashSet<Ulid> = p
        .jobs
        .values()
        .filter(|j| j.is_generate())
        .map(|j| j.id)
        .collect();
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        m.layers = vec![layer];
        for c in m.candidates.iter_mut().filter(|c| {
            c.id != candidate_id
                && c.status == CandidateStatus::Accepted
                && generate_jobs.contains(&c.job_id)
        }) {
            c.status = CandidateStatus::Dismissed;
        }
        m.candidates[idx].status = CandidateStatus::Accepted;
        untrash_candidate(m, idx);
        if first {
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
            .push(push_revision(ctx, m, RevisionReason::Accept, Some(label)));
        Ok(())
    })?;
    Ok(BaseCommit {
        layer_id,
        revision_id: p.manifest.head_revision,
        file,
        patches_dropped,
    })
}

/// The revision label of a drop on Working: `Dropped: ` + the first 40 chars of the file name.
pub fn drop_label(file_name: &str) -> String {
    let head: String = file_name.trim().chars().take(LABEL_PROMPT_CHARS).collect();
    format!("Dropped: {}", head.trim_end())
}

/// `working/<file name>` for an image dropped on Working: the stem under FR-PRJ-44, the
/// extension lower case (the format's when the name has no image extension).
pub fn dropped_working_name(source: &std::path::Path, format: pf_imaging::ImageFormat) -> String {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = if pf_imaging::has_image_extension(source) {
        source
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    } else {
        format.extension().to_owned()
    };
    format!("{WORKING_DIR}/{}.{ext}", naming::sanitise_name(&stem))
}

/// `base_from_file` (FR-PRJ-55, #61): an image file dropped on the Working section becomes the
/// project's base image in one commit, as a pick does (FR-GEN-27, 28): the file is copied
/// write-once to `working/<file name>` (the same bytes already there are reused; other bytes take
/// a free ` (2)` name), the stack becomes `[base layer]` with no job or candidate, every earlier
/// base candidate goes back to `dismissed` (R84), and one `accept` revision is labelled
/// `Dropped: <file name>`. The source is never moved or changed. Fails with `job_active` (R97),
/// `not_found` (no such file), the creation checks (`unsupported`, `animated`, `too_large`,
/// `corrupt`, FR-PRJ-06, R33), `not_available_offline` and the commit errors.
pub fn base_from_file(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    source: &std::path::Path,
) -> CoreResult<BaseCommit> {
    p.require_writable()?;
    require_no_active_job(p)?;
    if !source.is_file() {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            format!("{} is not a file", source.display()),
        )
        .with_field("path"));
    }
    crate::library::hydrate_if_needed(ctx, source)?;
    let info = pf_imaging::probe_file_complete(source).map_err(crate::project::probe_error)?;
    let mut wanted = dropped_working_name(source, info.format);
    // The original's file is never the base image's too (the rebuild tells them apart, sweep).
    let original = p.manifest.original.as_ref().map(|o| o.file.clone());
    if let Some(orig) = original.filter(|o| naming::same_name(o, &wanted)) {
        let (stem, ext) = wanted.rsplit_once('.').unwrap_or((wanted.as_str(), ""));
        let free = naming::first_free(stem, |c| {
            let rel = format!("{c}.{ext}");
            naming::same_name(&rel, &orig) || p.path(&rel).exists()
        });
        wanted = format!("{free}.{ext}");
    }
    let (file, content_hash) = place_base_file(ctx, p, source, &wanted)?;
    let layer_id = ctx.new_id();
    let layer = Layer::base(
        layer_id,
        BaseImage {
            file: file.clone(),
            content_hash,
            width: info.width,
            height: info.height,
            job_id: None,
            candidate_id: None,
        },
    );
    let patches_dropped = p
        .manifest
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Patch)
        .count();
    let label = drop_label(&crate::project::folder_name(source));
    commit(ctx, install_id, p, |ctx, p, m, plan| {
        m.layers = vec![layer];
        statuses_follow_base(p, m);
        plan.revisions
            .push(push_revision(ctx, m, RevisionReason::Accept, Some(label)));
        Ok(())
    })?;
    Ok(BaseCommit {
        layer_id,
        revision_id: p.manifest.head_revision,
        file,
        patches_dropped,
    })
}

/// R97 (R84 across undo/redo): after a restore, among generate candidates only the restored base
/// image's is `accepted`; an earlier base's goes back to `dismissed`, and with no base image none
/// is `accepted`. A restored base candidate in the trash comes back with it while its file can
/// (else it stays as it is: the base image is its own copy in `working/`).
pub(crate) fn statuses_follow_base(p: &OpenProject, m: &mut Manifest) {
    let generate_jobs: std::collections::HashSet<Ulid> = p
        .jobs
        .values()
        .filter(|j| j.is_generate())
        .map(|j| j.id)
        .collect();
    let base = m
        .layers
        .first()
        .filter(|l| l.kind == LayerKind::Base)
        .and_then(|l| l.candidate_id);
    for c in m.candidates.iter_mut().filter(|c| {
        Some(c.id) != base
            && c.status == CandidateStatus::Accepted
            && generate_jobs.contains(&c.job_id)
    }) {
        c.status = CandidateStatus::Dismissed;
    }
    let Some(idx) = base.and_then(|id| m.candidates.iter().position(|c| c.id == id)) else {
        return;
    };
    let c = m.candidates[idx].clone();
    match c.status {
        CandidateStatus::Accepted | CandidateStatus::Invalid => {}
        _ if c.trash_id.is_some() => {
            if restorable(p, &c) {
                m.candidates[idx].status = CandidateStatus::Accepted;
                untrash_candidate(m, idx);
            }
        }
        _ => m.candidates[idx].status = CandidateStatus::Accepted,
    }
}
