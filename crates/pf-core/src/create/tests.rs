use super::*;
use crate::library::{self, CardBadge};
use crate::test_support::*;

fn created(item: &CreateItem) -> &Path {
    match item {
        CreateItem::Created { path, .. } => path,
        other => panic!("{other:?}"),
    }
}

#[test]
fn fr_prj_04_source_is_copied_into_working_and_left_unchanged() {
    let e = env();
    let src = e.outside("crypt.png", &png_bytes(8, 8, 10));
    let before = (
        std::fs::metadata(&src).unwrap().modified().unwrap(),
        fsx::hash_file(&src).unwrap(),
    );
    let items = e.create(Kind::Map, std::slice::from_ref(&src));
    let dir = created(&items[0]);
    assert_eq!(dir, e.lib().join("Maps/crypt"));
    assert_eq!(fsx::hash_file(&dir.join("working/crypt.png")).unwrap(), before.1);
    let after = (
        std::fs::metadata(&src).unwrap().modified().unwrap(),
        fsx::hash_file(&src).unwrap(),
    );
    assert_eq!(before, after);
}

#[test]
fn fr_prj_04_a_source_inside_the_library_is_copied_too() {
    let e = env();
    let a = e.project("Crypt");
    let inside = a.join("working/Crypt.png");
    let items = e.create(Kind::Token, std::slice::from_ref(&inside));
    let dir = created(&items[0]);
    assert_eq!(dir, e.lib().join("Tokens/Crypt"));
    assert!(dir.join("working/Crypt.png").is_file() && inside.is_file());
}

