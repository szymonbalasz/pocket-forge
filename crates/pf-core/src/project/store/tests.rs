use super::*;
use crate::context::{FaultAction, Faults};
use crate::project::{open, restore_revision};
use crate::test_support::*;

pub(crate) struct S {
    pub e: Env,
    pub dir: std::path::PathBuf,
    pub p: OpenProject,
}

pub(crate) fn setup() -> S {
    let e = env();
    let dir = e.project("Crypt");
    let p = e.open(&dir);
    S { e, dir, p }
}

impl S {
    pub(crate) fn mask(&mut self) -> Mask {
        let rect = Rect {
            x: 4,
            y: 4,
            w: 8,
            h: 6,
        };
        put_mask(
            &self.e.ctx,
            self.e.install(),
            &mut self.p,
            rect,
            vec![255; 48],
        )
        .unwrap()
    }

    pub(crate) fn job(&mut self, prompt: &str) -> JobFile {
        let mask = self.mask();
        let new = NewJob {
            operation: "edit_masked".into(),
            model_id: "fake-model".into(),
            final_prompt: prompt.into(),
            candidate_count: 3,
            region: serde_json::json!({"inner": {"x": 4, "y": 4, "w": 8, "h": 6}, "outer": {"x": 0, "y": 0, "w": 16, "h": 14}, "margin": 4}),
            mask_id: Some(mask.id),
            ..Default::default()
        };
        record_job(
            &self.e.ctx,
            self.e.install(),
            &mut self.p,
            new,
            rgba_png(16, 14, [1, 2, 3, 255]),
            rgba_png(16, 14, [255, 255, 255, 255]),
        )
        .unwrap()
    }

    pub(crate) fn to(&mut self, job: Ulid, state: JobState) {
        update_job(
            &self.e.ctx,
            &mut self.p,
            job,
            JobUpdate {
                state: Some(state),
                provider: Some(serde_json::json!({"handle": "h-1"})),
                ..Default::default()
            },
        )
        .unwrap();
    }

    /// A job through to `fetched` with `n` candidates.
    pub(crate) fn fetched(&mut self, prompt: &str, n: usize) -> (JobFile, Vec<Candidate>) {
        let job = self.job(prompt);
        for s in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
            self.to(job.id, s);
        }
        let cands = (0..n)
            .map(|i| NewCandidate {
                bytes: rgba_png(16, 14, [i as u8 * 50, 0, 0, 255]),
                seed: Some(i as i64),
                provider_meta: serde_json::json!({"i": i}),
                invalid: false,
                ..Default::default()
            })
            .collect();
        let out =
            add_candidates(&self.e.ctx, self.e.install(), &mut self.p, job.id, cands).unwrap();
        (job, out)
    }

    pub(crate) fn patch() -> NewPatch {
        NewPatch {
            png: rgba_png(8, 6, [0, 255, 0, 255]),
            rect: Some(Rect {
                x: 4,
                y: 4,
                w: 8,
                h: 6,
            }),
            region: serde_json::json!({"inner": {"x": 4, "y": 4, "w": 8, "h": 6}}),
            blend: serde_json::json!({"feather_px": 4}),
        }
    }

    pub(crate) fn accept(&mut self, job: Ulid, cand: Ulid) -> CoreResult<LayerCommit> {
        accept_candidate(
            &self.e.ctx,
            self.e.install(),
            &mut self.p,
            job,
            cand,
            Self::patch(),
            None,
        )
    }

    pub(crate) fn reopen(&mut self) {
        self.p = self.e.open(&self.dir);
    }

    pub(crate) fn dismiss(&mut self, job: Ulid) {
        resolve_job(
            &self.e.ctx,
            self.e.install(),
            &mut self.p,
            job,
            ResolveReason::Dismiss,
        )
        .unwrap();
    }

    /// #56: "Discard the rest" (one trash item for the candidates).
    pub(crate) fn discard_rest(&mut self, job: Ulid) -> usize {
        discard_rest(&self.e.ctx, self.e.install(), &mut self.p, job).unwrap()
    }

    /// Dismiss, then discard every candidate: what a dismiss did before #56 (candidates and
    /// inputs in the trash).
    pub(crate) fn dismiss_and_discard(&mut self, job: Ulid) {
        self.dismiss(job);
        self.discard_rest(job);
    }

    pub(crate) fn cand(&self, id: Ulid) -> Candidate {
        self.p
            .manifest
            .candidates
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .clone()
    }
}

