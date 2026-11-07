use super::*;
use crate::context::{FaultAction, Faults};

struct Env {
    dir: tempfile::TempDir,
    ctx: Context,
    settings: Settings,
}

fn png_bytes(w: u32, h: u32, shade: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&vec![shade; (w * h * 3) as usize])
        .unwrap();
    wr.finish().unwrap();
    out
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
    ctx.library_override = Some(dir.path().join("lib"));
    crate::library::prepare_override(&ctx).unwrap();
    let settings = Settings::defaults(&ctx);
    Env { dir, ctx, settings }
}

impl Env {
    fn image(&self, rel: &str) -> PathBuf {
        let p = self.dir.path().join("lib").join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, png_bytes(40, 30, 7)).unwrap();
        p
    }
    fn open(&self, p: &Path) -> OpenProject {
        open(&self.ctx, &self.settings, p).unwrap()
    }
    fn install(&self) -> Ulid {
        self.settings.install_id
    }
    fn draft(&self, p: &mut OpenProject, prompt: &str) -> CoreResult<u64> {
        put_draft(
            &self.ctx,
            self.install(),
            p,
            DraftInput {
                final_prompt: prompt.into(),
                ..Default::default()
            },
        )
    }
}

fn tree(dir: &Path) -> Vec<String> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.path().is_dir() {
            out.extend(tree(&e.path()).into_iter().map(|c| format!("{name}/{c}")));
        } else {
            out.push(name);
        }
    }
    out.sort();
    out
}

#[test]
fn fr_prj_13_opening_an_image_writes_nothing() {
    let e = env();
    let img = e.image("dungeons/crypt.png");
    let before = tree(&e.dir.path().join("lib"));
    let p = e.open(&img);
    assert!(!p.on_disk);
    assert_eq!(p.snapshot().head_revision, None);
    assert_eq!(
        (p.manifest.original.width, p.manifest.original.height),
        (40, 30)
    );
    assert_eq!(tree(&e.dir.path().join("lib")), before);
}

#[test]
fn fr_prj_13_first_commit_creates_the_folder_and_r0() {
    let e = env();
    let img = e.image("dungeons/crypt.png");
    let mut p = e.open(&img);
    let rev = save_revision(&e.ctx, e.install(), &mut p, Some("first".into())).unwrap();
    assert_eq!(rev.seq, 0);
    assert_eq!(rev.reason, RevisionReason::Create);
    let dir = e.dir.path().join("lib/dungeons/crypt.png.pforge");
    let files = tree(&dir);
    assert_eq!(
        files,
        vec![
            "manifest.json".to_owned(),
            format!("revisions/{}.json", rev.id)
        ]
    );
    let m: Manifest =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m.commit.seq, 1);
    assert_eq!(m.head_revision, rev.id);
    assert_eq!(m.original.content_hash, fsx::hash_file(&img).unwrap());
}

