use serde_json::Value;

use super::*;
use crate::context::{FaultAction, Faults};
use crate::project::store::tests::{S, setup};
use crate::project::store::{JobUpdate, NewCandidate, add_candidates, update_job};
use crate::project::{DraftInput, put_draft, retry_pending, spool};
use crate::test_support::*;

fn other_pc() -> String {
    Ulid::from_parts(2, 7).to_string()
}

fn draft(s: &mut S, prompt: &str) -> CoreResult<u64> {
    put_draft(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        DraftInput {
            final_prompt: prompt.into(),
            ..Default::default()
        },
        None,
    )
}

fn read_value(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn write_value(path: &std::path::Path, v: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(v).unwrap()).unwrap();
}

/// Walkthrough step 7: `project.json` copied to `project-OTHERPC.json` by "another machine",
/// with its own draft.
fn make_copy(s: &S, prompt: &str) -> std::path::PathBuf {
    let mut v = read_value(&s.dir.join("project.json"));
    v["draft"]["final_prompt"] = Value::from(prompt);
    v["commit"]["install_id"] = Value::from(other_pc());
    v["commit"]["seq"] = Value::from(v["commit"]["seq"].as_u64().unwrap() + 3);
    let copy = s.dir.join("project-OTHERPC.json");
    write_value(&copy, &v);
    copy
}

fn events(s: &S, name: &str) -> usize {
    s.e.events.names().iter().filter(|n| *n == name).count()
}

fn prompt(m: &Manifest) -> String {
    m.draft
        .as_ref()
        .map(|d| d.final_prompt.clone())
        .unwrap_or_default()
}

#[test]
fn fr_prj_34_fr_prj_36_copy_shows_both_and_keep_this_makes_a_conflict_revision() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    let copy = make_copy(&s, "theirs");
    s.reopen();
    assert!(
        matches!(s.p.conflict, Some(ConflictSource::Copy(ref c)) if c == "project-OTHERPC.json")
    );
    assert!(s.p.warnings.iter().any(|w| w.code == "conflict"));
    assert_eq!(events(&s, "project:conflict"), 1);
    assert_eq!(
        draft(&mut s, "blocked").unwrap_err().code,
        ErrorCode::Conflict
    );

    let v = conflict_get(&s.p, s.e.install()).unwrap();
    assert_eq!(v.source, "copy");
    assert_eq!(v.this.file.as_deref(), Some("project.json"));
    assert_eq!(v.other.file.as_deref(), Some("project-OTHERPC.json"));
    assert_eq!((v.this.machine, v.other.machine), ("this", "other"));
    assert!(v.this.can_keep && v.other.can_keep);
    assert_eq!(v.this.layers.len(), 1);
    assert_eq!(v.this.newest_revision.as_ref().unwrap().seq, 0);
    assert!(v.other.committed_at.is_some());

    let rev = resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
    assert_eq!((rev.reason, rev.seq), (RevisionReason::Conflict, 1));
    let file: RevisionFile = read_json(&s.dir.join(revision_file(rev.id))).unwrap();
    assert_eq!(
        file.draft.unwrap().final_prompt,
        "theirs",
        "the other version is a revision"
    );
    assert_eq!(prompt(&s.p.manifest), "mine");
    assert_ne!(
        s.p.manifest.head_revision, rev.id,
        "the kept version stays the head"
    );
    assert!(!copy.exists(), "the copy went to the trash");
    let item =
        s.p.manifest
            .trash
            .iter()
            .find(|t| t.kind == TrashKind::ConflictCopy)
            .unwrap();
    assert!(
        s.dir
            .join(item.trashed_path("project-OTHERPC.json"))
            .is_file()
    );
    assert!(s.p.conflict.is_none());
    assert!(!s.p.warnings.iter().any(|w| w.code == "conflict"));
    draft(&mut s, "writable again").unwrap();
    s.reopen();
    assert!(s.p.conflict.is_none());
    assert_eq!(
        conflict_get(&s.p, s.e.install()).unwrap_err().code,
        ErrorCode::NoConflict
    );
}

#[test]
fn fr_prj_36_keep_other_makes_the_copy_current_and_this_a_revision() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    make_copy(&s, "theirs");
    s.reopen();
    let rev = resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::Other).unwrap();
    assert_eq!(prompt(&s.p.manifest), "theirs");
    let file: RevisionFile = read_json(&s.dir.join(revision_file(rev.id))).unwrap();
    assert_eq!(file.draft.unwrap().final_prompt, "mine");
    assert_eq!(file.label.as_deref(), Some("Not kept (this machine)"));
    s.reopen();
    assert_eq!(prompt(&s.p.manifest), "theirs");
    assert!(s.p.manifest.revisions.iter().any(|r| r.id == rev.id));
}