#[test]
fn fr_prj_28_record_job_writes_inputs_then_job_then_index() {
    let mut s = setup();
    let job = s.job("moss");
    let d = s.dir.join(job.dir());
    assert!(d.join("region.png").is_file() && d.join("mask.png").is_file());
    let on_disk: JobFile =
        serde_json::from_slice(&std::fs::read(d.join("job.json")).unwrap()).unwrap();
    assert_eq!(on_disk.state, JobState::Requested);
    assert_eq!(s.p.manifest.jobs[0].state, JobState::Requested);
    s.to(job.id, JobState::Submitted);
    let on_disk: JobFile =
        serde_json::from_slice(&std::fs::read(d.join("job.json")).unwrap()).unwrap();
    assert_eq!(
        on_disk.state,
        JobState::Submitted,
        "every transition persisted"
    );
    assert_eq!(on_disk.provider["handle"], "h-1", "provider kept verbatim");
    let err = update_job(
        &s.e.ctx,
        &mut s.p,
        job.id,
        JobUpdate {
            state: Some(JobState::Fetched),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    let err = update_job(
        &s.e.ctx,
        &mut s.p,
        job.id,
        JobUpdate {
            state: Some(JobState::Requested),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("state"));
}

#[test]
fn fr_prj_29_requested_job_reopened_is_lost_and_others_are_resumable() {
    let mut s = setup();
    let a = s.job("one");
    let b = s.job("two");
    s.to(b.id, JobState::Submitted);
    s.reopen();
    assert_eq!(s.p.jobs[&a.id].state, JobState::Lost);
    let lost = s.p.warnings.iter().find(|w| w.code == "lost").unwrap();
    assert_eq!(lost.detail["job_ids"][0], a.id.to_string());
    let res = s.p.warnings.iter().find(|w| w.code == "resumable").unwrap();
    assert_eq!(res.detail["jobs"][0]["state"], "submitted");
}

#[test]
fn fr_prj_30_fr_rgn_39_candidates_land_in_numbered_iteration_folders() {
    let mut s = setup();
    let (job, cands) = s.fetched("Kitchen, with pots!", 3);
    let files: Vec<_> = cands.iter().map(|c| c.file.as_str()).collect();
    assert_eq!(
        files,
        [
            "results/001 kitchen with pots/01.png",
            "results/001 kitchen with pots/02.png",
            "results/001 kitchen with pots/03.png"
        ]
    );
    assert!(s.dir.join(files[0]).is_file());
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
    assert_eq!(s.p.jobs[&job.id].candidate_ids.len(), 3);
    assert_eq!(s.p.manifest.jobs[0].iteration, Some(1));
    assert_eq!(s.p.manifest.iteration_count(), 1);
    assert_eq!(
        s.p.snapshot().candidates.len(),
        3,
        "R44: the snapshot carries candidates"
    );
    // A taken folder name gets " (2)"; numbering continues.
    std::fs::create_dir_all(s.dir.join("results/002 edit")).unwrap();
    let (job2, c2) = s.fetched("", 1);
    let _ = job2;
    assert_eq!(c2[0].file, "results/002 edit (2)/01.png");
}

#[test]
fn fr_rgn_24_undecodable_candidate_is_invalid() {
    let mut s = setup();
    let job = s.job("x");
    for st in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
        s.to(job.id, st);
    }
    let out = add_candidates(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        vec![
            NewCandidate {
                bytes: b"garbage".to_vec(),
                ..Default::default()
            },
            NewCandidate {
                bytes: rgba_png(4, 4, [0; 4]),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    assert_eq!(out[0].status, CandidateStatus::Invalid);
    assert_eq!(out[0].file, "results/001 x/01.bin");
    assert_eq!(out[1].status, CandidateStatus::New);
}

#[test]
fn fr_prj_31_owner_56_accept_is_one_commit_and_keeps_the_rest() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 3);
    set_star(&s.e.ctx, s.e.install(), &mut s.p, c[1].id, true).unwrap();
    let before_hash = s.p.manifest.original.as_ref().unwrap().content_hash.clone();
    let lc = s.accept(job.id, c[0].id).unwrap();
    let m = &s.p.manifest;
    let layer = m.layers.last().unwrap();
    assert_eq!((layer.id, layer.kind), (lc.layer_id, LayerKind::Patch));
    assert_eq!(
        layer.file.as_deref(),
        Some(layer_file(lc.layer_id).as_str())
    );
    assert_eq!((layer.cx, layer.cy), (8.0, 7.0));
    assert!(s.dir.join(layer_file(lc.layer_id)).is_file());
    let rev = m.revisions.last().unwrap();
    assert_eq!(
        (rev.id, rev.reason, rev.label.as_deref()),
        (lc.revision_id, RevisionReason::Accept, Some("moss"))
    );
    assert_eq!(s.cand(c[0].id).status, CandidateStatus::Accepted);
    assert_eq!(s.cand(c[1].id).status, CandidateStatus::Dismissed);
    assert!(s.cand(c[1].id).trash_id.is_none(), "starred stays");
    let rest = s.cand(c[2].id);
    assert_eq!(rest.status, CandidateStatus::Dismissed);
    assert!(rest.trash_id.is_none(), "#56: unstarred stays too");
    assert!(s.dir.join(&rest.file).is_file(), "still in results/");
    assert!(m.trash.is_empty());
    assert_eq!(s.p.jobs[&job.id].state, JobState::Closed);
    assert!(s.p.jobs[&job.id].resolved_at.is_some());
    assert!(
        s.dir.join(job.dir()).join("region.png").is_file(),
        "accepted jobs keep inputs (R9)"
    );
    assert_eq!(m.original.as_ref().unwrap().content_hash, before_hash);
    s.reopen();
    assert_eq!(s.p.manifest.layers.len(), 2);
}

#[test]
fn owner_56_dismiss_and_the_next_submit_keep_every_result() {
    let mut s = setup();
    let (job, c) = s.fetched("dust", 2);
    s.dismiss(job.id);
    for c in &c {
        let now = s.cand(c.id);
        assert_eq!(
            (now.status, now.trash_id),
            (CandidateStatus::Dismissed, None)
        );
        assert!(s.dir.join(&c.file).is_file(), "kept in results/");
    }
    assert!(
        s.dir.join(job.dir()).join("region.png").is_file(),
        "inputs kept"
    );
    assert!(s.p.manifest.trash.is_empty());
    // Unstarring a leftover no longer trashes it.
    set_star(&s.e.ctx, s.e.install(), &mut s.p, c[0].id, true).unwrap();
    set_star(&s.e.ctx, s.e.install(), &mut s.p, c[0].id, false).unwrap();
    assert!(s.dir.join(&c[0].file).is_file());
    assert!(s.p.manifest.trash.is_empty());
    // The next submit resolves the same way.
    let (second, c2) = s.fetched("moss", 1);
    resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        second.id,
        ResolveReason::NextSubmit,
    )
    .unwrap();
    assert!(s.dir.join(&c2[0].file).is_file());
    assert!(s.p.manifest.trash.is_empty());
}

#[test]
fn owner_56_job_inputs_are_trashed_only_when_every_candidate_is_discarded() {
    let mut s = setup();
    let (job, c) = s.fetched("dust", 2);
    s.dismiss(job.id);
    discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, c[0].id).unwrap();
    assert!(!s.dir.join(&c[0].file).exists(), "discard → trash");
    assert!(
        s.dir.join(job.dir()).join("region.png").is_file(),
        "one left"
    );
    discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, c[1].id).unwrap();
    assert!(!s.dir.join(job.dir()).join("region.png").exists());
    assert!(
        s.p.manifest
            .trash
            .iter()
            .any(|t| t.kind == TrashKind::JobInputs)
    );
    // An open (fetched) job keeps its inputs while all its candidates are discarded.
    let (open, d) = s.fetched("moss", 1);
    discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, d[0].id).unwrap();
    assert!(s.dir.join(open.dir()).join("region.png").is_file());
    s.dismiss(open.id);
    assert!(
        !s.dir.join(open.dir()).join("region.png").exists(),
        "on resolve"
    );
}

