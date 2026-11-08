use std::sync::atomic::Ordering;

use super::*;
use crate::context::{FaultAction, Faults, ManualClock};
use crate::project::store::tests::setup;
use crate::project::store::{ResolveReason, resolve_job, set_star};
use crate::test_support::*;

const DAY_MS: i64 = 86_400_000;

fn set_now(ctx: &mut Context, ms: i64) {
    ctx.clock = Box::new(ManualClock::new(Timestamp(ms)));
}

#[test]
fn fr_prj_32_purge_after_30_days_keeps_starred_and_accepted() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 3);
    set_star(&s.e.ctx, s.e.install(), &mut s.p, c[1].id, true).unwrap();
    s.accept(job.id, c[0].id).unwrap();
    let trashed = s.cand(c[2].id);
    let item = s.p.manifest.trash[0].clone();
    assert_eq!(Some(item.id), trashed.trash_id);
    let trashed_copy = s.dir.join(item.trashed_path(&trashed.file));
    assert!(trashed_copy.is_file());

    // 29 days: nothing yet.
    set_now(&mut s.e.ctx, T0 + 29 * DAY_MS);
    s.reopen();
    assert_eq!(s.p.manifest.trash.len(), 1);
    assert!(trashed_copy.is_file());

    // 31 days (walkthrough step 8): the trashed candidate is gone; starred and accepted remain.
    set_now(&mut s.e.ctx, T0 + 31 * DAY_MS);
    s.reopen();
    assert!(s.p.manifest.trash.is_empty());
    assert!(!trashed_copy.exists());
    assert!(!s.dir.join(TRASH_DIR).join(item.id.to_string()).exists());
    assert!(s.dir.join(&c[0].file).is_file(), "accepted kept");
    assert!(s.dir.join(&c[1].file).is_file(), "starred kept");
    assert!(s.dir.join(job.dir()).join("region.png").is_file());
    let (name, payload) =
        s.e.events
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(n, _)| n == "trash:purged")
            .cloned()
            .unwrap();
    assert_eq!(name, "trash:purged");
    assert_eq!(payload["count"], 1);
    assert!(payload["bytes"].as_u64().unwrap() > 0);
    // The record stays (its file is gone) and says so (BL-23): a re-star can't bring it back.
    assert!(s.cand(c[2].id).trash_id.is_some());
    assert_eq!(s.cand(c[2].id).purged_at, Some(Timestamp(T0 + 31 * DAY_MS)));
    assert_eq!(s.cand(c[0].id).purged_at, None, "accepted isn't purged");
    assert_eq!(s.cand(c[1].id).purged_at, None, "starred isn't purged");
    s.reopen();
    assert_eq!(
        s.cand(c[2].id).purged_at,
        Some(Timestamp(T0 + 31 * DAY_MS)),
        "persisted"
    );
    let err = set_star(&s.e.ctx, s.e.install(), &mut s.p, c[2].id, true).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn fr_prj_32_purge_date_math_and_next_purge() {
    let mut s = setup();
    assert_eq!(disk_usage(&s.p).next_purge, None);
    let (job, _) = s.fetched("dust", 2);
    set_now(&mut s.e.ctx, T0 + 5 * DAY_MS);
    resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        ResolveReason::Dismiss,
    )
    .unwrap();
    let first =
        s.p.manifest
            .trash
            .iter()
            .map(|t| t.trashed_at)
            .min()
            .unwrap();
    assert_eq!(first, Timestamp(T0 + 5 * DAY_MS));
    assert_eq!(
        disk_usage(&s.p).next_purge,
        Some(Timestamp(T0 + 35 * DAY_MS))
    );
    let rows = trash_list(&s.p);
    assert!(
        rows.iter()
            .all(|r| r.purge_at == Timestamp(T0 + 35 * DAY_MS))
    );
    // Exactly 30 days is not yet "older than 30 days".
    set_now(&mut s.e.ctx, T0 + 35 * DAY_MS);
    assert_eq!(
        purge_expired(&s.e.ctx, s.e.install(), &mut s.p)
            .unwrap()
            .count,
        0
    );
    set_now(&mut s.e.ctx, T0 + 35 * DAY_MS + 1);
    assert_eq!(
        purge_expired(&s.e.ctx, s.e.install(), &mut s.p)
            .unwrap()
            .count,
        2,
        "candidates and inputs"
    );
}

