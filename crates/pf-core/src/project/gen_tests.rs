//! GEN slice 2 (PRJ side): projects with no image, the `base` layer, decoded `layers[0]`, the
//! pick commit and `draft.generate` ([new-asset-generation](../../../../docs/specs/new-asset-generation.md)).

use serde_json::json;

use super::*;
use crate::context::{FaultAction, Faults};
use crate::core::Core;
use crate::create::{create_empty_project, suggest_name};
use crate::error::ErrorCode;
use crate::library::{self, CardBadge};
use crate::test_support::*;

fn core_with_empty(section: &str, name: &str) -> (tempfile::TempDir, Core, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = crate::context::Context::deterministic(dir.path().join("data"), Timestamp(T0));
    ctx.library_override = Some(dir.path().join("lib"));
    let core = Core::new(ctx).unwrap();
    let out = core
        .dispatch(
            "gen_project_create",
            json!({"section": section, "name": name}),
        )
        .unwrap();
    let path = out["path"].as_str().unwrap().to_owned();
    (dir, core, path)
}

// ---------------------------------------------------------------- FR-GEN-02 creation

#[test]
fn fr_gen_02_project_with_no_image_has_every_folder_null_original_and_r0() {
    let e = env();
    let dir = e.empty_project(Kind::Token, "Goblin archer");
    assert_eq!(dir, e.lib().join("Tokens/Goblin archer"));
    for d in PROJECT_DIRS {
        assert!(dir.join(d).is_dir(), "{d}");
    }
    assert!(
        std::fs::read_dir(dir.join(WORKING_DIR))
            .unwrap()
            .next()
            .is_none(),
        "no image is copied"
    );
    let v: Value =
        serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert_eq!(v["kind"], "token");
    assert_eq!(v["original"], Value::Null);
    assert_eq!(v["layers"], json!([]));
    let p = e.open(&dir);
    assert_eq!(p.manifest.revisions.len(), 1);
    let r0 = read_revision(&p, p.manifest.head_revision).unwrap();
    assert_eq!((r0.seq, r0.reason), (0, RevisionReason::Create));
    assert!(r0.layers.is_empty());
    // Nothing temp is left behind.
    assert!(
        std::fs::read_dir(e.lib().join("Tokens"))
            .unwrap()
            .flatten()
            .all(|en| !en.file_name().to_string_lossy().contains(fsx::TEMP_MARKER))
    );
}

#[test]
fn fr_gen_02_creation_is_atomic_temp_folder_then_rename() {
    let mut e = env();
    e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Error).unwrap();
    let err = create_empty_project(&e.ctx, &e.settings, Kind::Map, "Crypt", None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Io);
    assert_eq!(
        tree(&e.lib().join("Maps")),
        Vec::<String>::new(),
        "no partial project under any name"
    );
}

#[test]
fn fr_gen_02_name_rules_invalid_name_and_name_taken() {
    let e = env();
    let bad = create_empty_project(&e.ctx, &e.settings, Kind::Map, "  ", None).unwrap_err();
    assert_eq!(bad.code, ErrorCode::InvalidName);
    assert_eq!(bad.field.as_deref(), Some("name"));
    // The name is put under FR-PRJ-44, as the dialog shows it.
    let dir = create_empty_project(&e.ctx, &e.settings, Kind::Map, "a/b.", None).unwrap();
    assert_eq!(dir, e.lib().join("Maps/a-b"));
    e.project("Crypt");
    let taken = create_empty_project(&e.ctx, &e.settings, Kind::Map, "crypt", None).unwrap_err();
    assert_eq!(
        taken.code,
        ErrorCode::NameTaken,
        "case-insensitive (FR-PRJ-44)"
    );
    // Another section is free.
    create_empty_project(&e.ctx, &e.settings, Kind::Item, "Crypt", None).unwrap();
}

#[test]
fn fr_gen_01_prefilled_name_per_section_with_suffix_on_collision() {
    let e = env();
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Map, None),
        "New map"
    );
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Token, None),
        "New token"
    );
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Item, None),
        "New item"
    );
    e.empty_project(Kind::Map, "New map");
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Map, None),
        "New map (2)"
    );
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Map, Some("Crypt: lower?")),
        "Crypt- lower-"
    );
}

#[test]
fn fr_gen_02_commands_create_and_suggest() {
    let (_d, core, path) = core_with_empty("maps", "Crypt");
    assert!(path.ends_with("Crypt"));
    let again = core
        .dispatch(
            "gen_project_create",
            json!({"section": "map", "name": "Crypt"}),
        )
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NameTaken);
    let s = core
        .dispatch(
            "gen_project_suggest_name",
            json!({"section": "maps", "name": "Crypt"}),
        )
        .unwrap();
    assert_eq!(s["name"], "Crypt (2)");
    let unknown = core
        .dispatch(
            "gen_project_create",
            json!({"section": "maps", "name": "x", "extra": 1}),
        )
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------- FR-GEN-03 no image