#[test]
fn fr_prj_06_unsupported_files_are_skipped_and_the_batch_continues() {
    let e = env();
    let psd = e.outside("art.psd", b"8BPS\0\x01rest");
    let ok = e.outside("ok.png", &png_bytes(4, 4, 2));
    let garbage = e.outside("garbage.png", b"not an image at all");
    let full = png_bytes(32, 32, 1);
    let cut = e.outside("cut.png", &full[..full.len() - 16]);
    let missing = e.dir.path().join("outside/none.png");
    let jpeg = e.outside("photo.jpg", &jpeg_bytes(16, 16));
    let items = e.create(Kind::Map, &[psd.clone(), ok, garbage, cut, missing, jpeg]);
    let reasons: Vec<_> = items
        .iter()
        .map(|i| match i {
            CreateItem::Skipped { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        [
            Some(SkipReason::Unsupported),
            None,
            Some(SkipReason::Corrupt),
            Some(SkipReason::Corrupt),
            Some(SkipReason::NotFound),
            None
        ]
    );
}

#[test]
fn fr_prj_06_animated_and_too_large_are_skipped() {
    let e = env();
    let mut big = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    big.extend_from_slice(&16385u32.to_be_bytes());
    big.extend_from_slice(&10u32.to_be_bytes());
    big.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    let big = e.outside("huge.png", &big);
    let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
    webp.extend_from_slice(&[10, 0, 0, 0, 0x02, 0, 0, 0]);
    let anim = e.outside("anim.webp", &webp);
    let items = e.create(Kind::Map, &[big, anim]);
    assert!(matches!(items[0], CreateItem::Skipped { reason: SkipReason::TooLarge, .. }));
    assert!(matches!(items[1], CreateItem::Skipped { reason: SkipReason::Animated, .. }));
}

#[test]
fn fr_prj_07_a_folder_makes_one_flat_project_per_image() {
    let e = env();
    e.outside("drop/a.png", &png_bytes(4, 4, 1));
    e.outside("drop/sub/b.png", &png_bytes(4, 4, 2));
    e.outside("drop/sub/deeper/c.jpg", &jpeg_bytes(8, 8));
    e.outside("drop/readme.txt", b"ignored");
    let items = e.create(Kind::Token, &[e.dir.path().join("outside/drop")]);
    assert_eq!(items.len(), 3, "{items:?}");
    let mut names: Vec<_> = items
        .iter()
        .map(|i| created(i).file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["a", "b", "c"]);
    assert!(e.lib().join("Tokens/b/working/b.png").is_file(), "no subfolders kept");
}

#[test]
fn fr_prj_08_name_collisions_get_a_suffix_and_same_hash_is_already_in_library() {
    let e = env();
    let a = e.outside("map.png", &png_bytes(4, 4, 1));
    let b = e.outside("other/Map.png", &png_bytes(4, 4, 99));
    let items = e.create(Kind::Map, &[a.clone(), b, a.clone()]);
    assert_eq!(created(&items[0]), e.lib().join("Maps/map"));
    assert_eq!(created(&items[1]), e.lib().join("Maps/Map (2)"), "case-insensitive");
    assert_eq!(
        items[2],
        CreateItem::AlreadyInLibrary {
            source: a.clone(),
            path: e.lib().join("Maps/map")
        }
    );
    // Dedupe is per section: the same image is a new project in Tokens.
    assert!(matches!(e.create(Kind::Token, &[a])[0], CreateItem::Created { .. }));
}

#[test]
fn fr_prj_44_awkward_names() {
    let e = env();
    let items = e.create(
        Kind::Item,
        &[
            e.outside("CON.png", &png_bytes(4, 4, 1)),
            e.outside("a b. .png", &png_bytes(4, 4, 2)),
            e.outside(&format!("{}.png", "x".repeat(80)), &png_bytes(4, 4, 3)),
        ],
    );
    let names: Vec<_> = items
        .iter()
        .map(|i| created(i).file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names[0], "CON_");
    assert_eq!(names[1], "a b");
    assert_eq!(names[2], "x".repeat(64));
}

#[test]
fn fr_prj_09_no_partial_project_after_a_failed_creation() {
    let mut e = env();
    let src = e.outside("crypt.png", &png_bytes(8, 8, 1));
    e.ctx.faults =
        crate::context::Faults::parse("after_temp:project.json", crate::context::FaultAction::Error)
            .unwrap();
    let items = e.create(Kind::Map, &[src]);
    assert!(matches!(items[0], CreateItem::Skipped { reason: SkipReason::Io, .. }));
    assert_eq!(tree(&e.lib().join("Maps")), Vec::<String>::new(), "nothing left");
}

#[test]
fn fr_prj_10_pasted_data_becomes_a_png_project() {
    let e = env();
    let mut c = Creator::new(&e.ctx, &e.settings, Kind::Map, None).unwrap();
    let dir = c.create_from_bytes(&jpeg_bytes(10, 6)).unwrap();
    assert_eq!(dir, e.lib().join("Maps/pasted-20270115-080000"));
    let png = dir.join("working/pasted-20270115-080000.png");
    let info = pf_imaging::probe_file(&png).unwrap();
    assert_eq!((info.format, info.width, info.height), (pf_imaging::ImageFormat::Png, 10, 6));
    let again = c.create_from_bytes(&png_bytes(4, 4, 1)).unwrap();
    assert_eq!(again, e.lib().join("Maps/pasted-20270115-080000 (2)"));
    assert_eq!(
        c.create_from_bytes(b"nope").unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn fr_prj_11_progress_per_file_and_cancel_keeps_finished_projects() {
    let e = env();
    let a = e.outside("a.png", &png_bytes(4, 4, 1));
    let b = e.outside("b.png", &png_bytes(4, 4, 2));
    e.create(Kind::Map, &[a.clone(), b.clone()]);
    let names = e.events.names();
    assert_eq!(names.iter().filter(|n| *n == "import:progress").count(), 2);
    assert_eq!(names.last().unwrap(), "import:done");
    let cancel = AtomicBool::new(true);
    let c = e.outside("c.png", &png_bytes(4, 4, 3));
    let err = create_projects(&e.ctx, &e.settings, Kind::Map, &[c], &cancel, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Canceled);
    assert_eq!(err.detail["items"], json!([]));
    assert!(e.lib().join("Maps/a").is_dir() && !e.lib().join("Maps/c").exists());
}

#[test]
fn fr_prj_01_creation_needs_an_available_root() {
    let mut e = env();
    e.ctx.library_override = None;
    e.settings.library.roots = vec![e.dir.path().join("gone")];
    let err = create_projects(
        &e.ctx,
        &e.settings,
        Kind::Map,
        &[e.outside("a.png", &png_bytes(4, 4, 1))],
        &AtomicBool::new(false),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::LibraryUnavailable);
}

// ---------------------------------------------------------------- home grid and rename

#[test]
fn fr_prj_49_home_lists_the_section_newest_first_with_badges() {
    let mut e = env();
    let a = e.project("Older");
    e.ctx.clock = Box::new(crate::context::ManualClock::new(crate::time::Timestamp(T0 + 60_000)));
    let b = e.project("Newer");
    std::fs::create_dir(e.lib().join("Maps/not a project")).unwrap();
    std::fs::create_dir(e.lib().join("Maps/.x.pf-tmp-01")).unwrap();
    std::fs::write(b.join("project-OTHERPC.json"), b"{}").unwrap();
    let broken = e.lib().join("Maps/Broken");
    std::fs::create_dir(&broken).unwrap();
    std::fs::write(broken.join("project.json"), b"{ nope").unwrap();
    let cards = library::list_projects(&e.ctx, &e.settings, Kind::Map).unwrap();
    let names: Vec<_> = cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Newer", "Older", "Broken"]);
    assert_eq!(cards[0].badges, [CardBadge::Conflict]);
    assert_eq!(cards[1].path, a);
    assert!(cards[1].badges.is_empty() && cards[1].id.parse::<ulid::Ulid>().is_ok());
    assert_eq!(cards[2].badges, [CardBadge::Unreadable]);
    assert_eq!(cards[0].iterations, 0);
    assert!(library::list_projects(&e.ctx, &e.settings, Kind::Token).unwrap().is_empty());
}

#[test]
fn fr_prj_17_home_hydrates_only_manifest_and_thumb() {
    let e = env();
    let dir = e.project("Cloud");
    std::fs::write(dir.join(THUMB_FILE), jpeg_bytes(4, 4)).unwrap();
    let cloud: Vec<PathBuf> = [MANIFEST_FILE, THUMB_FILE, "working/Cloud.png"]
        .iter()
        .map(|f| dir.join(f))
        .collect();
    e.fs.cloud_only.lock().unwrap().extend(cloud.clone());
    let cards = library::list_projects(&e.ctx, &e.settings, Kind::Map).unwrap();
    assert!(cards[0].thumb.is_some());
    assert!(e.fs.is_cloud_only(&cloud[2]), "the original stays in the cloud");
    assert!(!e.fs.is_cloud_only(&cloud[0]) && !e.fs.is_cloud_only(&cloud[1]));
    e.fs.cloud_only.lock().unwrap().insert(cloud[0].clone());
    e.fs.offline.store(true, std::sync::atomic::Ordering::Relaxed);
    let cards = library::list_projects(&e.ctx, &e.settings, Kind::Map).unwrap();
    assert_eq!(cards[0].badges, [CardBadge::NotAvailableOffline]);
}

#[test]
fn fr_prj_45_rename_a_closed_project() {
    let e = env();
    let dir = e.project("Crypt");
    e.project("Taken");
    let rename = |name: &str, open: Option<&Path>| {
        library::rename_project(&e.ctx, &e.settings, &dir, name, open)
    };
    assert_eq!(rename("taken", None).unwrap_err().code, ErrorCode::NameTaken);
    assert_eq!(rename("a:b", None).unwrap_err().code, ErrorCode::InvalidName);
    assert_eq!(rename("CON", None).unwrap_err().code, ErrorCode::InvalidName);
    assert_eq!(rename("Vault", Some(&dir)).unwrap_err().code, ErrorCode::ProjectOpen);
    let before = std::fs::read(dir.join(MANIFEST_FILE)).unwrap();
    let new = rename("Vault", None).unwrap();
    assert_eq!(new, e.lib().join("Maps/Vault"));
    assert_eq!(std::fs::read(new.join(MANIFEST_FILE)).unwrap(), before, "nothing inside changes");
    e.open(&new);
    let case_only = library::rename_project(&e.ctx, &e.settings, &new, "VAULT", None).unwrap();
    assert!(case_only.ends_with("VAULT"));
}