#[test]
fn owner_56_discard_rest_is_one_commit_and_skips_accepted_and_starred() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 4);
    set_star(&s.e.ctx, s.e.install(), &mut s.p, c[1].id, true).unwrap();
    s.accept(job.id, c[0].id).unwrap();
    let revs = s.p.manifest.revisions.len();
    let seq = s.p.manifest.trash.len();
    assert_eq!(s.discard_rest(job.id), 2);
    assert_eq!(s.p.manifest.trash.len(), seq + 1, "one trash item");
    assert_eq!(s.p.manifest.revisions.len(), revs, "no revision");
    assert_eq!(s.cand(c[0].id).status, CandidateStatus::Accepted);
    assert!(s.cand(c[1].id).starred && s.cand(c[1].id).trash_id.is_none());
    for i in [2, 3] {
        let x = s.cand(c[i].id);
        assert_eq!(x.status, CandidateStatus::Discarded);
        assert!(x.trash_id.is_some() && !s.dir.join(&x.file).exists());
    }
    assert!(
        s.dir.join(job.dir()).join("region.png").is_file(),
        "accepted keeps inputs"
    );
    assert_eq!(s.discard_rest(job.id), 0, "nothing left: no-op");
    // Undo = restore the trash item: the files come back as kept results.
    let tid = s.cand(c[2].id).trash_id.unwrap();
    crate::project::retention::trash_restore(&s.e.ctx, s.e.install(), &mut s.p, tid).unwrap();
    for i in [2, 3] {
        let x = s.cand(c[i].id);
        assert_eq!((x.status, x.trash_id), (CandidateStatus::Dismissed, None));
        assert!(s.dir.join(&x.file).is_file());
    }
}