#[test]
fn fr_gen_03_open_with_no_image_is_writable_and_snapshot_says_so() {
    let e = env();
    let dir = e.empty_project(Kind::Map, "Crypt");
    let p = e.open(&dir);
    assert_eq!(p.read_only, None, "no original to check");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let s = p.snapshot();
    assert!(!s.has_image);
    assert_eq!((s.image_path, s.original_image_id), (None, None));
    assert_eq!((s.width, s.height, s.format), (0, 0, None));
    assert!(s.layers.is_empty());
    let v = serde_json::to_value(p.snapshot()).unwrap();
    assert_eq!(v["original_image_id"], Value::Null);
    assert_eq!(v["has_image"], false);
}

#[test]
fn fr_gen_03_region_edit_masks_and_export_fail_with_no_image() {
    let (_d, core, path) = core_with_empty("maps", "Crypt");
    let snap = core
        .dispatch("project_open", json!({ "path": path }))
        .unwrap();
    let code = |name: &str, input: Value| core.dispatch(name, input).unwrap_err().code;
    assert_eq!(
        code("project_prefetch_original", json!({})),
        ErrorCode::NoImage
    );
    assert_eq!(
        code(
            "rgn_region_fit",
            json!({"project_id": snap["project_id"], "inner": {"x": 0, "y": 0, "w": 32, "h": 32}})
        ),
        ErrorCode::NoImage
    );
    assert_eq!(
        code(
            "edt_mask_submit",
            json!({"project_id": snap["project_id"], "rect": {"x": 0, "y": 0, "w": 2, "h": 2},
                "data_b64": "/////w=="})
        ),
        ErrorCode::NoImage
    );
    let settings = json!({"settings": {"grid": {"mode": "cells", "cols": 10, "rows": 10},
        "target_px_per_square": 70, "format": "png"}});
    assert_eq!(code("export_plan", settings.clone()), ErrorCode::NoImage);
    assert_eq!(code("export_start", settings.clone()), ErrorCode::NoImage);
    assert_eq!(code("export_set_settings", settings), ErrorCode::NoImage);
    // Store-level: the decoded handle says the same.
    let st = core.state();
    let p = st.project.as_ref().unwrap();
    assert_eq!(p.require_image().unwrap_err().code, ErrorCode::NoImage);
    assert_eq!(
        store::decoded_original(&core.ctx, p).unwrap_err().code,
        ErrorCode::NoImage
    );
}

#[test]
fn fr_gen_03_home_card_has_no_thumbnail_and_a_no_image_badge() {
    let e = env();
    let dir = e.empty_project(Kind::Item, "Sword");
    // A stale thumbnail (e.g. from before undoing the first pick) is not shown.
    std::fs::write(dir.join(THUMB_FILE), b"jpeg").unwrap();
    let cards = library::list_projects(&e.ctx, &e.settings, Kind::Item).unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].name, "Sword");
    assert_eq!(cards[0].thumb, None);
    assert_eq!(cards[0].badges, [CardBadge::NoImage]);
    assert_eq!(cards[0].iterations, 0);
    assert_eq!(
        serde_json::to_value(CardBadge::NoImage).unwrap(),
        "no_image"
    );
}

#[test]
fn fr_gen_03_rebuild_from_history_keeps_original_null() {
    let e = env();
    let dir = e.empty_project(Kind::Map, "Crypt");
    std::fs::remove_file(dir.join(MANIFEST_FILE)).unwrap();
    let p = e.open(&dir);
    assert!(p.warnings.iter().any(|w| w.code == "recovered"));
    assert_eq!(p.manifest.original, None);
    assert!(p.manifest.layers.is_empty());
}

// ---------------------------------------------------------------- FR-GEN-19 draft.generate

fn rgn_draft(prompt: &str) -> DraftInput {
    DraftInput {
        final_prompt: prompt.into(),
        model_id: Some("fake-inpaint".into()),
        ..Default::default()
    }
}

#[test]
fn fr_gen_19_new_image_save_and_region_edit_save_leave_each_other_alone() {
    let e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    put_draft(&e.ctx, e.install(), &mut p, rgn_draft("moss"), None).unwrap();
    let g = json!({"model_id": "fake-gen-wh", "size": {"mode": "grid", "cols": 30, "rows": 40, "pps": 70}});
    put_draft(
        &e.ctx,
        e.install(),
        &mut p,
        DraftInput::generate_only(Some(g.clone())),
        None,
    )
    .unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!(d.final_prompt, "moss", "the region-edit draft is untouched");
    assert_eq!(d.model_id.as_deref(), Some("fake-inpaint"));
    assert_eq!(d.generate.as_ref(), Some(&g));
    // A region-edit save (no `generate`) keeps the New image draft.
    put_draft(&e.ctx, e.install(), &mut p, rgn_draft("ivy"), None).unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!(
        (d.final_prompt.as_str(), d.generate.as_ref()),
        ("ivy", Some(&g))
    );
    // Durable, and `null` clears it.
    let p2 = e.open(&dir);
    assert_eq!(p2.manifest.draft.unwrap().generate, Some(g));
    put_draft(
        &e.ctx,
        e.install(),
        &mut p,
        DraftInput::generate_only(None),
        None,
    )
    .unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!((d.final_prompt.as_str(), d.generate), ("ivy", None));
}