#[test]
fn fr_prj_12_project_files_hold_only_relative_paths() {
    let e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    let text = std::fs::read_to_string(p.dir.join("manifest.json")).unwrap();
    assert!(text.contains(r#""file": "../crypt.png""#), "{text}");
    let root = e.dir.path().to_string_lossy().replace('\\', "\\\\");
    assert!(!text.contains(&*root), "absolute path leaked: {text}");
}

#[test]
fn fr_prj_14_opening_the_pforge_folder_or_the_image_opens_the_project() {
    let e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    e.draft(&mut p, "moss").unwrap();
    let via_image = e.open(&img);
    let via_dir = e.open(&p.dir);
    assert_eq!(via_image.manifest, p.manifest);
    assert_eq!(via_dir.manifest, p.manifest);
    assert_eq!(via_dir.image_path, img);
}

#[test]
fn fr_prj_22_save_close_reopen_is_field_identical() {
    let e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    e.draft(&mut p, "a mossy crypt").unwrap();
    let reopened = e.open(&img);
    assert!(reopened.on_disk);
    assert_eq!(reopened.manifest, p.manifest);
    assert_eq!(reopened.snapshot(), p.snapshot());
    assert_eq!(
        reopened.manifest.draft.as_ref().unwrap().final_prompt,
        "a mossy crypt"
    );
}

#[test]
fn fr_prj_19_first_draft_commit_creates_r0_with_the_draft() {
    let e = env();
    let mut p = e.open(&e.image("crypt.png"));
    let seq = e.draft(&mut p, "x").unwrap();
    assert_eq!(seq, 1);
    let r0 = read_revision(&p, p.manifest.head_revision).unwrap();
    assert_eq!(r0.reason, RevisionReason::Create);
    assert_eq!(r0.draft.unwrap().final_prompt, "x");
    assert_eq!(e.draft(&mut p, "y").unwrap(), 2);
    assert_eq!(
        p.manifest.revisions.len(),
        1,
        "a draft save is not a revision"
    );
}

#[test]
fn fr_prj_23_save_only_if_changed_since_the_head() {
    let e = env();
    let mut p = e.open(&e.image("crypt.png"));
    let r0 = save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    let err = save_revision(&e.ctx, e.install(), &mut p, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unchanged);
    e.draft(&mut p, "changed").unwrap();
    let r1 = save_revision(&e.ctx, e.install(), &mut p, Some("mine".into())).unwrap();
    assert_eq!(
        (r1.seq, r1.reason, r1.label.as_deref()),
        (1, RevisionReason::Save, Some("mine"))
    );
    assert_eq!(read_revision(&p, r1.id).unwrap().parent, Some(r0.id));
}

#[test]
fn fr_prj_20_crash_after_manifest_temp_reopens_the_state_before() {
    let mut e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    e.draft(&mut p, "before").unwrap();
    let committed = e.open(&img).manifest;
    e.ctx.faults = Faults::parse("after_temp:manifest.json", FaultAction::Error).unwrap();
    assert!(e.draft(&mut p, "after").is_err());
    assert_eq!(
        p.manifest, committed,
        "in-memory state unchanged on failure"
    );
    // Simulate the kill leaving the temp file behind.
    std::fs::write(p.dir.join(".manifest.json.pf-tmp-x"), b"partial").unwrap();
    e.ctx.faults = Faults::none();
    let reopened = e.open(&img);
    assert_eq!(reopened.manifest, committed);
    assert!(
        !tree(&p.dir).iter().any(|f| f.contains(fsx::TEMP_MARKER)),
        "temp files removed on open"
    );
}

#[test]
fn fr_prj_20_crash_after_rename_reopens_the_state_after() {
    let mut e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    e.draft(&mut p, "before").unwrap();
    e.ctx.faults = Faults::parse("after_rename:manifest.json", FaultAction::Error).unwrap();
    let _ = e.draft(&mut p, "after");
    e.ctx.faults = Faults::none();
    let reopened = e.open(&img);
    assert_eq!(
        reopened.manifest.draft.unwrap().final_prompt,
        "after",
        "the rename is the commit point"
    );
}

#[test]
fn fr_prj_20_crash_during_the_first_commit_reopens_as_new() {
    let mut e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    e.ctx.faults = Faults::parse("after_temp", FaultAction::Error).unwrap();
    assert!(save_revision(&e.ctx, e.install(), &mut p, None).is_err());
    e.ctx.faults = Faults::none();
    std::fs::write(p.dir.join(".manifest.json.pf-tmp-y"), b"partial").unwrap();
    let reopened = e.open(&img);
    assert!(!reopened.on_disk);
    assert!(tree(&p.dir).iter().all(|f| !f.contains(fsx::TEMP_MARKER)));
    let mut reopened = reopened;
    save_revision(&e.ctx, e.install(), &mut reopened, None).unwrap();
    assert!(e.open(&img).on_disk);
}

#[test]
fn fr_prj_26_newer_schema_opens_read_only_and_writes_nothing() {
    let e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    let path = p.dir.join("manifest.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["schema_version"] = 2.into();
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut p = e.open(&img);
    assert_eq!(p.read_only, Some(ReadOnlyReason::NewerVersion));
    let err = e.draft(&mut p, "x").unwrap_err();
    assert_eq!(err.code, ErrorCode::ReadOnly);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn fr_prj_15_changed_original_opens_read_only() {
    let e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    std::fs::write(&img, png_bytes(40, 30, 200)).unwrap();
    let p = e.open(&img);
    assert_eq!(p.read_only, Some(ReadOnlyReason::OriginalChanged));
}

#[test]
fn fr_prj_15_touched_but_identical_original_stays_writable() {
    let e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    std::fs::write(&img, png_bytes(40, 30, 7)).unwrap(); // same bytes, new mtime
    assert_eq!(e.open(&img).read_only, None);
}

#[test]
fn fr_prj_16_missing_original_opens_read_only_with_a_warning() {
    let e = env();
    let img = e.image("crypt.png");
    let mut p = e.open(&img);
    save_revision(&e.ctx, e.install(), &mut p, None).unwrap();
    std::fs::remove_file(&img).unwrap();
    let p = e.open(&p.dir);
    assert_eq!(p.read_only, Some(ReadOnlyReason::OriginalMissing));
    assert_eq!(p.warnings[0].code, "original_missing");
}

#[test]
fn fr_prj_35_seq_mismatch_refuses_the_write() {
    let e = env();
    let img = e.image("crypt.png");
    let mut a = e.open(&img);
    e.draft(&mut a, "one").unwrap();
    let mut b = e.open(&img);
    e.draft(&mut b, "two").unwrap();
    let err = e.draft(&mut a, "three").unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(e.open(&img).manifest.draft.unwrap().final_prompt, "two");
}

#[test]
fn fr_prj_17_cloud_only_original_offline_is_not_available() {
    let e = env();
    let img = e.image("crypt.png");
    // The deterministic context uses FakeFsAttributes; reach it through a fresh context.
    let fs = crate::context::FakeFsAttributes::default();
    fs.cloud_only.lock().unwrap().insert(img.clone());
    fs.offline.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut ctx = Context::deterministic(e.ctx.data_dir.clone(), Timestamp(0));
    ctx.library_override = e.ctx.library_override.clone();
    ctx.fs = Box::new(fs);
    let err = open(&ctx, &e.settings, &img).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotAvailableOffline);
    assert!(!project_dir_for(&img).exists());
}

#[test]
fn outside_library_is_refused() {
    let e = env();
    let outside = e.dir.path().join("elsewhere.png");
    std::fs::write(&outside, png_bytes(4, 4, 1)).unwrap();
    let err = open(&e.ctx, &e.settings, &outside).unwrap_err();
    assert_eq!(err.code, ErrorCode::OutsideLibrary);
}

#[test]
fn unsupported_image_is_refused_on_open() {
    let e = env();
    let p = e.dir.path().join("lib/notes.png");
    std::fs::write(&p, b"not an image").unwrap();
    assert_eq!(
        open(&e.ctx, &e.settings, &p).unwrap_err().code,
        ErrorCode::Unsupported
    );
}