#[test]
fn owner_56_projects_with_candidates_trashed_before_56_still_load_and_restore() {
    let mut s = setup();
    let (job, c) = s.fetched("dust", 2);
    s.dismiss(job.id);
    // What a dismiss wrote before #56: dismissed, `trash_id` set, file in the trash.
    let file = c[0].file.clone();
    let i = candidate_index(&s.p.manifest, c[0].id).unwrap();
    commit(&s.e.ctx, s.e.install(), &mut s.p, |ctx, _, m, _| {
        let tid = trash_files(
            ctx,
            m,
            TrashKind::Candidates,
            vec![file.clone()],
            "dismissed",
        );
        m.candidates[i].trash_id = Some(tid);
        Ok(())
    })
    .unwrap();
    assert!(!s.dir.join(&file).exists());
    s.reopen();
    assert_eq!(s.cand(c[0].id).status, CandidateStatus::Dismissed);
    // The old trashed one doesn't count as kept: discarding the other trashes the inputs.
    discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, c[1].id).unwrap();
    assert!(!s.dir.join(job.dir()).join("region.png").exists());
    // Star brings the old one back, as before.
    set_star(&s.e.ctx, s.e.install(), &mut s.p, c[0].id, true).unwrap();
    assert!(s.dir.join(&file).is_file());
}

#[test]
fn fr_prj_31_r9_discarding_everything_trashes_candidates_and_inputs_and_the_empty_folder() {
    let mut s = setup();
    let (job, c) = s.fetched("dust", 2);
    let folder = s.dir.join("results/001 dust");
    s.dismiss_and_discard(job.id);
    assert!(
        c.iter()
            .all(|c| s.cand(c.id).status == CandidateStatus::Discarded)
    );
    assert!(
        !folder.exists(),
        "an iteration folder left empty is removed"
    );
    assert!(!s.dir.join(job.dir()).join("region.png").exists());
    assert!(
        s.p.manifest
            .trash
            .iter()
            .any(|t| t.kind == TrashKind::JobInputs)
    );
    assert_eq!(s.p.jobs[&job.id].state, JobState::Closed);
    // Starring brings the file and its folder back (FR-PRJ-31).
    set_star(&s.e.ctx, s.e.install(), &mut s.p, c[0].id, true).unwrap();
    assert!(s.dir.join(&c[0].file).is_file());
    // Resolving again is a no-op.
    resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        ResolveReason::NextSubmit,
    )
    .unwrap();
}