#[test]
fn fr_gen_19_a_locked_new_image_save_survives_a_later_region_edit_save() {
    let mut e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    let g = json!({"model_id": "fake-gen-wh"});
    e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Locked).unwrap();
    let err = put_draft(
        &e.ctx,
        e.install(),
        &mut p,
        DraftInput::generate_only(Some(g.clone())),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    e.ctx.faults = Faults::none();
    put_draft(&e.ctx, e.install(), &mut p, rgn_draft("moss"), None).unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!(
        (d.final_prompt.as_str(), d.generate.as_ref()),
        ("moss", Some(&g))
    );
    assert!(p.pending_draft.is_none());
}

#[test]
fn fr_gen_19_put_draft_command_takes_generate_alone() {
    let (d, core, path) = core_with_empty("tokens", "Goblin");
    let _ = d;
    core.dispatch("project_open", json!({ "path": path }))
        .unwrap();
    core.dispatch(
        "project_put_draft",
        json!({"generate": {"model_id": "fake-gen-wh", "size": {"mode": "aspect", "aspect": "1:1"}}}),
    )
    .unwrap();
    let snap = core.dispatch("project_get", json!({})).unwrap();
    assert_eq!(snap["draft"]["generate"]["model_id"], "fake-gen-wh");
    assert_eq!(snap["draft"]["final_prompt"], "");
    let bad = core
        .dispatch("project_put_draft", json!({"generate": [1, 2]}))
        .unwrap_err();
    assert_eq!(
        (bad.code, bad.field.as_deref()),
        (ErrorCode::InvalidInput, Some("generate"))
    );
    // A region-edit save keeps it.
    core.dispatch("project_put_draft", json!({"draft": {"final_prompt": "x"}}))
        .unwrap();
    let snap = core.dispatch("project_get", json!({})).unwrap();
    assert_eq!(snap["draft"]["generate"]["model_id"], "fake-gen-wh");
    assert_eq!(snap["draft"]["final_prompt"], "x");
    // The New image tab's shape `{draft: {generate}}` merges too (GEN slice 5, G5).
    core.dispatch(
        "project_put_draft",
        json!({"draft": {"generate": {"model_id": "fake-gen-aspect"}}}),
    )
    .unwrap();
    let snap = core.dispatch("project_get", json!({})).unwrap();
    assert_eq!(snap["draft"]["generate"]["model_id"], "fake-gen-aspect");
    assert_eq!(
        snap["draft"]["final_prompt"], "x",
        "the region-edit draft is untouched"
    );
}

// ---------------------------------------------------------------- FR-GEN-27 … 30 the pick commit

use crate::job::{JobFile, JobState};
use crate::project::store::{
    JobUpdate, NewCandidate, NewJob, NewPatch, ResolveReason, pick_base, working_name,
};

struct G {
    e: Env,
    dir: std::path::PathBuf,
    p: OpenProject,
}

fn gen_env(kind: Kind, name: &str) -> G {
    let e = env();
    let dir = e.empty_project(kind, name);
    let p = e.open(&dir);
    G { e, dir, p }
}

/// A token-ish candidate: a coloured square, transparent in its left half.
fn token_png(shade: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_fn(32, 24, |x, _| {
        if x < 16 {
            image::Rgba([shade, 0, 0, 0])
        } else {
            image::Rgba([shade, 10, 20, 255])
        }
    })
    .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
    .unwrap();
    out
}