#[test]
fn fr_prj_31_trash_list_and_restore_bring_files_back() {
    let mut s = setup();
    let (job, c) = s.fetched("dust", 2);
    resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        ResolveReason::Dismiss,
    )
    .unwrap();
    let rows = trash_list(&s.p);
    assert_eq!(rows.len(), 2);
    let cands = rows
        .iter()
        .find(|r| r.kind == TrashKind::Candidates)
        .unwrap();
    assert_eq!(cands.files.len(), 2);
    assert_eq!(cands.path, c[0].file);
    assert!(cands.bytes > 0);
    let inputs = rows
        .iter()
        .find(|r| r.kind == TrashKind::JobInputs)
        .unwrap()
        .clone();

    trash_restore(&s.e.ctx, s.e.install(), &mut s.p, cands.id).unwrap();
    for x in &c {
        assert!(s.dir.join(&x.file).is_file(), "back at its old path");
        assert_eq!(s.cand(x.id).trash_id, None);
    }
    assert!(!s.dir.join(TRASH_DIR).join(cands.id.to_string()).exists());
    trash_restore(&s.e.ctx, s.e.install(), &mut s.p, inputs.id).unwrap();
    assert!(s.dir.join(job.dir()).join("region.png").is_file());
    assert!(s.p.manifest.trash.is_empty());

    // Survives a reopen: nothing moves back into the trash.
    s.reopen();
    assert!(s.dir.join(&c[0].file).is_file());
    assert!(s.p.manifest.trash.is_empty());

    let err = trash_restore(&s.e.ctx, s.e.install(), &mut s.p, Ulid::nil()).unwrap_err();
    assert_eq!(
        (err.code, err.field.as_deref()),
        (ErrorCode::UnknownId, Some("item_id"))
    );
}

#[test]
fn fr_prj_31_restore_only_candidates_job_inputs_and_conflict_copies() {
    // R51 (BL-24): other kinds are listed but not restorable; the next sweep would undo it.
    let mut s = setup();
    let (job, _) = s.fetched("ash", 1);
    resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        ResolveReason::Dismiss,
    )
    .unwrap();
    let orphan = std::path::Path::new(PFORGE_DIR).join("stray.bin");
    std::fs::write(s.dir.join(&orphan), b"stray").unwrap();
    s.reopen();
    let rows = trash_list(&s.p);
    let by_kind = |k: TrashKind| rows.iter().find(|r| r.kind == k).unwrap().clone();
    assert!(by_kind(TrashKind::Candidates).restorable);
    assert!(by_kind(TrashKind::JobInputs).restorable);
    let o = by_kind(TrashKind::Orphan);
    assert!(!o.restorable);

    let err = trash_restore(&s.e.ctx, s.e.install(), &mut s.p, o.id).unwrap_err();
    assert_eq!(
        (err.code, err.field.as_deref()),
        (ErrorCode::NotRestorable, Some("item_id"))
    );
    assert_eq!(err.detail["kind"], "orphan");
    assert!(
        s.p.manifest.trash.iter().any(|t| t.id == o.id),
        "still in the trash"
    );
    assert!(s.dir.join(TRASH_DIR).join(o.id.to_string()).exists());

    for k in [
        TrashKind::Candidates,
        TrashKind::JobInputs,
        TrashKind::ConflictCopy,
    ] {
        assert!(restorable_kind(k));
    }
    for k in [
        TrashKind::Mask,
        TrashKind::Layer,
        TrashKind::Manifest,
        TrashKind::Orphan,
    ] {
        assert!(!restorable_kind(k));
    }
}

#[test]
fn fr_prj_20_restore_that_fails_to_commit_leaves_the_item_in_the_trash() {
    let mut s = setup();
    let (job, c) = s.fetched("dust", 1);
    resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        ResolveReason::Dismiss,
    )
    .unwrap();
    let item = trash_list(&s.p)
        .into_iter()
        .find(|r| r.kind == TrashKind::Candidates)
        .unwrap();
    s.e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Error).unwrap();
    assert!(trash_restore(&s.e.ctx, s.e.install(), &mut s.p, item.id).is_err());
    s.e.ctx.faults = Faults::none();
    assert!(
        !s.dir.join(&c[0].file).exists(),
        "moved back into the trash"
    );
    let it = s.p.manifest.trash.iter().find(|t| t.id == item.id).unwrap();
    assert!(s.dir.join(it.trashed_path(&c[0].file)).is_file());
    s.reopen();
    assert!(s.p.manifest.trash.iter().any(|t| t.id == item.id));
}

