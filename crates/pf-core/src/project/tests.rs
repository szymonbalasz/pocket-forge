use super::*;
use crate::context::{FaultAction, Faults};
use crate::test_support::*;

fn draft(e: &Env, p: &mut OpenProject, prompt: &str) -> CoreResult<u64> {
    put_draft(
        &e.ctx,
        e.install(),
        p,
        DraftInput {
            final_prompt: prompt.into(),
            ..Default::default()
        },
        None,
    )
}

#[test]
fn fr_prj_12_fr_prj_09_creation_writes_the_folder_layout_and_r0() {
    let e = env();
    let dir = e.project("Crypt");
    assert_eq!(dir, e.lib().join("Maps/Crypt"));
    let p = e.open(&dir);
    let files = tree(&dir);
    for expected in [
        "project.json",
        "working/Crypt.png",
        "references/",
        "structure/",
        "parts/",
        "results/",
        "exports/",
        ".pforge/layers/",
        ".pforge/masks/",
        ".pforge/jobs/",
        ".pforge/trash/",
    ] {
        assert!(
            files.iter().any(|f| f == expected),
            "{expected} in {files:?}"
        );
    }
    let r0 = p.manifest.revisions[0].clone();
    assert_eq!((r0.seq, r0.reason), (0, RevisionReason::Create));
    assert!(dir.join(revision_file(r0.id)).is_file());
    assert_eq!(p.manifest.kind, Kind::Map);
    assert_eq!(p.manifest.commit.seq, 1);
    let text = std::fs::read_to_string(dir.join("project.json")).unwrap();
    assert!(text.contains(r#""file": "working/Crypt.png""#), "{text}");
    let root = e.dir.path().to_string_lossy().replace('\\', "\\\\");
    assert!(!text.contains(&*root), "absolute path leaked: {text}");
}

#[test]
fn fr_prj_14_open_takes_a_project_folder_only() {
    let e = env();
    let dir = e.project("Crypt");
    let open_err = |p: &Path| open(&e.ctx, &e.settings, p, None).unwrap_err().code;
    assert_eq!(
        open_err(&dir.join("working/Crypt.png")),
        ErrorCode::NotAProject
    );
    std::fs::create_dir(e.lib().join("Maps/empty")).unwrap();
    assert_eq!(
        open_err(&e.lib().join("Maps/empty")),
        ErrorCode::NotAProject
    );
    assert_eq!(open_err(&e.lib().join("Maps/none")), ErrorCode::NotFound);
    let outside = e.dir.path().join("outside");
    assert_eq!(open_err(&outside), ErrorCode::OutsideLibrary);
    let p = e.open(&dir);
    assert_eq!(p.snapshot().name, "Crypt");
    assert_eq!(p.snapshot().image_path, Some(dir.join("working/Crypt.png")));
}

#[test]
fn fr_prj_22_save_close_reopen_is_field_identical() {
    let e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    draft(&e, &mut p, "a mossy crypt").unwrap();
    let reopened = e.open(&dir);
    assert_eq!(reopened.manifest, p.manifest);
    assert_eq!(reopened.snapshot(), p.snapshot());
    assert_eq!(
        reopened.manifest.draft.as_ref().unwrap().final_prompt,
        "a mossy crypt"
    );
}

#[test]
fn fr_prj_19_a_draft_save_is_a_commit_not_a_revision() {
    let e = env();
    let mut p = e.open(&e.project("Crypt"));
    assert_eq!(draft(&e, &mut p, "x").unwrap(), 2);
    assert_eq!(draft(&e, &mut p, "y").unwrap(), 3);
    assert_eq!(p.manifest.revisions.len(), 1);
}

#[test]
fn fr_prj_19_r30_draft_save_carries_export_settings() {
    let e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    let settings =
        serde_json::json!({"grid": {"mode": "px", "source_px_per_square": 70}, "format": "webp"});
    put_draft(
        &e.ctx,
        e.install(),
        &mut p,
        DraftInput::default(),
        Some(settings.as_object().unwrap().clone()),
    )
    .unwrap();
    let back = e.open(&dir);
    assert_eq!(back.manifest.export.unwrap()["format"], "webp");
}

#[test]
fn fr_prj_23_save_only_if_changed_since_the_head() {
    let e = env();
    let mut p = e.open(&e.project("Crypt"));
    let r0 = p.manifest.head_revision;
    let err = save_revision(&e.ctx, e.install(), &mut p, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unchanged);
    draft(&e, &mut p, "changed").unwrap();
    let r1 = save_revision(&e.ctx, e.install(), &mut p, Some("mine".into())).unwrap();
    assert_eq!(
        (r1.seq, r1.reason, r1.label.as_deref()),
        (1, RevisionReason::Save, Some("mine"))
    );
    assert_eq!(read_revision(&p, r1.id).unwrap().parent, Some(r0));
}

#[test]
fn fr_prj_23_fr_prj_21_save_with_a_pending_draft_saves_it_or_says_why() {
    let mut e = env();
    let mut p = e.open(&e.project("Crypt"));
    e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Locked).unwrap();
    assert_eq!(
        draft(&e, &mut p, "moss").unwrap_err().code,
        ErrorCode::Locked
    );
    // The head equals the manifest, but the draft waits in memory: never `unchanged`.
    let err = save_revision(&e.ctx, e.install(), &mut p, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    assert!(has_unsaved(&p));
    // Unlocked: Ctrl+S writes the waiting draft and the revision with it.
    e.ctx.faults = Faults::none();
    let r1 = save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    assert!(!has_unsaved(&p));
    let rev = read_revision(&p, r1.id).unwrap();
    assert_eq!(rev.draft.unwrap().final_prompt, "moss");
}

#[test]
fn fr_prj_43_restore_writes_a_new_restore_revision() {
    let e = env();
    let mut p = e.open(&e.project("Crypt"));
    let r0 = p.manifest.head_revision;
    draft(&e, &mut p, "later").unwrap();
    save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    let r2 = restore_revision(&e.ctx, e.install(), &mut p, r0).unwrap();
    assert_eq!((r2.seq, r2.reason), (2, RevisionReason::Restore));
    assert_eq!(p.manifest.draft, None);
    assert_eq!(p.manifest.revisions.len(), 3, "no revision is ever deleted");
    let err = restore_revision(&e.ctx, e.install(), &mut p, Ulid::from_parts(1, 1)).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownId);
}

#[test]
fn bl_32_open_sweep_keeps_a_fresh_export_temp_like_fr_exp_17() {
    let e = env();
    let dir = e.project("Crypt");
    let exports = dir.join(EXPORTS_DIR);
    std::fs::create_dir_all(&exports).unwrap();
    let old = exports.join(".Crypt.png.pf-tmp-01old");
    let fresh = exports.join(".Crypt.png.pf-tmp-01fresh");
    let layer = dir.join(".pforge/layers/.a.png.pf-tmp-y");
    for p in [&old, &fresh, &layer] {
        std::fs::write(p, b"x").unwrap();
    }
    let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(two_hours_ago)
        .unwrap();
    e.open(&dir);
    assert!(!old.exists(), "a stale export temp is swept");
    assert!(fresh.exists(), "a fresh one may be a running export's");
    assert!(!layer.exists(), "app-owned temps elsewhere go at any age");
}

#[test]
fn fr_prj_20_crash_after_manifest_temp_reopens_the_state_before() {
    let mut e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    draft(&e, &mut p, "before").unwrap();
    let committed = e.open(&dir).manifest;
    e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Error).unwrap();
    assert!(draft(&e, &mut p, "after").is_err());
    assert_eq!(
        p.manifest, committed,
        "in-memory state unchanged on failure"
    );
    // Simulate the kill leaving temp files behind.
    std::fs::write(dir.join(".project.json.pf-tmp-x"), b"partial").unwrap();
    std::fs::write(dir.join(".pforge/layers/.a.png.pf-tmp-y"), b"partial").unwrap();
    e.ctx.faults = Faults::none();
    let reopened = e.open(&dir);
    assert_eq!(reopened.manifest, committed);
    assert!(
        !tree(&dir).iter().any(|f| f.contains(fsx::TEMP_MARKER)),
        "temp files removed on open"
    );
}