impl G {
    /// A generate job through to `fetched` with one candidate per shade (no provider call).
    fn generate(&mut self, typed: &str, shades: &[u8]) -> (JobFile, Vec<Candidate>) {
        let (ctx, install) = (&self.e.ctx, self.e.install());
        let mut extra = Extra::new();
        extra.insert("typed_prompt".into(), json!(typed));
        extra.insert("blend_instruction".into(), json!(false));
        let new = NewJob {
            operation: "generate".into(),
            model_id: "fake-gen-wh".into(),
            final_prompt: format!("{typed}, top-down"),
            candidate_count: shades.len() as u32,
            extra,
            ..Default::default()
        };
        let job = store::record_job(ctx, install, &mut self.p, new, vec![], vec![]).unwrap();
        for s in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
            store::update_job(
                ctx,
                &mut self.p,
                job.id,
                JobUpdate {
                    state: Some(s),
                    provider: Some(json!({"handle": "h-1"})),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let cands = shades
            .iter()
            .map(|&s| NewCandidate {
                bytes: token_png(s),
                seed: Some(i64::from(s)),
                ..Default::default()
            })
            .collect();
        let out = store::add_candidates(ctx, install, &mut self.p, job.id, cands).unwrap();
        (self.p.jobs[&job.id].clone(), out)
    }

    fn pick(&mut self, job: Ulid, cand: Ulid) -> CoreResult<store::BaseCommit> {
        pick_base(&self.e.ctx, self.e.install(), &mut self.p, job, cand)
    }

    fn status(&self, id: Ulid) -> CandidateStatus {
        self.p
            .manifest
            .candidates
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .status
    }

    /// A region edit accepted on top of the current image: one patch layer (FR-GEN-28).
    fn patch(&mut self) {
        let job = self.edit_job(&[JobState::Submitted, JobState::Running, JobState::Succeeded]);
        let c = self.edit_results(job.id);
        self.accept_edit(job.id, c[0].id).unwrap();
    }

    /// A region edit job on the current image, moved through `states`.
    fn edit_job(&mut self, states: &[JobState]) -> JobFile {
        let (ctx, install) = (&self.e.ctx, self.e.install());
        let mask = store::put_mask(ctx, install, &mut self.p, EDIT_RECT, vec![255; 48]).unwrap();
        let new = NewJob {
            operation: "edit_masked".into(),
            model_id: "fake-model".into(),
            final_prompt: "moss".into(),
            candidate_count: 1,
            region: json!({"inner": {"x": 2, "y": 2, "w": 8, "h": 6}}),
            mask_id: Some(mask.id),
            ..Default::default()
        };
        let png = edit_png();
        let job = store::record_job(ctx, install, &mut self.p, new, png.clone(), png).unwrap();
        for &s in states {
            store::update_job(
                ctx,
                &mut self.p,
                job.id,
                JobUpdate {
                    state: Some(s),
                    provider: Some(json!({"handle": "h-2"})),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        self.p.jobs[&job.id].clone()
    }

    /// The `succeeded` region edit job's one candidate (the job becomes `fetched`).
    fn edit_results(&mut self, job: Ulid) -> Vec<Candidate> {
        let one = vec![NewCandidate {
            bytes: edit_png(),
            ..Default::default()
        }];
        store::add_candidates(&self.e.ctx, self.e.install(), &mut self.p, job, one).unwrap()
    }

    fn accept_edit(&mut self, job: Ulid, cand: Ulid) -> CoreResult<store::LayerCommit> {
        store::accept_candidate(
            &self.e.ctx,
            self.e.install(),
            &mut self.p,
            job,
            cand,
            NewPatch {
                png: edit_png(),
                rect: Some(EDIT_RECT),
                region: json!({"inner": {"x": 2, "y": 2, "w": 8, "h": 6}}),
                blend: json!({"feather_px": 0}),
            },
            None,
        )
    }
}

const EDIT_RECT: Rect = Rect {
    x: 2,
    y: 2,
    w: 8,
    h: 6,
};

fn edit_png() -> Vec<u8> {
    rgba_png(8, 6, [0, 255, 0, 255])
}

#[test]
fn fr_gen_27_pick_is_one_commit_base_layer_write_once_copy_and_accept_revision() {
    let mut g = gen_env(Kind::Token, "Goblin archer");
    let (job, c) = g.generate(
        "goblin archer with a longbow, leather armour, hood up",
        &[10, 20, 30],
    );
    let seq_before = g.p.last_commit.unwrap().0;
    let out = g.pick(job.id, c[1].id).unwrap();
    assert_eq!(g.p.last_commit.unwrap().0, seq_before + 1, "one commit");
    // The candidate is copied write-once, untouched, to working/NNN <slug> NN.<ext>.
    assert_eq!(out.file, working_name(&c[1].file));
    assert!(
        out.file.starts_with("working/001 ") && out.file.ends_with(" 02.png"),
        "{}",
        out.file
    );
    assert_eq!(
        std::fs::read(g.dir.join(&out.file)).unwrap(),
        std::fs::read(g.dir.join(&c[1].file)).unwrap()
    );
    assert!(
        g.dir.join(&c[1].file).is_file(),
        "the candidate stays in results/"
    );
    // The stack is [base layer].
    let m = &g.p.manifest;
    assert_eq!(m.layers.len(), 1);
    let base = &m.layers[0];
    assert_eq!((base.id, base.kind), (out.layer_id, LayerKind::Base));
    assert_eq!(base.file.as_deref(), Some(out.file.as_str()));
    assert_eq!(
        base.content_hash.as_deref(),
        Some(fsx::hash_file(&g.dir.join(&out.file)).unwrap().as_str())
    );
    assert_eq!((base.width, base.height), (Some(32), Some(24)));
    assert_eq!(
        (base.job_id, base.candidate_id),
        (Some(job.id), Some(c[1].id))
    );
    assert_eq!(m.original, None, "no original appears");
    // Statuses: accepted, the other `new` ones dismissed (kept), the job closed.
    assert_eq!(
        c.iter().map(|x| g.status(x.id)).collect::<Vec<_>>(),
        [
            CandidateStatus::Dismissed,
            CandidateStatus::Accepted,
            CandidateStatus::Dismissed
        ]
    );
    assert!(g.dir.join(&c[0].file).is_file() && g.dir.join(&c[2].file).is_file());
    assert_eq!(g.p.jobs[&job.id].state, JobState::Closed);
    assert_eq!(
        m.jobs.iter().find(|r| r.id == job.id).unwrap().state,
        JobState::Closed
    );
    // One `accept` revision labelled "New: " + 40 chars of the typed prompt.
    let head = read_revision(&g.p, m.head_revision).unwrap();
    assert_eq!(
        (head.id, head.seq, head.reason),
        (out.revision_id, 1, RevisionReason::Accept)
    );
    assert_eq!(
        head.label.as_deref(),
        Some("New: goblin archer with a longbow, leather ar")
    );
    assert_eq!(head.layers, m.layers);
    assert_eq!(out.patches_dropped, 0);
    // The project has an image now, and it survives a reopen.
    let s = g.p.snapshot();
    assert!(s.has_image);
    assert_eq!(s.original_image_id, Some(out.layer_id));
    assert_eq!(s.image_path, Some(g.dir.join(&out.file)));
    assert_eq!(
        (s.width, s.height, s.format.as_deref()),
        (32, 24, Some("png"))
    );
    let again = g.e.open(&g.dir);
    assert_eq!(again.manifest.layers, g.p.manifest.layers);
    assert_eq!(again.read_only, None);
    let job_file: Value = serde_json::from_slice(
        &std::fs::read(g.dir.join(job.dir()).join(crate::job::JOB_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(job_file["state"], "closed");
    // EDT resolves the base image; the card has a thumbnail source again (no badge).
    assert_eq!(
        g.p.resolve_image(out.layer_id).unwrap().path,
        g.dir.join(&out.file)
    );
    let cards = library::list_projects(&g.e.ctx, &g.e.settings, Kind::Token).unwrap();
    assert!(cards[0].badges.is_empty(), "{:?}", cards[0].badges);
}

#[test]
fn fr_gen_33_region_edit_and_export_read_the_base_image_alpha_over_the_flatten_colour() {
    let mut g = gen_env(Kind::Token, "Goblin");
    let (job, c) = g.generate("goblin", &[200]);
    g.pick(job.id, c[0].id).unwrap();
    g.p.require_image().unwrap();
    let rgb = store::decoded_original(&g.e.ctx, &g.p).unwrap();
    assert_eq!(rgb.dimensions(), (32, 24));
    assert_eq!(
        rgb.get_pixel(3, 3).0,
        pf_imaging::FLATTEN_BG,
        "alpha 0 → #808080"
    );
    assert_eq!(rgb.get_pixel(20, 3).0, [200, 10, 20]);
    // The candidate and picked files keep their alpha (FR-GEN-12).
    let file = g.p.manifest.base_file().unwrap().to_owned();
    let kept = image::open(g.dir.join(file)).unwrap();
    assert!(kept.color().has_alpha());
}

#[test]
fn fr_gen_28_pick_drops_patch_layers_from_the_stack_and_undo_restores_them() {
    let mut g = gen_env(Kind::Map, "Crypt");
    let (job, c) = g.generate("flooded crypt", &[10, 20, 30]);
    let r1 = g.pick(job.id, c[1].id).unwrap();
    g.patch();
    assert_eq!(g.p.manifest.layers.len(), 2);
    let r2 = g.p.manifest.head_revision;
    // Re-pick #3 of the same (closed) job.
    let r3 = g.pick(job.id, c[2].id).unwrap();
    assert_eq!(r3.patches_dropped, 1);
    assert_eq!(g.p.manifest.layers.len(), 1);
    assert_eq!(g.p.manifest.layers[0].candidate_id, Some(c[2].id));
    assert_eq!(g.p.jobs[&job.id].state, JobState::Closed, "no transition");
    // Undo (FR-GEN-29): the parent revision's stack, patch included, then r1.
    let head = read_revision(&g.p, r3.revision_id).unwrap();
    assert_eq!(head.parent, Some(r2));
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, r2).unwrap();
    assert_eq!(g.p.manifest.layers.len(), 2);
    assert_eq!(g.p.manifest.layers[0].id, r1.layer_id);
    assert_eq!(g.p.manifest.layers[1].kind, LayerKind::Patch);
    let rgb = store::decoded_original(&g.e.ctx, &g.p).unwrap();
    assert_eq!(rgb.get_pixel(20, 3).0, [20, 10, 20], "decodes #2 again");
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, r1.revision_id).unwrap();
    assert_eq!(g.p.manifest.layers.len(), 1);
    // Both picked files stay in working/ (write-once, never deleted).
    assert!(g.dir.join(working_name(&c[1].file)).is_file());
    assert!(g.dir.join(working_name(&c[2].file)).is_file());
}

#[test]
fn fr_gen_29_undoing_the_first_pick_returns_to_no_image() {
    let mut g = gen_env(Kind::Item, "Sword");
    let r0 = g.p.manifest.head_revision;
    let (job, c) = g.generate("a sword", &[10, 20]);
    let out = g.pick(job.id, c[0].id).unwrap();
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, r0).unwrap();
    assert!(!g.p.manifest.has_image());
    assert!(g.p.manifest.layers.is_empty());
    assert_eq!(
        store::decoded_original(&g.e.ctx, &g.p).unwrap_err().code,
        ErrorCode::NoImage
    );
    // The picked image of a later revision still resolves (EDT's undo stack).
    assert_eq!(
        g.p.resolve_image(out.layer_id).unwrap().path,
        g.dir.join(&out.file)
    );
    let cards = library::list_projects(&g.e.ctx, &g.e.settings, Kind::Item).unwrap();
    assert_eq!(cards[0].badges, [CardBadge::NoImage]);
    // Redo.
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, out.revision_id).unwrap();
    assert_eq!(g.p.manifest.original_image_id(), Some(out.layer_id));
    assert!(store::decoded_original(&g.e.ctx, &g.p).is_ok());
}

#[test]
fn fr_gen_30_pick_from_an_older_iteration_and_restore_a_trashed_file() {
    let mut g = gen_env(Kind::Map, "Crypt");
    let (j1, c1) = g.generate("crypt one", &[10, 20, 30]);
    store::resolve_job(
        &g.e.ctx,
        g.e.install(),
        &mut g.p,
        j1.id,
        ResolveReason::NextSubmit,
    )
    .unwrap();
    let (j2, c2) = g.generate("crypt two", &[40, 50]);
    g.pick(j2.id, c2[0].id).unwrap();
    // An older, closed iteration: no transition, no spend.
    let out = g.pick(j1.id, c1[2].id).unwrap();
    assert_eq!(g.p.jobs[&j1.id].state, JobState::Closed);
    assert_eq!(g.status(c1[2].id), CandidateStatus::Accepted);
    assert!(out.file.starts_with("working/001 "), "{}", out.file);
    // A file a sweep moved to the trash is restored in the same commit.
    let victim = c1[0].clone();
    commit(&g.e.ctx, g.e.install(), &mut g.p, |ctx, _, m, _| {
        let id = store::trash_files(ctx, m, TrashKind::Orphan, vec![victim.file.clone()], "test");
        let c = m.candidates.iter_mut().find(|c| c.id == victim.id).unwrap();
        c.status = CandidateStatus::Trashed;
        c.trash_id = Some(id);
        Ok(())
    })
    .unwrap();
    assert!(!g.dir.join(&victim.file).exists(), "in the trash");
    g.pick(j1.id, victim.id).unwrap();
    assert!(g.dir.join(&victim.file).is_file(), "moved back to results/");
    assert!(g.dir.join(working_name(&victim.file)).is_file());
    let c =
        g.p.manifest
            .candidates
            .iter()
            .find(|c| c.id == victim.id)
            .unwrap();
    assert_eq!((c.status, c.trash_id), (CandidateStatus::Accepted, None));
    // Discarded candidates and other jobs' candidates can't be picked.
    store::discard_candidate(&g.e.ctx, g.e.install(), &mut g.p, c2[1].id).unwrap();
    assert_eq!(
        g.pick(j2.id, c2[1].id).unwrap_err().code,
        ErrorCode::InvalidCandidate
    );
    assert_eq!(
        g.pick(j1.id, c2[0].id).unwrap_err().code,
        ErrorCode::InvalidCandidate
    );
}

#[test]
fn fr_gen_27_a_crash_between_the_copy_and_the_manifest_reuses_the_copy() {
    let mut g = gen_env(Kind::Map, "Crypt");
    let (job, c) = g.generate("crypt", &[10, 20]);
    let before = g.p.manifest.clone();
    g.e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Error).unwrap();
    assert!(g.pick(job.id, c[0].id).is_err());
    g.e.ctx.faults = Faults::none();
    assert_eq!(
        g.p.manifest, before,
        "a failed pick leaves the project unchanged"
    );
    let copy = working_name(&c[0].file);
    assert!(
        g.dir.join(&copy).is_file(),
        "the unreferenced copy is harmless"
    );
    // Reopen (old state), pick again: the copy is reused by hash, not duplicated.
    g.p = g.e.open(&g.dir);
    assert!(!g.p.manifest.has_image());
    let out = g.pick(job.id, c[0].id).unwrap();
    assert_eq!(out.file, copy);
    assert_eq!(tree(&g.dir.join(WORKING_DIR)).len(), 1);
    // A different file under that name is never overwritten: the copy takes a free name.
    let (job2, c2) = g.generate("crypt", &[30]);
    let clash = working_name(&c2[0].file);
    std::fs::write(g.dir.join(&clash), b"another file").unwrap();
    let out2 = g.pick(job2.id, c2[0].id).unwrap();
    assert_ne!(out2.file, clash);
    assert!(out2.file.ends_with(" (2).png"), "{}", out2.file);
    assert_eq!(std::fs::read(g.dir.join(&clash)).unwrap(), b"another file");
}

#[test]
fn fr_gen_34_pick_fails_fast_when_read_only() {
    let mut g = gen_env(Kind::Map, "Crypt");
    let (job, c) = g.generate("crypt", &[10]);
    g.p.read_only = Some(ReadOnlyReason::NewerVersion);
    assert_eq!(
        g.pick(job.id, c[0].id).unwrap_err().code,
        ErrorCode::ReadOnly
    );
    assert!(
        !g.dir.join(working_name(&c[0].file)).exists(),
        "nothing written"
    );
}

#[test]
fn fr_gen_14_folder_files_carry_cloud_only_and_corrupt_states() {
    let e = env();
    let dir = e.empty_project(Kind::Map, "Crypt");
    let p = e.open(&dir);
    std::fs::write(dir.join("structure/ok.png"), png_bytes(8, 8, 1)).unwrap();
    std::fs::write(dir.join("structure/bad.png"), b"nope").unwrap();
    std::fs::write(dir.join("structure/cloud.png"), png_bytes(8, 8, 2)).unwrap();
    e.fs.cloud_only
        .lock()
        .unwrap()
        .insert(dir.join("structure/cloud.png"));
    let mut files = folder_list(&p, "structure").unwrap();
    set_file_states(&e.ctx, &mut files);
    let states: Vec<_> = files.iter().map(|f| (f.name.as_str(), f.state)).collect();
    assert_eq!(
        states,
        [
            ("bad.png", Some(FileState::Corrupt)),
            ("cloud.png", Some(FileState::CloudOnly)),
            ("ok.png", None)
        ]
    );
    let v = serde_json::to_value(&files[1]).unwrap();
    assert_eq!(v["state"], "cloud_only");
    assert!(
        serde_json::to_value(&files[2])
            .unwrap()
            .get("state")
            .is_none()
    );
}

#[test]
fn r84_fr_prj_15_a_changed_or_missing_base_image_opens_read_only() {
    let mut g = gen_env(Kind::Token, "Goblin archer");
    let (job, c) = g.generate("goblin archer", &[10, 20]);
    let out = g.pick(job.id, c[0].id).unwrap();
    // Unchanged: writable, no warning.
    let p = g.e.open(&g.dir);
    assert_eq!(p.read_only, None);
    assert!(!p.warnings.iter().any(|w| w.code == "base_changed"));
    // Saved over by another editor (other bytes, another size).
    let path = g.dir.join(&out.file);
    let kept = std::fs::read(&path).unwrap();
    std::fs::write(&path, crate::test_support::png_bytes(8, 8, 3)).unwrap();
    let p = g.e.open(&g.dir);
    assert_eq!(p.read_only, Some(ReadOnlyReason::BaseChanged));
    let w = p
        .warnings
        .iter()
        .find(|w| w.code == "base_changed")
        .unwrap();
    assert!(w.message.contains("read-only"), "{}", w.message);
    // Gone.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        g.e.open(&g.dir).read_only,
        Some(ReadOnlyReason::BaseChanged)
    );
    // Put back: writable again.
    std::fs::write(&path, kept).unwrap();
    assert_eq!(g.e.open(&g.dir).read_only, None);
}

// ---------------------------------------------------------------- phase 4 code review (R97)

#[test]
fn fr_gen_19_undo_and_redo_of_a_pick_keep_draft_generate() {
    let mut g = gen_env(Kind::Token, "Goblin");
    let r0 = g.p.manifest.head_revision;
    let first = json!({"model_id": "fake-gen-wh", "typed_prompt": "goblin"});
    put_draft(
        &g.e.ctx,
        g.e.install(),
        &mut g.p,
        DraftInput::generate_only(Some(first)),
        None,
    )
    .unwrap();
    let (job, c) = g.generate("goblin", &[10, 20]);
    let out = g.pick(job.id, c[0].id).unwrap();
    // The tab changed after the pick; r0 has no draft at all.
    let now = json!({"model_id": "fake-gen-wh", "typed_prompt": "goblin with a bow"});
    put_draft(
        &g.e.ctx,
        g.e.install(),
        &mut g.p,
        DraftInput::generate_only(Some(now.clone())),
        None,
    )
    .unwrap();
    let generate = |g: &G| g.p.manifest.draft.as_ref().and_then(|d| d.generate.clone());
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, r0).unwrap();
    assert_eq!(
        generate(&g),
        Some(now.clone()),
        "undo keeps the tab's draft"
    );
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, out.revision_id).unwrap();
    assert_eq!(generate(&g), Some(now.clone()), "redo keeps it too");
    // On disk: a reopen shows what the tab shows.
    let again = g.e.open(&g.dir);
    assert_eq!(again.manifest.draft.and_then(|d| d.generate), Some(now));
}

