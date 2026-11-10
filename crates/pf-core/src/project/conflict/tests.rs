use serde_json::Value;

use super::*;
use crate::context::{FaultAction, Faults};
use crate::project::store::tests::{S, setup};
use crate::project::store::{JobUpdate, NewCandidate, add_candidates, update_job};
use crate::project::{DraftInput, ImageKind, put_draft, retry_pending, spool};
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
    assert!(
        crate::project::has_unsaved(&s.p),
        "the refused draft is kept"
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
    assert_eq!(
        prompt(&s.p.manifest),
        "blocked",
        "this machine's newest draft, typed during the conflict"
    );
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
    assert_eq!(
        disk["draft"]["final_prompt"], "next",
        "the refused save is this machine's version"
    );
}

/// Walk 2 N1, repro B: a prompt typed while the conflict is open is refused, kept in memory
/// (closing asks), waits through the 10 s retry and is saved by Keep this.
#[test]
fn fr_prj_36_fr_prj_21_a_draft_refused_during_a_conflict_is_saved_by_keep_this() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    make_copy(&s, "theirs");
    s.reopen();
    assert!(s.p.conflict.is_some());
    assert_eq!(
        draft(&mut s, "typed during").unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(crate::project::has_unsaved(&s.p), "closing must ask");
    assert!(
        retry_pending(&s.e.ctx, s.e.install(), &mut s.p),
        "still pending"
    );
    assert!(
        crate::project::has_unsaved(&s.p),
        "the retry doesn't drop it"
    );
    assert_eq!(
        prompt(&read_json(&s.dir.join("project.json")).unwrap()),
        "mine"
    );

    resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
    assert!(!crate::project::has_unsaved(&s.p));
    assert_eq!(prompt(&s.p.manifest), "typed during");
    s.reopen();
    assert_eq!(prompt(&s.p.manifest), "typed during", "on disk");
}

/// Walk 2 N1, repro A: Keep other after a refused save keeps the other prompt current and puts
/// this machine's refused one in the `conflict` revision; nothing is left pending to overwrite it.
#[test]
fn fr_prj_36_keep_other_puts_a_refused_draft_in_the_conflict_revision() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    let mut v = read_value(&s.dir.join("project.json"));
    v["commit"]["seq"] = Value::from(40);
    v["commit"]["install_id"] = Value::from(other_pc());
    v["draft"]["final_prompt"] = Value::from("theirs");
    write_value(&s.dir.join("project.json"), &v);
    assert_eq!(
        draft(&mut s, "mine, edited").unwrap_err().code,
        ErrorCode::Conflict
    );

    let rev = resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::Other).unwrap();
    assert!(!crate::project::has_unsaved(&s.p));
    assert!(!retry_pending(&s.e.ctx, s.e.install(), &mut s.p));
    let file: RevisionFile = read_json(&s.dir.join(revision_file(rev.id))).unwrap();
    assert_eq!(file.draft.unwrap().final_prompt, "mine, edited");
    s.reopen();
    assert_eq!(prompt(&s.p.manifest), "theirs", "the kept version stays");
}

fn cand(shade: u8) -> NewCandidate {
    NewCandidate {
        bytes: rgba_png(16, 14, [shade, shade, shade, 255]),
        seed: Some(i64::from(shade)),
        provider_meta: serde_json::json!({"paid": true}),
        ..Default::default()
    }
}

fn succeeded(s: &mut S) -> crate::job::JobFile {
    let job = s.job("moss");
    for st in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
        s.to(job.id, st);
    }
    job
}

#[test]
fn fr_prj_40_candidates_spooled_during_a_conflict_replay_after_the_resolve() {
    let mut s = setup();
    let job = succeeded(&mut s);
    make_copy(&s, "theirs");
    // The store refuses during the conflict with a spoolable code; the runner spools (R48).
    let err = add_candidates(&s.e.ctx, s.e.install(), &mut s.p, job.id, vec![cand(9)]).unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(spool::spoolable(&err));
    let pid = s.p.manifest.project_id;
    spool::push_candidates(&s.e.ctx, pid, job.id, &[cand(9)]).unwrap();
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 1);
    // Not while the conflict waits for the user.
    let r = spool::replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!(r.entries, 0);
    assert!(s.p.manifest.candidates.is_empty());

    resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
    let r = spool::replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!((r.entries, r.fetched.clone()), (1, vec![job.id]));
    assert_eq!(
        spool::pending_count(&s.e.ctx, pid),
        0,
        "replayed and deleted"
    );
    let c = &s.p.manifest.candidates[0];
    assert_eq!(c.seed, Some(9));
    assert!(s.dir.join(&c.file).is_file());
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
    assert!(!spool::spool_dir(&s.e.ctx, pid).exists());
}