#[test]
fn fr_rgn_28_discard_trashes_and_terminal_jobs_only_get_resolved_at() {
    let mut s = setup();
    let (_job, c) = s.fetched("x", 2);
    let d = discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, c[0].id).unwrap();
    assert_eq!(d.status, CandidateStatus::Discarded);
    assert!(!s.dir.join(&c[0].file).exists());
    let failed = s.job("y");
    s.to(failed.id, JobState::Failed);
    let r = resolve_job(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        failed.id,
        ResolveReason::Dismiss,
    )
    .unwrap();
    assert_eq!(r.state, JobState::Failed);
    assert!(r.resolved_at.is_some());
}

#[test]
fn fr_prj_43_r32_undo_accept_then_repick_restores_a_trashed_candidate() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 3);
    let lc = s.accept(job.id, c[0].id).unwrap();
    // #56: only an explicit discard trashes one.
    discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, c[2].id).unwrap();
    let parent = crate::project::read_revision(&s.p, lc.revision_id)
        .unwrap()
        .parent
        .unwrap();
    restore_revision(&s.e.ctx, s.e.install(), &mut s.p, parent).unwrap();
    assert_eq!(s.p.manifest.layers.len(), 1, "undo = restore the parent");
    assert_eq!(
        s.cand(c[0].id).status,
        CandidateStatus::Accepted,
        "statuses unchanged (R3)"
    );
    assert!(s.cand(c[2].id).trash_id.is_some());
    let again = s.accept(job.id, c[2].id).unwrap();
    assert_ne!(again.layer_id, lc.layer_id);
    let c2 = s.cand(c[2].id);
    assert_eq!((c2.status, c2.trash_id), (CandidateStatus::Accepted, None));
    assert!(
        s.dir.join(&c2.file).is_file(),
        "restored in the same commit"
    );
    assert_eq!(
        s.p.jobs[&job.id].state,
        JobState::Closed,
        "the job stays closed"
    );
    // Its patch is in the stack again: no second re-pick.
    assert_eq!(
        s.accept(job.id, c[1].id).unwrap_err().code,
        ErrorCode::RepickUnavailable
    );
}

#[test]
fn r32_no_repick_once_a_newer_job_exists() {
    let mut s = setup();
    let (job, c) = s.fetched("one", 2);
    let lc = s.accept(job.id, c[0].id).unwrap();
    let parent = crate::project::read_revision(&s.p, lc.revision_id)
        .unwrap()
        .parent
        .unwrap();
    restore_revision(&s.e.ctx, s.e.install(), &mut s.p, parent).unwrap();
    s.job("two");
    assert_eq!(
        s.accept(job.id, c[1].id).unwrap_err().code,
        ErrorCode::RepickUnavailable
    );
}

#[test]
fn fr_rgn_29_retune_makes_a_new_layer_and_keeps_the_old_file() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 1);
    let lc = s.accept(job.id, c[0].id).unwrap();
    let rt =
        replace_patch_layer(&s.e.ctx, s.e.install(), &mut s.p, lc.layer_id, S::patch()).unwrap();
    assert_ne!(rt.layer_id, lc.layer_id);
    let top = s.p.manifest.layers.last().unwrap();
    assert_eq!((top.id, top.retune_of), (rt.layer_id, Some(lc.layer_id)));
    assert_eq!(s.p.manifest.layers.len(), 2);
    assert_eq!(
        s.p.manifest.revisions.last().unwrap().reason,
        RevisionReason::Retune
    );
    assert!(s.dir.join(layer_file(lc.layer_id)).is_file());
    s.reopen();
    assert!(
        !s.p.manifest
            .trash
            .iter()
            .any(|t| t.files.contains(&layer_file(lc.layer_id))),
        "a layer a revision references is never swept"
    );
    assert_eq!(
        s.p.resolve_image(lc.layer_id).unwrap().kind,
        crate::project::ImageKind::Layer
    );
}