#[test]
fn fr_prj_36_a_version_with_missing_files_cant_be_kept_yet() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    let copy = make_copy(&s, "theirs");
    let mut v = read_value(&copy);
    let ghost = Ulid::from_parts(1, 99);
    let mut layer = Layer::original(ghost, 40, 30);
    layer.kind = LayerKind::Patch;
    layer.file = Some(layer_file(ghost));
    v["layers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(&layer).unwrap());
    write_value(&copy, &v);
    s.reopen();
    let view = conflict_get(&s.p, s.e.install()).unwrap();
    assert!(view.other.readable);
    assert!(!view.other.can_keep);
    assert_eq!(view.other.missing_files, vec![layer_file(ghost)]);
    assert_eq!(view.other.layers.len(), 2);
    let err = resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::Other).unwrap_err();
    assert_eq!(err.code, ErrorCode::FilesMissing);
    assert_eq!(err.detail["files"][0], layer_file(ghost));
    assert!(s.p.conflict.is_some(), "still in conflict");
    resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
}

#[test]
fn fr_prj_35_seq_mismatch_refuses_the_write_and_fires_conflict() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    // Another machine committed meanwhile (synced in over our file).
    let mut v = read_value(&s.dir.join("project.json"));
    v["commit"]["seq"] = Value::from(40);
    v["commit"]["install_id"] = Value::from(other_pc());
    v["draft"]["final_prompt"] = Value::from("theirs");
    write_value(&s.dir.join("project.json"), &v);

    let err = draft(&mut s, "next").unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(s.p.conflict, Some(ConflictSource::Seq));
    assert_eq!(events(&s, "project:conflict"), 1);
    assert_eq!(
        read_value(&s.dir.join("project.json"))["commit"]["seq"],
        40,
        "refused"
    );
    let view = conflict_get(&s.p, s.e.install()).unwrap();
    assert_eq!(view.source, "seq");
    assert_eq!(view.this.file, None, "this machine's state in memory");
    assert_eq!(view.other.seq, Some(40));
    resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
    let disk = read_value(&s.dir.join("project.json"));
    assert_eq!(disk["commit"]["seq"], 41);
    assert_eq!(disk["draft"]["final_prompt"], "mine");
}

#[test]
fn fr_prj_40_candidates_spooled_during_a_conflict_replay_after_the_resolve() {
    let mut s = setup();
    let job = s.job("moss");
    for st in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
        s.to(job.id, st);
    }
    make_copy(&s, "theirs");
    let cands = vec![NewCandidate {
        bytes: rgba_png(16, 14, [9, 9, 9, 255]),
        seed: Some(5),
        provider_meta: serde_json::json!({"paid": true}),
        invalid: false,
    }];
    let err = add_candidates(&s.e.ctx, s.e.install(), &mut s.p, job.id, cands).unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(err.detail["spooled"], true);
    let spooled_id: Ulid = err.detail["candidate_ids"][0]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let pid = s.p.manifest.project_id;
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 1);
    assert!(s.p.manifest.candidates.is_empty());

    resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
    assert_eq!(
        spool::pending_count(&s.e.ctx, pid),
        0,
        "replayed and deleted"
    );
    let c = s.cand(spooled_id);
    assert_eq!(c.seed, Some(5));
    assert!(s.dir.join(&c.file).is_file());
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
    assert!(!spool::spool_dir(&s.e.ctx, pid).exists());
}

#[test]
fn fr_prj_40_spool_survives_a_crash_and_replays_on_open() {
    let mut s = setup();
    let job = s.job("moss");
    s.to(job.id, JobState::Submitted);
    // Read-only while the job runs: updates go to the spool, memory keeps the state.
    s.p.read_only = Some(crate::project::ReadOnlyReason::OriginalChanged);
    let err = update_job(
        &s.e.ctx,
        &mut s.p,
        job.id,
        JobUpdate {
            state: Some(JobState::Running),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        (err.code, &err.detail["spooled"]),
        (ErrorCode::ReadOnly, &Value::Bool(true))
    );
    assert_eq!(s.p.jobs[&job.id].state, JobState::Running);
    let err = update_job(
        &s.e.ctx,
        &mut s.p,
        job.id,
        JobUpdate {
            state: Some(JobState::Succeeded),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::ReadOnly);
    let err = add_candidates(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        vec![NewCandidate {
            bytes: rgba_png(16, 14, [1, 1, 1, 255]),
            ..Default::default()
        }],
    )
    .unwrap_err();
    assert_eq!(err.detail["spooled"], true);
    let pid = s.p.manifest.project_id;
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 3);
    let on_disk: crate::job::JobFile = read_json(&s.dir.join(job.dir()).join("job.json")).unwrap();
    assert_eq!(on_disk.state, JobState::Submitted, "nothing written");

    // "Crash": the in-memory state is gone; the next (writable) open replays in order.
    s.reopen();
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 0);
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
    assert_eq!(s.p.manifest.candidates.len(), 1);
    assert_eq!(s.p.manifest.jobs[0].state, JobState::Fetched);
}