#[test]
fn fr_prj_40_spool_survives_a_crash_and_replays_in_order_once_writable() {
    let mut s = setup();
    let job = s.job("moss");
    s.to(job.id, JobState::Submitted);
    // Read-only while the job runs: the store refuses, the runner spools in order.
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
    assert_eq!(err.code, ErrorCode::ReadOnly);
    let pid = s.p.manifest.project_id;
    for st in [JobState::Running, JobState::Succeeded] {
        let u = spool::SpoolUpdate {
            state: Some(st),
            ..Default::default()
        };
        spool::push_update(&s.e.ctx, pid, job.id, &u).unwrap();
    }
    spool::push_candidates(&s.e.ctx, pid, job.id, &[cand(1)]).unwrap();
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 3);
    assert_eq!(
        spool::replay(&s.e.ctx, s.e.install(), &mut s.p)
            .unwrap()
            .entries,
        0
    );
    let on_disk: crate::job::JobFile = read_json(&s.dir.join(job.dir()).join("job.json")).unwrap();
    assert_eq!(on_disk.state, JobState::Submitted, "nothing written");

    // "Crash": the in-memory state is gone; the next writable open replays in seq order.
    s.reopen();
    let r = spool::replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!(r.entries, 3);
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 0);
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
    assert_eq!(s.p.manifest.candidates.len(), 1);
    assert_eq!(s.p.manifest.jobs[0].state, JobState::Fetched);
}

#[test]
fn fr_prj_40_a_spool_entry_refused_for_good_is_dropped_and_reported() {
    // BL-25: a transition the job already passed is dropped, and the user is told.
    let mut s = setup();
    let job = succeeded(&mut s);
    add_candidates(&s.e.ctx, s.e.install(), &mut s.p, job.id, vec![cand(1)]).unwrap();
    let pid = s.p.manifest.project_id;
    let u = spool::SpoolUpdate {
        state: Some(JobState::Running),
        ..Default::default()
    };
    spool::push_update(&s.e.ctx, pid, job.id, &u).unwrap();
    let r = spool::replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!(r.entries, 0);
    assert_eq!(r.dropped.len(), 1);
    let d = &r.dropped[0];
    assert_eq!((d.job_id, d.op), (job.id, "update_job"));
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 0, "deleted");
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched, "unchanged");
    let events = s.e.events.0.lock().unwrap();
    let (_, payload) = events
        .iter()
        .rev()
        .find(|(n, _)| n == spool::EVENT_SPOOL_DROPPED)
        .expect("project:spool-dropped fired");
    assert_eq!(payload["job_id"], job.id.to_string());
    assert_eq!(payload["op"], "update_job");
    assert_eq!(payload["code"], d.code.as_str());
    assert_eq!(payload["project_id"], pid.to_string());
}

#[test]
fn fr_prj_21_fr_prj_40_locked_candidates_wait_in_the_spool_until_unlocked() {
    let mut s = setup();
    let job = succeeded(&mut s);
    let pid = s.p.manifest.project_id;
    s.e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Locked).unwrap();
    let err = add_candidates(&s.e.ctx, s.e.install(), &mut s.p, job.id, vec![cand(1)]).unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    assert!(spool::spoolable(&err));
    spool::push_candidates(&s.e.ctx, pid, job.id, &[cand(1)]).unwrap();
    let r = spool::replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!(r.entries, 0, "still locked: kept");
    assert_eq!(spool::pending_count(&s.e.ctx, pid), 1);
    s.e.ctx.faults = Faults::none();
    let r = spool::replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!(r.entries, 1);
    assert_eq!(s.p.manifest.candidates.len(), 1);
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
    let _ = retry_pending;
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