#[test]
fn fr_prj_20_crash_after_rename_reopens_the_state_after() {
    let mut e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    e.ctx.faults = Faults::parse("after_rename:project.json", FaultAction::Error).unwrap();
    let _ = draft(&e, &mut p, "after");
    e.ctx.faults = Faults::none();
    assert_eq!(
        e.open(&dir).manifest.draft.unwrap().final_prompt,
        "after",
        "the rename is the commit point"
    );
}

#[test]
fn fr_prj_47_user_temp_named_files_are_never_swept() {
    let e = env();
    let dir = e.project("Crypt");
    std::fs::write(dir.join("references/.mood.png.pf-tmp-x"), b"mine").unwrap();
    std::fs::write(dir.join("references/stray.txt"), b"mine").unwrap();
    e.open(&dir);
    assert!(dir.join("references/.mood.png.pf-tmp-x").exists());
    assert!(dir.join("references/stray.txt").exists());
}

#[test]
fn fr_prj_26_newer_schema_opens_read_only_and_writes_nothing() {
    let e = env();
    let dir = e.project("Crypt");
    let path = dir.join("project.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["schema_version"] = 2.into();
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    std::fs::write(dir.join(".pforge/.x.pf-tmp-1"), b"t").unwrap();
    let before = tree(&dir);
    let mut p = e.open(&dir);
    assert_eq!(p.read_only, Some(ReadOnlyReason::NewerVersion));
    assert_eq!(
        draft(&e, &mut p, "x").unwrap_err().code,
        ErrorCode::ReadOnly
    );
    assert_eq!(tree(&dir), before, "nothing written, not even temp cleanup");
}

#[test]
fn fr_prj_15_changed_original_opens_read_only_even_within_one_mtime_tick() {
    let e = env();
    let dir = e.project("Crypt");
    let orig = dir.join("working/Crypt.png");
    let mtime = std::fs::metadata(&orig).unwrap().modified().unwrap();
    // Same size, different bytes, and the old mtime put back: size + mtime can't tell.
    let mut bytes = std::fs::read(&orig).unwrap();
    let n = bytes.len();
    bytes[n - 20] ^= 0xFF;
    std::fs::write(&orig, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&orig)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let p = e.open(&dir);
    assert_eq!(p.read_only, Some(ReadOnlyReason::OriginalChanged));
    assert_eq!(p.warnings[0].code, "original_changed");
}

#[test]
fn fr_prj_15_touched_but_identical_original_stays_writable() {
    let e = env();
    let dir = e.project("Crypt");
    let orig = dir.join("working/Crypt.png");
    let bytes = std::fs::read(&orig).unwrap();
    std::fs::write(&orig, bytes).unwrap(); // same bytes, new mtime
    assert_eq!(e.open(&dir).read_only, None);
}

#[test]
fn fr_prj_16_missing_original_relinks_by_hash_into_working() {
    let e = env();
    let dir = e.project("Crypt");
    let orig = dir.join("working/Crypt.png");
    let keep = e.outside("backup/any-name.png", &std::fs::read(&orig).unwrap());
    let wrong = e.outside("other.png", &png_bytes(40, 30, 99));
    std::fs::remove_file(&orig).unwrap();
    let mut p = e.open(&dir);
    assert_eq!(p.read_only, Some(ReadOnlyReason::OriginalMissing));
    assert_eq!(p.warnings[0].code, "original_missing");
    assert_eq!(
        draft(&e, &mut p, "x").unwrap_err().code,
        ErrorCode::ReadOnly
    );
    let err = relink_original(&e.ctx, e.install(), &mut p, &wrong).unwrap_err();
    assert_eq!(err.code, ErrorCode::HashMismatch);
    relink_original(&e.ctx, e.install(), &mut p, &keep).unwrap();
    assert!(orig.is_file() && keep.is_file(), "copied back, source kept");
    assert_eq!(p.read_only, None);
    draft(&e, &mut p, "works again").unwrap();
    assert_eq!(e.open(&dir).read_only, None);
}

#[test]
fn fr_prj_17_cloud_only_original_is_hydrated_on_open_or_offline() {
    let e = env();
    let dir = e.project("Crypt");
    let orig = dir.join("working/Crypt.png");
    e.fs.cloud_only.lock().unwrap().insert(orig.clone());
    e.fs.offline
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let err = open(&e.ctx, &e.settings, &dir, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotAvailableOffline);
    e.fs.offline
        .store(false, std::sync::atomic::Ordering::Relaxed);
    e.open(&dir);
    assert!(!e.fs.is_cloud_only(&orig), "hydrated once");
}

#[test]
fn r33_truncated_original_is_refused_on_open() {
    let e = env();
    let dir = e.project("Crypt");
    let orig = dir.join("working/Crypt.png");
    let bytes = std::fs::read(&orig).unwrap();
    std::fs::write(&orig, &bytes[..bytes.len() - 30]).unwrap();
    let err = open(&e.ctx, &e.settings, &dir, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Corrupt);
}

#[test]
fn fr_prj_35_seq_mismatch_refuses_the_write() {
    let e = env();
    let dir = e.project("Crypt");
    let mut a = e.open(&dir);
    let mut b = e.open(&dir);
    draft(&e, &mut b, "two").unwrap();
    let err = draft(&e, &mut a, "three").unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(e.open(&dir).manifest.draft.unwrap().final_prompt, "two");
}

#[test]
fn fr_prj_46_kind_follows_the_folder_at_the_next_commit() {
    let e = env();
    let dir = e.project("Crypt");
    let moved = e.lib().join("Tokens/Crypt");
    rename_dir(&dir, &moved);
    let mut p = e.open(&moved);
    assert_eq!(p.manifest.kind, Kind::Token);
    assert!(p.warnings.iter().any(|w| w.code == "kind_changed"));
    draft(&e, &mut p, "x").unwrap();
    let v: Value =
        serde_json::from_slice(&std::fs::read(moved.join("project.json")).unwrap()).unwrap();
    assert_eq!(v["kind"], "token");
}

#[test]
fn edge_folder_moved_while_open_fails_the_next_commit_with_not_found() {
    let e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    rename_dir(&dir, &e.lib().join("Maps/Elsewhere"));
    let err = draft(&e, &mut p, "x").unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn fr_prj_21_edge_folder_moved_while_open_keeps_the_draft_and_says_so_without_paths() {
    let e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    let moved = e.lib().join("Maps/Elsewhere");
    rename_dir(&dir, &moved);

    // The background draft save: refused, kept in memory, surfaced (walkthrough D3).
    let err = draft(&e, &mut p, "unsaved prompt").unwrap_err();
    assert!(is_folder_moved(&err), "{err:?}");
    assert!(has_unsaved(&p), "closing must ask");
    let failed = e
        .events
        .0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(n, _)| n == "project:save-failed")
        .cloned()
        .unwrap();
    assert_eq!(
        failed.1,
        serde_json::json!({"reason": "folder_moved", "retrying": true})
    );
    assert!(p.warnings.iter().any(|w| w.code == FOLDER_MOVED), "banner");

    // Manual Save and Restore: the same error, never the OS text or an absolute path.
    let root = e.dir.path().to_string_lossy().into_owned();
    let head = p.manifest.head_revision;
    for err in [
        save_revision(&e.ctx, e.install(), &mut p, None).unwrap_err(),
        restore_revision(&e.ctx, e.install(), &mut p, head).unwrap_err(),
    ] {
        assert!(is_folder_moved(&err), "{err:?}");
        assert!(!err.message.contains(&root), "{}", err.message);
        assert!(!err.message.contains("os error"), "{}", err.message);
    }

    // Still moved at the 10 s retry; renamed back → the retry commits the kept draft.
    assert!(retry_pending(&e.ctx, e.install(), &mut p));
    rename_dir(&moved, &dir);
    assert!(!retry_pending(&e.ctx, e.install(), &mut p));
    assert!(!has_unsaved(&p));
    assert!(!p.warnings.iter().any(|w| w.code == FOLDER_MOVED));
    assert_eq!(
        e.open(&dir).manifest.draft.unwrap().final_prompt,
        "unsaved prompt"
    );
}

#[test]
fn fr_prj_21_other_draft_save_failures_are_surfaced_not_retried() {
    let mut e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Error).unwrap();
    let err = draft(&e, &mut p, "x").unwrap_err();
    assert_eq!(err.code, ErrorCode::Io);
    assert!(!has_unsaved(&p));
    let failed = e
        .events
        .0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(n, _)| n == "project:save-failed")
        .cloned()
        .unwrap();
    assert_eq!(
        failed.1,
        serde_json::json!({"reason": "io", "retrying": false})
    );
}