#[test]
fn fr_prj_21_fr_prj_40_locked_candidates_spool_and_the_retry_replays_them() {
    let mut s = setup();
    let job = s.job("moss");
    for st in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
        s.to(job.id, st);
    }
    s.e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Locked).unwrap();
    let err = add_candidates(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        vec![NewCandidate {
            bytes: rgba_png(16, 14, [1, 1, 1, 255]),
            ..Default::default()
        }],
    )
    .unwrap_err();
    assert_eq!(
        (err.code, &err.detail["spooled"]),
        (ErrorCode::Locked, &Value::Bool(true))
    );
    assert!(events(&s, "project:save-failed") >= 1);
    assert!(
        retry_pending(&s.e.ctx, s.e.install(), &mut s.p),
        "still locked"
    );
    s.e.ctx.faults = Faults::none();
    assert!(!retry_pending(&s.e.ctx, s.e.install(), &mut s.p));
    assert_eq!(s.p.manifest.candidates.len(), 1);
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
}

#[test]
fn fr_prj_34_oq_prj_10_a_job_copy_with_the_later_state_wins() {
    let mut s = setup();
    let a = s.job("one");
    s.to(a.id, JobState::Submitted);
    let b = s.job("two");
    s.to(b.id, JobState::Submitted);
    s.to(b.id, JobState::Running);
    // a: the copy is later (running) → it wins. b: the copy is earlier → job.json stays.
    for (job, state) in [(a.id, "running"), (b.id, "submitted")] {
        let dir = s.dir.join(job_dir(job));
        let mut v = read_value(&dir.join("job.json"));
        v["state"] = Value::from(state);
        v["provider"] = serde_json::json!({"handle": "h-other"});
        write_value(&dir.join("job-OTHERPC.json"), &v);
    }
    s.reopen();
    assert_eq!(s.p.jobs[&a.id].state, JobState::Running);
    assert_eq!(s.p.jobs[&a.id].provider["handle"], "h-other");
    let disk_a: crate::job::JobFile =
        read_json(&s.dir.join(job_dir(a.id)).join("job.json")).unwrap();
    assert_eq!(disk_a.state, JobState::Running);
    assert_eq!(s.p.jobs[&b.id].state, JobState::Running);
    let trashed: Vec<&String> =
        s.p.manifest
            .trash
            .iter()
            .filter(|t| t.kind == TrashKind::ConflictCopy)
            .flat_map(|t| &t.files)
            .collect();
    assert_eq!(trashed.len(), 2, "both copies trashed: {trashed:?}");
    for t in
        s.p.manifest
            .trash
            .iter()
            .filter(|t| t.kind == TrashKind::ConflictCopy)
    {
        for f in &t.files {
            assert!(!s.dir.join(f).exists());
            assert!(s.dir.join(t.trashed_path(f)).is_file());
        }
    }
    // The trashed copy of `a` holds the loser (submitted).
    let item =
        s.p.manifest
            .trash
            .iter()
            .find(|t| t.kind == TrashKind::ConflictCopy)
            .unwrap();
    let a_copy = format!("{}/job-OTHERPC.json", job_dir(a.id));
    if let Some(f) = item.files.iter().find(|f| **f == a_copy) {
        let loser: crate::job::JobFile = read_json(&s.dir.join(item.trashed_path(f))).unwrap();
        assert_eq!(loser.state, JobState::Submitted);
    }
    assert!(s.p.conflict.is_none(), "job copies never need the user");
}

#[test]
fn fr_prj_34_a_thumbnail_copy_goes_to_the_trash() {
    let mut s = setup();
    std::fs::write(s.dir.join(".pforge/thumb-LAPTOP.jpg"), b"jpeg").unwrap();
    s.reopen();
    assert!(!s.dir.join(".pforge/thumb-LAPTOP.jpg").exists());
    assert!(
        s.p.manifest
            .trash
            .iter()
            .any(|t| t.kind == TrashKind::ConflictCopy
                && t.files == vec![".pforge/thumb-LAPTOP.jpg".to_owned()])
    );
}
