use super::*;
use crate::project::store::tests::{S, setup};
use crate::project::store::{NewCandidate, add_candidates};
use crate::test_support::*;

fn cand(seed: i64) -> NewCandidate {
    NewCandidate {
        bytes: rgba_png(16, 14, [seed as u8, 0, 0, 255]),
        seed: Some(seed),
        provider_meta: serde_json::json!({}),
        invalid: false,
        ..Default::default()
    }
}

fn succeeded(s: &mut S, prompt: &str) -> crate::job::JobFile {
    let job = s.job(prompt);
    for st in [JobState::Submitted, JobState::Running, JobState::Succeeded] {
        s.to(job.id, st);
    }
    job
}

fn running(s: &mut S, prompt: &str) -> crate::job::JobFile {
    let job = s.job(prompt);
    s.to(job.id, JobState::Submitted);
    job
}

fn to_running(s: &S, job: Ulid) {
    let u = SpoolUpdate {
        state: Some(JobState::Running),
        ..Default::default()
    };
    push_update(&s.e.ctx, s.p.manifest.project_id, job, &u).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    n.sort();
    n
}

#[test]
fn fr_prj_40_r102_an_entry_is_removed_json_first_and_keeps_its_bytes_if_that_fails() {
    let s = setup();
    let pid = s.p.manifest.project_id;
    let job = Ulid::from_parts(3, 3);
    push_candidates(&s.e.ctx, pid, job, &[cand(1)]).unwrap();
    let dir = spool_dir(&s.e.ctx, pid);
    let path = dir.join("1.json");
    let entry: Entry = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // The .json can't be removed (an AV lock): its bytes must stay with it.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    remove_entry(&dir, &entry, &path);
    assert!(dir.join("1-01.png").is_file(), "bytes kept with the entry");
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, serde_json::to_vec(&entry).unwrap()).unwrap();
    remove_entry(&dir, &entry, &path);
    assert!(names(&dir).is_empty());
}

#[test]
fn fr_prj_40_r102_an_entry_whose_bytes_are_gone_is_quarantined_and_later_ones_replay() {
    let mut s = setup();
    let pid = s.p.manifest.project_id;
    let done = succeeded(&mut s, "moss");
    let later = running(&mut s, "fern");
    push_candidates(&s.e.ctx, pid, done.id, &[cand(1)]).unwrap();
    to_running(&s, later.id);
    let dir = spool_dir(&s.e.ctx, pid);
    std::fs::remove_file(dir.join("1-01.png")).unwrap();

    let r = replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!((r.entries, r.quarantined.clone()), (1, vec![1]));
    assert_eq!(
        s.p.jobs[&later.id].state,
        JobState::Running,
        "later entry replayed"
    );
    assert!(!has_pending(&s.e.ctx, pid));
    assert_eq!(names(&dir.join(QUARANTINE_DIR)), ["1.json"]);
    assert!(
        s.e.events
            .names()
            .contains(&EVENT_SPOOL_QUARANTINED.to_owned())
    );
    // A new entry never reuses a quarantined seq.
    to_running(&s, later.id);
    assert!(dir.join("2.json").is_file());
}

#[test]
fn fr_prj_40_r102_candidates_whose_files_are_taken_are_quarantined_with_their_bytes() {
    let mut s = setup();
    let pid = s.p.manifest.project_id;
    let job = succeeded(&mut s, "moss");
    let later = running(&mut s, "fern");
    add_candidates(&s.e.ctx, s.e.install(), &mut s.p, job.id, vec![cand(1)]).unwrap();
    // A stale copy of the job (its `fetched` follow-up lost): add_candidates finds 01.png taken.
    s.p.jobs.get_mut(&job.id).unwrap().state = JobState::Succeeded;
    push_candidates(&s.e.ctx, pid, job.id, &[cand(2)]).unwrap();
    to_running(&s, later.id);

    let r = replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!((r.entries, r.quarantined.clone()), (1, vec![1]));
    assert_eq!(s.p.jobs[&later.id].state, JobState::Running);
    assert!(!has_pending(&s.e.ctx, pid));
    let q = spool_dir(&s.e.ctx, pid).join(QUARANTINE_DIR);
    assert_eq!(names(&q), ["1-01.png", "1.json"], "paid bytes kept");
}

#[test]
fn fr_prj_40_r102_an_entry_that_wont_parse_is_quarantined() {
    let mut s = setup();
    let pid = s.p.manifest.project_id;
    let later = running(&mut s, "fern");
    let dir = spool_dir(&s.e.ctx, pid);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("1.json"), b"{ trunc").unwrap();
    to_running(&s, later.id);
    let r = replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!((r.entries, r.quarantined.clone()), (1, vec![1]));
    assert!(!has_pending(&s.e.ctx, pid));
}

#[test]
fn fr_rgn_37_r109_candidates_of_a_job_not_yet_known_wait_instead_of_quarantine() {
    let mut s = setup();
    let pid = s.p.manifest.project_id;
    let job = succeeded(&mut s, "moss");
    // This machine can't read the job yet (job.json not synced here).
    let known = s.p.jobs.remove(&job.id).unwrap();
    push_candidates(&s.e.ctx, pid, job.id, &[cand(1)]).unwrap();
    let r = replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert!(r.quarantined.is_empty(), "{:?}", r.quarantined);
    assert!(r.dropped.is_empty());
    assert!(has_pending(&s.e.ctx, pid), "still waiting");
    // The job turns up: the next replay stores the candidates.
    s.p.jobs.insert(job.id, known);
    let r = replay(&s.e.ctx, s.e.install(), &mut s.p).unwrap();
    assert_eq!(r.fetched, vec![job.id]);
    assert!(!has_pending(&s.e.ctx, pid));
    assert_eq!(s.p.jobs[&job.id].state, JobState::Fetched);
}