#[test]
fn fr_prj_41_trash_restore_is_refused_read_only() {
    let mut s = setup();
    let (job, _) = s.fetched("dust", 1);
    resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        ResolveReason::Dismiss,
    )
    .unwrap();
    let id = s.p.manifest.trash[0].id;
    s.p.read_only = Some(crate::project::ReadOnlyReason::OriginalChanged);
    let err = trash_restore(&s.e.ctx, s.e.install(), &mut s.p, id).unwrap_err();
    assert_eq!(err.code, ErrorCode::ReadOnly);
    set_now(&mut s.e.ctx, T0 + 40 * DAY_MS);
    assert_eq!(
        purge_expired(&s.e.ctx, s.e.install(), &mut s.p)
            .unwrap()
            .count,
        0,
        "nothing is written in a read-only project"
    );
}

#[test]
fn fr_prj_33_disk_usage_by_category_never_hydrates() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 2);
    s.accept(job.id, c[0].id).unwrap();
    std::fs::write(s.dir.join("references/mood.png"), vec![0u8; 1000]).unwrap();
    std::fs::create_dir_all(s.dir.join("parts/doors")).unwrap();
    std::fs::write(s.dir.join("parts/doors/door.png"), vec![0u8; 500]).unwrap();
    std::fs::write(s.dir.join("exports/Crypt_70px.png"), vec![0u8; 300]).unwrap();
    // Every file is cloud-only: disk use must read metadata only.
    for f in tree(&s.dir).iter().filter(|f| !f.ends_with('/')) {
        s.e.fs.cloud_only.lock().unwrap().insert(s.dir.join(f));
    }
    let calls = s.e.fs.hydrate_calls.load(Ordering::Relaxed);
    let u = disk_usage(&s.p);
    assert_eq!(s.e.fs.hydrate_calls.load(Ordering::Relaxed), calls);
    let size = |rel: &str| std::fs::metadata(s.dir.join(rel)).unwrap().len();
    assert_eq!(u.bytes.original, size("working/Crypt.png"));
    assert_eq!(u.bytes.user_folders, 1500);
    assert_eq!(u.bytes.exports, 300);
    assert_eq!(u.bytes.results, size(&c[0].file));
    assert!(u.bytes.layers > 0 && u.bytes.masks > 0 && u.bytes.job_inputs > 0);
    let item = &s.p.manifest.trash[0];
    assert_eq!(u.bytes.trash, size(&item.trashed_path(&c[1].file)));
    assert!(u.bytes.other >= size("project.json"));
    assert_eq!(u.total, u.bytes.total());
    assert_eq!(u.trash_count, 1);
}

#[test]
fn fr_prj_21_locked_draft_stays_in_memory_and_is_retried() {
    let mut s = setup();
    s.e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Locked).unwrap();
    let input = crate::project::DraftInput {
        final_prompt: "moss".into(),
        ..Default::default()
    };
    let err =
        crate::project::put_draft(&s.e.ctx, s.e.install(), &mut s.p, input, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    assert!(crate::project::has_unsaved(&s.p), "state stays in memory");
    let failed =
        s.e.events
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(n, _)| n == "project:save-failed")
            .cloned()
            .unwrap();
    assert_eq!(failed.1, json!({"reason": "locked", "retrying": true}));

    // Still locked at the next 10 s tick.
    assert!(crate::project::retry_pending(
        &s.e.ctx,
        s.e.install(),
        &mut s.p
    ));
    assert!(s.p.pending_draft.is_some());

    // Unlocked: the retry commits it.
    s.e.ctx.faults = Faults::none();
    assert!(!crate::project::retry_pending(
        &s.e.ctx,
        s.e.install(),
        &mut s.p
    ));
    assert!(!crate::project::has_unsaved(&s.p));
    assert_eq!(s.p.manifest.draft.as_ref().unwrap().final_prompt, "moss");
    s.reopen();
    assert_eq!(s.p.manifest.draft.as_ref().unwrap().final_prompt, "moss");
}