/// Review item 9 / R102: a `lost` or `failed` copy (e.g. another machine's sweep) never beats
/// a copy that kept the job handle or candidates, however late it was written.
#[test]
fn fr_prj_34_oq_prj_10_a_copy_with_a_handle_or_candidates_beats_a_lost_or_failed_one() {
    let mut s = setup();
    let job = succeeded(&mut s);
    update_job(
        &s.e.ctx,
        &mut s.p,
        job.id,
        JobUpdate {
            provider: Some(serde_json::json!({"job_handle": "h-paid"})),
            ..Default::default()
        },
    )
    .unwrap();
    add_candidates(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        vec![cand(1), cand(2)],
    )
    .unwrap();
    let ours = s.p.jobs[&job.id].clone();
    assert_eq!(ours.state, JobState::Fetched);
    for state in ["lost", "failed"] {
        let dir = s.dir.join(job_dir(job.id));
        let mut v = read_value(&dir.join("job.json"));
        v["state"] = Value::from(state);
        v["provider"] = Value::Null;
        v["candidate_ids"] = serde_json::json!([]);
        v["updated_at"] = Value::from("2099-01-01T00:00:00Z");
        write_value(&dir.join("job-OTHERPC.json"), &v);
        s.reopen();
        let now = &s.p.jobs[&job.id];
        assert_eq!(now.state, JobState::Fetched, "{state} copy lost");
        assert_eq!(now.provider["job_handle"], "h-paid");
        assert_eq!(now.candidate_ids, ours.candidate_ids);
        let disk: crate::job::JobFile = read_json(&dir.join("job.json")).unwrap();
        assert_eq!(disk.state, JobState::Fetched);
        let _ = std::fs::remove_file(dir.join("job-OTHERPC.json"));
    }

    // Unit rows: a handle alone (submitted) beats `failed`; without a handle or candidates the
    // later state still wins; `canceled` (the user's) stays later than a running copy.
    let at = |state, handle: bool, t: i64| {
        let mut j = ours.clone();
        j.state = state;
        j.candidate_ids.clear();
        j.provider = if handle {
            serde_json::json!({"job_handle": "h"})
        } else {
            Value::Null
        };
        j.updated_at = crate::time::Timestamp(t);
        j
    };
    assert!(later(
        &at(JobState::Submitted, true, 1),
        &at(JobState::Failed, false, 9)
    ));
    assert!(!later(
        &at(JobState::Lost, false, 9),
        &at(JobState::Running, true, 1)
    ));
    assert!(later(
        &at(JobState::Lost, false, 9),
        &at(JobState::Requested, false, 1)
    ));
    assert!(later(
        &at(JobState::Canceled, true, 9),
        &at(JobState::Running, true, 1)
    ));
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

/// Walk 2 N2, R57: a trashed `project.json` conflict copy has no Restore and the core refuses it
/// before moving anything, so the project doesn't fall back into a conflict.
#[test]
fn fr_prj_31_r57_a_manifest_conflict_copy_is_not_restorable() {
    use crate::project::retention::{trash_list, trash_restore};
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    let copy = make_copy(&s, "theirs");
    s.reopen();
    resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
    let row = trash_list(&s.p)
        .into_iter()
        .find(|r| r.kind == TrashKind::ConflictCopy)
        .unwrap();
    assert!(!row.restorable, "no Restore button");
    assert!(row.bytes > 0);

    let err = trash_restore(&s.e.ctx, s.e.install(), &mut s.p, row.id).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotRestorable);
    assert!(!copy.exists(), "nothing moved back");
    assert!(s.p.conflict.is_none(), "no phantom conflict");
    assert!(!s.p.warnings.iter().any(|w| w.code == "conflict"));
    draft(&mut s, "still writable").unwrap();
}

/// Walk 2 N2: Keep this when the copy has gone meanwhile resolves without an empty trash item.
#[test]
fn fr_prj_36_keep_this_after_the_copy_vanished_adds_no_trash_item() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    let copy = make_copy(&s, "theirs");
    s.reopen();
    std::fs::remove_file(&copy).unwrap();
    let view = conflict_get(&s.p, s.e.install()).unwrap();
    assert!(!view.other.readable);
    resolve_conflict(&s.e.ctx, s.e.install(), &mut s.p, Keep::This).unwrap();
    assert!(s.p.conflict.is_none());
    assert!(s.p.manifest.trash.is_empty(), "{:?}", s.p.manifest.trash);
}

