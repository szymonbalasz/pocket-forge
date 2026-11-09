//! The **pick commit** (GEN FR-GEN-27 … 30, OQ-GEN-03/04): a generate candidate becomes the
//! project's base image in one PRJ commit. The candidate file is copied write-once to
//! `working/NNN <slug> NN.<ext>` (before the manifest, schema §3); the stack becomes
//! `[base layer]`; the candidate is `accepted`, its job `closed` and the job's other `new`
//! candidates `dismissed` (kept, #56); one `accept` revision labelled `New: <typed prompt>`.
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

/// `pick_base` (FR-GEN-27, 28, 30): the pick commit behind `gen_candidate_pick`. Any valid,
/// non-discarded candidate of a `fetched` job (the first pick: the job is closed) or of a
/// `closed` one (a re-pick from any iteration, no transition, no spend) can be picked; a file
/// moved to the trash by a sweep is restored in the same commit. Fails with `invalid_candidate`
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
            job_id,
            candidate_id,
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
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        m.layers = vec![layer];
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