#[test]
fn fr_prj_24_a_deleted_manifest_is_rebuilt_from_the_history() {
    let e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    draft(&e, &mut p, "keep me").unwrap();
    let r1 = save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    std::fs::remove_file(dir.join("project.json")).unwrap();
    let q = e.open(&dir);
    assert!(q.warnings.iter().any(|w| w.code == "recovered"));
    assert!(e.events.names().contains(&"project:recovered".to_owned()));
    assert_eq!(q.manifest.head_revision, r1.id);
    assert_eq!(q.manifest.draft.unwrap().final_prompt, "keep me");
    assert_eq!(q.manifest.original, p.manifest.original);
    assert!(dir.join("project.json").is_file(), "recovery written");
    let mut q = e.open(&dir);
    draft(&e, &mut q, "and writable").unwrap();
}

#[test]
fn fr_prj_24_corrupt_manifest_is_rebuilt_too() {
    let e = env();
    let dir = e.project("Crypt");
    std::fs::write(dir.join("project.json"), b"{ broken").unwrap();
    let q = e.open(&dir);
    assert!(q.warnings.iter().any(|w| w.code == "recovered"));
}

#[test]
fn fr_prj_47_fr_prj_48_user_folders_list_and_add_copies() {
    let e = env();
    let dir = e.project("Crypt");
    std::fs::create_dir_all(dir.join("references/sub")).unwrap();
    std::fs::write(dir.join("references/mood.PNG"), png_bytes(4, 4, 1)).unwrap();
    std::fs::write(dir.join("references/sub/notes.txt"), b"hi").unwrap();
    let p = e.open(&dir);
    let files = folder_list(&p, "references").unwrap();
    let names: Vec<_> = files.iter().map(|f| (f.name.as_str(), f.image)).collect();
    assert_eq!(names, [("mood.PNG", true), ("sub/notes.txt", false)]);
    let src = e.outside("mood.PNG", b"another");
    let after = folder_add(&e.ctx, &p, "references", &[src.clone(), src]).unwrap();
    let names: Vec<_> = after.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["mood (2).PNG", "mood (3).PNG", "mood.PNG", "sub/notes.txt"]
    );
    assert_eq!(
        std::fs::read(dir.join("references/mood.PNG")).unwrap(),
        png_bytes(4, 4, 1)
    );
    assert_eq!(
        folder_add(&e.ctx, &p, "working", &[]).unwrap_err().code,
        ErrorCode::UnknownFolder
    );
    assert_eq!(
        folder_list(&p, "../x").unwrap_err().code,
        ErrorCode::UnknownFolder
    );
    assert_eq!(folder_list(&p, "working").unwrap()[0].name, "Crypt.png");
}