#[test]
fn bl_22_oq_prj_11_either_versions_layers_resolve_while_in_conflict() {
    let mut s = setup();
    draft(&mut s, "mine").unwrap();
    let copy = make_copy(&s, "theirs");
    let mut v = read_value(&copy);
    let theirs = Ulid::from_parts(1, 98);
    let mut layer = Layer::original(theirs, 40, 30);
    layer.kind = LayerKind::Patch;
    layer.file = Some(layer_file(theirs));
    v["layers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(&layer).unwrap());
    write_value(&copy, &v);
    std::fs::create_dir_all(s.dir.join(LAYERS_DIR)).unwrap();
    std::fs::write(s.dir.join(layer_file(theirs)), b"png").unwrap();
    let stranger = Ulid::from_parts(1, 97);
    assert_eq!(
        s.p.resolve_image(theirs).unwrap_err().code,
        ErrorCode::UnknownId,
        "no conflict yet: only this version's stack"
    );

    s.reopen();
    assert!(s.p.conflict.is_some());
    let r = s.p.resolve_image(theirs).unwrap();
    assert_eq!(r.kind, ImageKind::Layer);
    assert_eq!(r.path, s.dir.join(layer_file(theirs)));
    let original = s.p.manifest.original_image_id().unwrap();
    assert_eq!(
        s.p.resolve_image(original).unwrap().kind,
        ImageKind::Original
    );
    assert_eq!(
        s.p.resolve_image(stranger).unwrap_err().code,
        ErrorCode::UnknownId
    );

    // A copy whose original points outside the project folder doesn't resolve.
    let mut v = read_value(&copy);
    let other_original = Ulid::from_parts(1, 96);
    v["layers"][0]["id"] = Value::from(other_original.to_string());
    v["original"]["file"] = Value::from("../outside.png");
    write_value(&copy, &v);
    assert_eq!(
        s.p.resolve_image(other_original).unwrap_err().code,
        ErrorCode::UnknownId
    );
}

/// R110 (walk-r109 M1), FR-PRJ-21, FR-PRJ-35: a `project.json` the commit check can't read (an
/// exclusive lock, a transient I/O error) is `locked` and the save waits; it's never `conflict`.
#[test]
fn r110_fr_prj_21_fr_prj_35_an_unreadable_project_json_is_locked_never_conflict() {
    for action in [FaultAction::Locked, FaultAction::Error] {
        let mut s = setup();
        let job = succeeded(&mut s);
        s.e.ctx.faults = Faults::parse("on_read:project.json", action).unwrap();
        // Draft: `locked`, kept in memory as the waiting save.
        assert_eq!(draft(&mut s, "moss").unwrap_err().code, ErrorCode::Locked);
        assert!(crate::project::has_unsaved(&s.p));
        // Mask, Ctrl+S, candidates (accept and job writes go through the same check).
        let rect = Rect {
            x: 4,
            y: 4,
            w: 8,
            h: 6,
        };
        let err =
            crate::project::store::put_mask(&s.e.ctx, s.e.install(), &mut s.p, rect, vec![255; 48])
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        let err =
            crate::project::save_revision(&s.e.ctx, s.e.install(), &mut s.p, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        let err =
            add_candidates(&s.e.ctx, s.e.install(), &mut s.p, job.id, vec![cand(1)]).unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        assert!(spool::spoolable(&err));
        // No conflict was raised: nothing to resolve.
        assert!(s.p.conflict.is_none(), "{action:?}");
        assert_eq!(events(&s, "project:conflict"), 0);
        // Readable again: the waiting save commits on the next retry.
        s.e.ctx.faults = Faults::none();
        assert!(!retry_pending(&s.e.ctx, s.e.install(), &mut s.p));
        assert_eq!(prompt(&s.p.manifest), "moss");
        assert_eq!(
            prompt(&read_json::<Manifest>(&s.dir.join("project.json")).unwrap()),
            "moss"
        );
    }
}

/// R110 (walk-r109 M1) on a real exclusive handle (`FileShare.None`, the walk's method).
#[cfg(windows)]
#[test]
fn r110_fr_prj_21_an_exclusive_handle_on_project_json_is_locked_then_saves() {
    use std::os::windows::fs::OpenOptionsExt;
    let open_exclusive = |dir: &std::path::Path| {
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(dir.join("project.json"))
            .unwrap()
    };
    let mut s = setup();
    // Held past the 3 s retry: `locked`, the draft waits, no conflict.
    let lock = open_exclusive(&s.dir);
    assert_eq!(draft(&mut s, "moss").unwrap_err().code, ErrorCode::Locked);
    assert!(s.p.conflict.is_none());
    assert!(crate::project::has_unsaved(&s.p));
    drop(lock);
    assert!(!retry_pending(&s.e.ctx, s.e.install(), &mut s.p));
    assert_eq!(prompt(&s.p.manifest), "moss");
    // Released within the retry: the save just goes through.
    let lock = open_exclusive(&s.dir);
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(lock);
    });
    draft(&mut s, "fern").unwrap();
    release.join().unwrap();
    assert!(s.p.conflict.is_none());
    assert_eq!(prompt(&s.p.manifest), "fern");
}