#[test]
fn fr_prj_23_restore_still_brings_back_the_region_edit_draft() {
    let mut g = gen_env(Kind::Map, "Crypt");
    put_draft(&g.e.ctx, g.e.install(), &mut g.p, rgn_draft("moss"), None).unwrap();
    save_revision(&g.e.ctx, g.e.install(), &mut g.p, None).unwrap();
    let (job, c) = g.generate("crypt", &[10]);
    let out = g.pick(job.id, c[0].id).unwrap();
    put_draft(&g.e.ctx, g.e.install(), &mut g.p, rgn_draft("ivy"), None).unwrap();
    let parent = read_revision(&g.p, out.revision_id)
        .unwrap()
        .parent
        .unwrap();
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, parent).unwrap();
    assert_eq!(g.p.manifest.draft.unwrap().final_prompt, "moss");
}

#[test]
fn fr_gen_30_r97_pick_is_refused_while_a_job_is_active() {
    let mut g = gen_env(Kind::Map, "Crypt");
    let (j1, c1) = g.generate("crypt", &[10, 20]);
    g.pick(j1.id, c1[0].id).unwrap();
    let head = g.p.manifest.head_revision;
    let running = g.edit_job(&[JobState::Submitted, JobState::Running]);
    let err = g.pick(j1.id, c1[1].id).unwrap_err();
    assert_eq!(err.code, ErrorCode::JobActive);
    assert_eq!(err.detail["job_id"], json!(running.id));
    assert_eq!(g.p.manifest.head_revision, head, "nothing committed");
    assert_eq!(g.p.manifest.layers[0].candidate_id, Some(c1[0].id));
    // Fetched (results in, nothing running): picking works again.
    store::update_job(
        &g.e.ctx,
        &mut g.p,
        running.id,
        JobUpdate {
            state: Some(JobState::Succeeded),
            ..Default::default()
        },
    )
    .unwrap();
    g.edit_results(running.id);
    g.pick(j1.id, c1[1].id).unwrap();
}

