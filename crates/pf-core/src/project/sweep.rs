//! Open-time recovery (FR-PRJ-20, 24, 28, 29; schema §3): temp files, the job and revision
//! indexes, roll-forward of commit follow-ups, trash file moves, and the orphan sweep of
//! `.pforge/`. User-owned folders, and files the app didn't name, are never swept (FR-PRJ-47).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde_json::json;
use ulid::Ulid;

use super::{OpenProject, RevisionRefs, Warning, commit, read_json, write_job};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::{JOB_FILE, JobFile, JobState};
use crate::library;
use crate::manifest::*;
use crate::naming;
use crate::settings::Settings;

/// Deletes temp files in the app-owned parts of a project (FR-PRJ-20).
pub(super) fn remove_temp_files(dir: &Path) {
    fsx::remove_temp_files_shallow(dir);
    for sub in [PFORGE_DIR, WORKING_DIR, RESULTS_DIR, EXPORTS_DIR] {
        fsx::remove_temp_files(&dir.join(sub));
    }
}

fn read_dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn load_jobs(dir: &Path) -> BTreeMap<Ulid, JobFile> {
    let mut jobs = BTreeMap::new();
    for name in read_dir_names(&dir.join(JOBS_DIR)) {
        let path = dir.join(JOBS_DIR).join(&name).join(JOB_FILE);
        if !path.is_file() {
            continue;
        }
        match read_json::<JobFile>(&path) {
            Ok(job) => {
                jobs.insert(job.id, job);
            }
            Err(e) => tracing::warn!(error = %e, "job file unreadable; skipped"),
        }
    }
    jobs
}

fn load_revisions(dir: &Path) -> Vec<RevisionFile> {
    let mut revs = vec![];
    for name in read_dir_names(&dir.join(REVISIONS_DIR)) {
        if !name.ends_with(".json") || name.contains(fsx::TEMP_MARKER) {
            continue;
        }
        match read_json::<RevisionFile>(&dir.join(REVISIONS_DIR).join(&name)) {
            Ok(rev) => revs.push(rev),
            Err(e) => tracing::warn!(error = %e, "revision file unreadable; skipped"),
        }
    }
    revs.sort_by_key(|r| (r.seq, r.created_at, r.id));
    revs
}

/// Loads the job files (authoritative, R2) and the revision files, and rebuilds the manifest's
/// `jobs[]` and `revisions[]` indexes from them in memory (written at the next commit).
pub(super) fn load_history(p: &mut OpenProject) -> CoreResult<()> {
    p.jobs = load_jobs(&p.dir);
    let old_rows: BTreeMap<Ulid, Extra> = p
        .manifest
        .jobs
        .iter()
        .map(|j| (j.id, j.extra.clone()))
        .collect();
    let mut rows: Vec<JobIndex> = p
        .jobs
        .values()
        .map(|j| JobIndex {
            id: j.id,
            state: j.state,
            iteration: j.iteration.as_ref().map(|i| i.n),
            extra: old_rows.get(&j.id).cloned().unwrap_or_default(),
        })
        .collect();
    // A job whose file isn't here (yet) keeps its index row.
    for old in &p.manifest.jobs {
        if !p.jobs.contains_key(&old.id) {
            rows.push(old.clone());
        }
    }
    p.manifest.jobs = rows;

    let revs = load_revisions(&p.dir);
    let mut refs = RevisionRefs::default();
    let mut entries: Vec<RevisionEntry> = revs.iter().map(|r| r.entry()).collect();
    for r in &revs {
        refs.add(r);
    }
    for old in &p.manifest.revisions {
        if !entries.iter().any(|e| e.id == old.id) {
            entries.push(old.clone());
        }
    }
    entries.sort_by_key(|e| (e.seq, e.created_at, e.id));
    p.manifest.revisions = entries;
    p.refs = refs;
    Ok(())
}