#[test]
fn fr_prj_19_r7_put_mask_points_the_draft_and_trashes_superseded_masks() {
    let mut s = setup();
    let a = s.mask();
    assert_eq!(s.p.manifest.draft.as_ref().unwrap().mask_id, Some(a.id));
    let png = image::open(s.dir.join(&a.file)).unwrap();
    assert_eq!((png.width(), png.height()), (8, 6));
    let b = s.mask();
    assert!(s.p.manifest.masks.iter().all(|k| k.id != a.id));
    assert!(
        s.p.manifest
            .trash
            .iter()
            .any(|t| t.kind == TrashKind::Mask && t.files == [a.file.clone()])
    );
    assert!(!s.dir.join(&a.file).exists());
    // A mask a job references is kept.
    let job = s.job("x");
    assert!(s.p.manifest.masks.iter().any(|k| Some(k.id) == job.mask_id));
    let _ = b;
    let err = put_mask(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        Rect {
            x: 38,
            y: 0,
            w: 8,
            h: 1,
        },
        vec![0; 8],
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRect);
    let err = put_mask(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        Rect {
            x: 0,
            y: 0,
            w: 2,
            h: 2,
        },
        vec![0; 3],
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::SizeMismatch);
}

#[test]
fn fr_exp_27_put_export_merges_into_the_export_record() {
    let mut s = setup();
    let f = |v: serde_json::Value| v.as_object().unwrap().clone();
    put_export(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        f(serde_json::json!({"format": "png"})),
    )
    .unwrap();
    put_export(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        f(serde_json::json!({"last": {"path": "exports/Crypt_70px.png"}})),
    )
    .unwrap();
    s.reopen();
    let e = s.p.manifest.export.unwrap();
    assert_eq!(
        (e["format"].as_str(), e["last"]["path"].as_str()),
        (Some("png"), Some("exports/Crypt_70px.png"))
    );
}

#[test]
fn fr_prj_20_kill_between_candidates_and_fetched_rolls_forward() {
    let mut s = setup();
    let job = s.job("moss");
    for st in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
        s.to(job.id, st);
    }
    // The `fetched` follow-up after the manifest commit fails.
    s.e.ctx.faults = Faults::parse("after_temp:job.json", FaultAction::Error).unwrap();
    let r = add_candidates(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        vec![NewCandidate {
            bytes: rgba_png(4, 4, [1, 1, 1, 255]),
            ..Default::default()
        }],
    );
    s.e.ctx.faults = Faults::none();
    // The iteration write before the files failed: nothing committed, retry works.
    assert!(r.is_err());
    let out = add_candidates(
        &s.e.ctx,
        s.e.install(),
        &mut s.p,
        job.id,
        vec![NewCandidate {
            bytes: rgba_png(4, 4, [1, 1, 1, 255]),
            ..Default::default()
        }],
    )
    .unwrap();
    // Now simulate a crash after the manifest: put job.json back to `succeeded`.
    let path = s.dir.join(job.dir()).join("job.json");
    let mut j: JobFile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    j.state = JobState::Succeeded;
    std::fs::write(&path, serde_json::to_vec(&j).unwrap()).unwrap();
    s.reopen();
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched, "rolled forward");
    assert_eq!(s.p.jobs[&job.id].candidate_ids, [out[0].id]);
}