#[test]
fn fr_rgn_40_r97_accept_refuses_a_candidate_made_on_another_base_image() {
    let mut g = gen_env(Kind::Map, "Crypt");
    let (j1, c1) = g.generate("crypt", &[10, 20]);
    let b1 = g.pick(j1.id, c1[0].id).unwrap();
    let edit = g.edit_job(&[JobState::Submitted, JobState::Running, JobState::Succeeded]);
    assert_eq!(
        edit.base_layer_id,
        Some(b1.layer_id),
        "the job remembers its base"
    );
    let rc = g.edit_results(edit.id);
    // Another base image picked while the results wait.
    g.pick(j1.id, c1[1].id).unwrap();
    let head = g.p.manifest.head_revision;
    let err = g.accept_edit(edit.id, rc[0].id).unwrap_err();
    assert_eq!(
        (err.code, err.field.as_deref()),
        (ErrorCode::InvalidCandidate, Some("candidate_id"))
    );
    assert!(
        err.message.contains("different base image"),
        "{}",
        err.message
    );
    // The candidate stays; nothing committed.
    assert_eq!(g.status(rc[0].id), CandidateStatus::New);
    assert_eq!(g.p.manifest.head_revision, head);
    assert_eq!(g.p.manifest.layers.len(), 1);
    // Undo the second pick: B1 is back, and the candidate can be accepted on it.
    let parent = read_revision(&g.p, head).unwrap().parent.unwrap();
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, parent).unwrap();
    g.accept_edit(edit.id, rc[0].id).unwrap();
    assert_eq!(g.status(rc[0].id), CandidateStatus::Accepted);
}