/// Roll-forward and sweep on open (FR-PRJ-20, 29, schema §3). Nothing is written in a
/// read-only project.
pub(super) fn on_open(ctx: &Context, install_id: Ulid, p: &mut OpenProject) -> CoreResult<()> {
    let mut lost = vec![];
    if p.read_only.is_none() {
        let now = ctx.now();
        let ids: Vec<Ulid> = p.jobs.keys().copied().collect();
        for id in ids {
            let Some(job) = p.jobs.get(&id) else { continue };
            let mut next = job.clone();
            let mine: Vec<&Candidate> = p
                .manifest
                .candidates
                .iter()
                .filter(|c| c.job_id == id)
                .collect();
            let target = match job.state {
                // FR-PRJ-29: no handle was saved before the crash.
                JobState::Requested => Some(JobState::Lost),
                // Candidates committed, the `fetched` follow-up wasn't.
                JobState::Succeeded if !mine.is_empty() => Some(JobState::Fetched),
                _ => None,
            };
            if let Some(t) = target {
                if t == JobState::Fetched {
                    next.candidate_ids = mine.iter().map(|c| c.id).collect();
                }
                let _ = next.transition(t, now);
            }
            // Accepted, or nothing left to compare, but the `closed` follow-up wasn't written.
            if next.state == JobState::Fetched
                && !mine.is_empty()
                && (mine.iter().any(|c| c.status == CandidateStatus::Accepted)
                    || mine.iter().all(|c| c.status != CandidateStatus::New))
            {
                let _ = next.transition(JobState::Closed, now);
                next.resolved_at.get_or_insert(now);
            }
            if next != *job {
                if job.state == JobState::Requested {
                    lost.push(id);
                }
                write_job(ctx, p, &next)?;
                if let Some(row) = p.manifest.jobs.iter_mut().find(|r| r.id == id) {
                    row.state = next.state;
                }
                p.jobs.insert(id, next);
            }
        }
        apply_file_moves(p);
        let orphans = find_orphans(p);
        if !orphans.is_empty() {
            commit(ctx, install_id, p, |ctx, _, m, _| {
                super::store::trash_files(ctx, m, TrashKind::Orphan, orphans, "orphan");
                Ok(())
            })?;
        }
        // FR-PRJ-32: items older than 30 days are deleted. Not fatal: retried at the next open.
        if let Err(e) = super::retention::purge_expired(ctx, install_id, p) {
            tracing::warn!(error = %e, "trash purge failed; retried at the next open");
        }
    }
    if !lost.is_empty() {
        p.warnings.push(Warning {
            code: "lost",
            message: format!(
                "{} job(s) were interrupted before the provider accepted them.",
                lost.len()
            ),
            detail: json!({ "job_ids": lost }),
        });
    }
    let resumable: Vec<serde_json::Value> = p
        .jobs
        .values()
        .filter(|j| !j.state.is_terminal())
        .map(|j| json!({ "job_id": j.id, "state": j.state }))
        .collect();
    if !resumable.is_empty() {
        p.warnings.push(Warning {
            code: "resumable",
            message: format!("{} job(s) can resume.", resumable.len()),
            detail: json!({ "jobs": resumable }),
        });
    }
    Ok(())
}

/// Brings files in line with the committed manifest (schema §3, §4.8): files of trash items move
/// into `.pforge/trash/<id>/…`, and a live candidate or job input missing from its place is moved
/// back from the trash (a star, restore or re-pick). Iteration folders left empty are removed
/// (FR-PRJ-31). Failures are logged and retried at the next commit or open.
pub(crate) fn apply_file_moves(p: &OpenProject) {
    let m = &p.manifest;
    let mut trashed: HashSet<&str> = HashSet::new();
    for item in &m.trash {
        for from in &item.files {
            trashed.insert(from);
            let (src, dst) = (p.path(from), p.path(&item.trashed_path(from)));
            if src.exists() && !dst.exists() {
                match fsx::move_file(&src, &dst) {
                    Ok(()) => remove_empty_result_dir(p, from),
                    Err(e) => tracing::warn!(error = %e, "trash move failed; retried later"),
                }
            }
        }
    }
    let mut live: Vec<String> = m
        .candidates
        .iter()
        .filter(|c| c.trash_id.is_none())
        .map(|c| c.file.clone())
        .collect();
    live.extend(p.jobs.values().flat_map(|j| j.input_files()));
    for rel in live {
        if trashed.contains(rel.as_str()) || p.path(&rel).exists() {
            continue;
        }
        let found = read_dir_names(&p.path(TRASH_DIR))
            .into_iter()
            .map(|id| p.path(&format!("{TRASH_DIR}/{id}/{rel}")))
            .find(|c| c.is_file());
        if let Some(src) = found
            && let Err(e) = fsx::move_file(&src, &p.path(&rel))
        {
            tracing::warn!(error = %e, "restore from the trash failed; retried later");
        }
    }
}