#[test]
fn fr_prj_20_kill_after_accept_commit_closes_the_job_and_moves_trash_on_open() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 2);
    s.accept(job.id, c[0].id).unwrap();
    assert!(s.dir.join(&c[1].file).is_file(), "#56: the rest stays");
    discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, c[1].id).unwrap();
    // Undo the follow-ups on disk: job.json back to fetched, the trashed file back in results.
    let path = s.dir.join(job.dir()).join("job.json");
    let mut j: JobFile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    j.state = JobState::Fetched;
    std::fs::write(&path, serde_json::to_vec(&j).unwrap()).unwrap();
    let t = s.cand(c[1].id);
    let item =
        s.p.manifest
            .trash
            .iter()
            .find(|i| Some(i.id) == t.trash_id)
            .unwrap()
            .clone();
    std::fs::rename(s.dir.join(item.trashed_path(&t.file)), s.dir.join(&t.file)).unwrap();
    s.reopen();
    assert_eq!(s.p.jobs[&job.id].state, JobState::Closed);
    assert!(!s.dir.join(&t.file).exists() && s.dir.join(item.trashed_path(&t.file)).is_file());
}

#[test]
fn fr_prj_20_orphans_in_pforge_go_to_the_trash_and_user_files_stay() {
    let mut s = setup();
    let stray_layer = format!("{LAYERS_DIR}/{}.png", Ulid::from_parts(9, 9));
    std::fs::write(s.dir.join(&stray_layer), b"x").unwrap();
    std::fs::write(s.dir.join(".pforge/notes.txt"), b"x").unwrap();
    std::fs::write(s.dir.join("results/mine.png"), b"x").unwrap();
    std::fs::write(s.dir.join(".pforge/thumb.jpg"), jpeg_bytes(4, 4)).unwrap();
    s.reopen();
    let orphan =
        s.p.manifest
            .trash
            .iter()
            .find(|t| t.kind == TrashKind::Orphan)
            .unwrap();
    assert_eq!(
        orphan.files,
        [stray_layer.clone(), ".pforge/notes.txt".to_owned()]
    );
    assert!(!s.dir.join(&stray_layer).exists());
    assert!(
        s.dir.join("results/mine.png").exists(),
        "files the app didn't name stay"
    );
    assert!(s.dir.join(".pforge/thumb.jpg").exists());
}

#[test]
fn fr_prj_41_read_only_refuses_every_store_write() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 1);
    s.p.read_only = Some(crate::project::ReadOnlyReason::OriginalChanged);
    let ctx = &s.e.ctx;
    let id = s.e.install();
    let codes = [
        put_mask(
            ctx,
            id,
            &mut s.p,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            vec![0],
        )
        .unwrap_err()
        .code,
        record_job(ctx, id, &mut s.p, NewJob::default(), vec![], vec![])
            .unwrap_err()
            .code,
        update_job(ctx, &mut s.p, job.id, JobUpdate::default())
            .unwrap_err()
            .code,
        set_star(ctx, id, &mut s.p, c[0].id, true).unwrap_err().code,
        discard_candidate(ctx, id, &mut s.p, c[0].id)
            .unwrap_err()
            .code,
        accept_candidate(ctx, id, &mut s.p, job.id, c[0].id, S::patch(), None)
            .unwrap_err()
            .code,
        resolve_job(ctx, id, &mut s.p, job.id, ResolveReason::Dismiss)
            .unwrap_err()
            .code,
        put_export(ctx, id, &mut s.p, Map::new()).unwrap_err().code,
    ];
    assert!(codes.iter().all(|c| *c == ErrorCode::ReadOnly), "{codes:?}");
    // Reading still works.
    assert_eq!(read_project(&s.p).layers.len(), 1);
    let _ = open;
}

#[test]
fn r4_resolve_image_covers_candidates_and_their_trashed_copies() {
    let mut s = setup();
    let (job, c) = s.fetched("moss", 2);
    assert_eq!(
        s.p.resolve_image(c[1].id).unwrap().path,
        s.dir.join(&c[1].file)
    );
    s.accept(job.id, c[0].id).unwrap();
    assert_eq!(
        s.p.resolve_image(c[1].id).unwrap().path,
        s.dir.join(&c[1].file),
        "#56: kept in results/"
    );
    discard_candidate(&s.e.ctx, s.e.install(), &mut s.p, c[1].id).unwrap();
    let r = s.p.resolve_image(c[1].id).unwrap();
    assert!(r.path.is_file() && r.path.to_string_lossy().contains("trash"));
}