#[test]
fn r84_r97_candidate_statuses_follow_undo_and_redo_of_a_pick() {
    use CandidateStatus::*;
    let mut g = gen_env(Kind::Map, "Crypt");
    let r0 = g.p.manifest.head_revision;
    let (job, c) = g.generate("crypt", &[10, 20, 30]);
    let first = g.pick(job.id, c[0].id).unwrap();
    let second = g.pick(job.id, c[1].id).unwrap();
    let statuses = |g: &G| c.iter().map(|x| g.status(x.id)).collect::<Vec<_>>();
    assert_eq!(statuses(&g), [Dismissed, Accepted, Dismissed]);
    // Undo the re-pick: #1 is the base again, and the only accepted one.
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, first.revision_id).unwrap();
    assert_eq!(statuses(&g), [Accepted, Dismissed, Dismissed]);
    // Undo the first pick: no image, nothing accepted.
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, r0).unwrap();
    assert_eq!(statuses(&g), [Dismissed, Dismissed, Dismissed]);
    // #2 discarded meanwhile; redo to the re-pick brings it back from the trash, accepted.
    store::discard_candidate(&g.e.ctx, g.e.install(), &mut g.p, c[1].id).unwrap();
    assert_eq!(g.status(c[1].id), Discarded);
    restore_revision(&g.e.ctx, g.e.install(), &mut g.p, second.revision_id).unwrap();
    assert_eq!(statuses(&g), [Dismissed, Accepted, Dismissed]);
    let c1 =
        g.p.manifest
            .candidates
            .iter()
            .find(|x| x.id == c[1].id)
            .unwrap();
    assert_eq!(c1.trash_id, None);
    assert!(g.dir.join(&c1.file).is_file(), "moved back to results/");
}