fn remove_empty_result_dir(p: &OpenProject, rel: &str) {
    if let Some((dir, _)) = rel.rsplit_once('/')
        && dir.starts_with(&format!("{RESULTS_DIR}/"))
    {
        fsx::remove_dir_if_empty(&p.path(dir));
    }
}

/// Files in `.pforge/` that no manifest field, revision or job file references (FR-PRJ-20).
fn find_orphans(p: &OpenProject) -> Vec<String> {
    let m = &p.manifest;
    let mut layers: HashSet<Ulid> = p.refs.layers.clone();
    layers.extend(m.layers.iter().map(|l| l.id));
    let mut masks: HashSet<Ulid> = p.refs.masks.clone();
    masks.extend(m.masks.iter().map(|k| k.id));
    masks.extend(m.draft.as_ref().and_then(|d| d.mask_id));
    masks.extend(m.layers.iter().filter_map(|l| l.mask_id));
    masks.extend(p.jobs.values().filter_map(|j| j.mask_id));
    let in_trash: HashSet<&str> = m
        .trash
        .iter()
        .flat_map(|i| i.files.iter().map(String::as_str))
        .collect();
    let id_png = |name: &str| {
        name.strip_suffix(".png")
            .and_then(|s| s.parse::<Ulid>().ok())
    };

    let mut orphans = vec![];
    for (dir, keep) in [(LAYERS_DIR, &layers), (MASKS_DIR, &masks)] {
        for name in read_dir_names(&p.path(dir)) {
            let rel = format!("{dir}/{name}");
            if p.path(&rel).is_file() && !id_png(&name).is_some_and(|id| keep.contains(&id)) {
                orphans.push(rel);
            }
        }
    }
    for name in read_dir_names(&p.path(JOBS_DIR)) {
        let job_dir = format!("{JOBS_DIR}/{name}");
        if p.path(&format!("{job_dir}/{JOB_FILE}")).exists() {
            // A known job folder, or one whose file is still syncing: never swept.
            continue;
        }
        for file in read_dir_names(&p.path(&job_dir)) {
            let rel = format!("{job_dir}/{file}");
            if p.path(&rel).is_file() {
                orphans.push(rel);
            }
        }
    }
    for name in read_dir_names(&p.path(PFORGE_DIR)) {
        let rel = format!("{PFORGE_DIR}/{name}");
        let known = rel == THUMB_FILE
            || naming::is_conflict_copy(&name, "thumb.jpg")
            || (name.starts_with("project.v") && name.ends_with(".json"));
        if !known && p.path(&rel).is_file() {
            orphans.push(rel);
        }
    }
    orphans.retain(|o| !in_trash.contains(o.as_str()));
    orphans
}