#[test]
fn fr_prj_17_folder_list_never_hydrates() {
    let e = env();
    let dir = e.project("Crypt");
    let f = dir.join("references/cloud.png");
    std::fs::write(&f, b"x").unwrap();
    e.fs.cloud_only.lock().unwrap().insert(f);
    let p = e.open(&dir);
    let calls =
        e.fs.hydrate_calls
            .load(std::sync::atomic::Ordering::Relaxed);
    folder_list(&p, "references").unwrap();
    assert_eq!(
        e.fs.hydrate_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        calls
    );
}

#[test]
fn r48_3_resolve_image_gives_the_original_by_its_stable_id() {
    let e = env();
    let dir = e.project("Crypt");
    let p = e.open(&dir);
    let id = p.snapshot().original_image_id.unwrap();
    assert_eq!(id, p.manifest.layers[0].id);
    assert_eq!(
        p.resolve_image(id).unwrap(),
        ResolvedImage {
            path: dir.join("working/Crypt.png"),
            kind: ImageKind::Original
        }
    );
    assert_eq!(
        e.open(&dir).snapshot().original_image_id,
        Some(id),
        "stable across reopen"
    );
    assert_eq!(
        p.resolve_image(Ulid::from_parts(5, 5)).unwrap_err().code,
        ErrorCode::UnknownId
    );
}