/// FR-PRJ-24: a manifest rebuilt from the newest revision, the revision files and the job files.
/// `None` if the folder has no revisions (then it isn't a project).
pub(super) fn rebuild_manifest(
    ctx: &Context,
    settings: &Settings,
    dir: &Path,
) -> CoreResult<Option<Manifest>> {
    let revs = load_revisions(dir);
    let Some(newest) = revs.last() else {
        return Ok(None);
    };
    let original = rebuild_original(ctx, dir)?;
    let jobs = load_jobs(dir);
    let mut candidates = vec![];
    for job in jobs.values() {
        let Some(it) = &job.iteration else { continue };
        let files = read_dir_names(&dir.join(&it.dir));
        for (index, id) in job.candidate_ids.iter().enumerate() {
            let prefix = format!("{:02}.", index + 1);
            let Some(name) = files.iter().find(|f| f.starts_with(&prefix)) else {
                continue;
            };
            let rel = format!("{}/{name}", it.dir);
            let probe = pf_imaging::probe_file(&dir.join(&rel)).ok();
            let accepted = newest.layers.iter().any(|l| l.candidate_id == Some(*id));
            candidates.push(Candidate {
                id: *id,
                job_id: job.id,
                index: index as u32,
                file: rel,
                format: probe.map_or("unknown".into(), |i| i.format.as_str().into()),
                width: probe.map_or(0, |i| i.width),
                height: probe.map_or(0, |i| i.height),
                seed: None,
                provider_meta: serde_json::Value::Null,
                status: match (accepted, probe) {
                    (true, _) => CandidateStatus::Accepted,
                    (false, Some(_)) => CandidateStatus::New,
                    (false, None) => CandidateStatus::Invalid,
                },
                starred: false,
                trash_id: None,
                extra: Extra::new(),
            });
        }
    }
    let masks = rebuild_masks(dir, newest, &jobs);
    let kind = library::kind_of_folder(dir).unwrap_or(Kind::Map);
    let first = &revs[0];
    Ok(Some(Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        project_id: ctx.new_id(),
        kind,
        created_at: first.created_at,
        commit: Commit {
            seq: 0,
            install_id: settings.install_id,
            at: ctx.now(),
        },
        original,
        layers: newest.layers.clone(),
        masks,
        draft: newest.draft.clone(),
        jobs: vec![],
        candidates,
        revisions: revs.iter().map(|r| r.entry()).collect(),
        head_revision: newest.id,
        trash: vec![],
        export: None,
        extra: Extra::new(),
    }))
}

fn rebuild_original(ctx: &Context, dir: &Path) -> CoreResult<Original> {
    let working = dir.join(WORKING_DIR);
    let name = read_dir_names(&working)
        .into_iter()
        .find(|n| !n.contains(fsx::TEMP_MARKER) && working.join(n).is_file())
        .ok_or_else(|| {
            CoreError::new(
                ErrorCode::Corrupt,
                "project.json is missing and working/ holds no original to rebuild it from",
            )
        })?;
    let path = working.join(&name);
    library::hydrate_if_needed(ctx, &path)?;
    let info = pf_imaging::probe_file_complete(&path).map_err(super::probe_error)?;
    let meta = std::fs::metadata(&path).map_err(|e| CoreError::io("reading the original", &e))?;
    Ok(Original {
        file: format!("{WORKING_DIR}/{name}"),
        content_hash: fsx::hash_file(&path)
            .map_err(|e| CoreError::io("hashing the original", &e))?,
        size: meta.len(),
        mtime: super::mtime_of(&meta),
        width: info.width,
        height: info.height,
        format: info.format.as_str().into(),
        extra: Extra::new(),
    })
}

/// Masks whose rect is known: the draft's (its region) and each job's (its region).
fn rebuild_masks(dir: &Path, newest: &RevisionFile, jobs: &BTreeMap<Ulid, JobFile>) -> Vec<Mask> {
    let rect_of = |region: &serde_json::Value| {
        serde_json::from_value::<Rect>(region.get("inner")?.clone()).ok()
    };
    let mut known: BTreeMap<Ulid, Rect> = BTreeMap::new();
    if let Some(d) = &newest.draft
        && let (Some(id), Some(r)) = (d.mask_id, d.region.as_ref().and_then(rect_of))
    {
        known.insert(id, r);
    }
    for j in jobs.values() {
        if let (Some(id), Some(r)) = (j.mask_id, rect_of(&j.region)) {
            known.insert(id, r);
        }
    }
    known
        .into_iter()
        .filter(|(id, _)| dir.join(mask_file(*id)).is_file())
        .map(|(id, rect)| Mask {
            id,
            file: mask_file(id),
            rect,
            created_at: newest.created_at,
            extra: Extra::new(),
        })
        .collect()
}