#[test]
fn fr_prj_50_thumbnail_is_written_after_creation_and_rebuilt_when_missing() {
    let e = env();
    let src = e.outside("Big.png", &png_bytes(1200, 600, 3));
    let thumbs = crate::thumb::Thumbnails::new(&e.ctx);
    let items = crate::create::create_projects(
        &e.ctx,
        &e.settings,
        Kind::Map,
        &[src],
        &std::sync::atomic::AtomicBool::new(false),
        Some(&thumbs),
    )
    .unwrap();
    let crate::create::CreateItem::Created { path: dir, .. } = &items[0] else {
        panic!("{items:?}")
    };
    let thumb = dir.join(THUMB_FILE);
    let img = image::open(&thumb).unwrap();
    assert_eq!((img.width(), img.height()), (512, 256));
    assert!(
        e.events
            .names()
            .contains(&"project:thumb-updated".to_owned())
    );
    std::fs::remove_file(&thumb).unwrap();
    open(&e.ctx, &e.settings, dir, Some(thumbs)).unwrap();
    assert!(thumb.is_file(), "rebuilt on open");
}

#[test]
fn fr_prj_42_decoded_original_is_owned_by_the_project() {
    let e = env();
    let p = e.open(&e.project("Crypt"));
    let img = store::decoded_original(&e.ctx, &p).unwrap();
    assert_eq!((img.width(), img.height()), (40, 30));
}
