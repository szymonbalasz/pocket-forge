//! FR-PRJ-56 … 58 (#62): rename of the open project, delete to the library trash, Recently
//! deleted, restore and the 30-day purge. A real asset project on disk, the fake provider, the
//! clock and fault seams. Generated images only (KQ-22).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::context::{FaultAction, Faults};
use crate::error::{CoreError, ErrorCode};
use crate::region_edit::job_tests::Lab;
use crate::test_support::rename_dir;

fn set_faults(lab: &mut Lab, spec: Option<(&str, FaultAction)>) {
    Arc::get_mut(&mut lab.core.ctx).unwrap().faults = match spec {
        Some((s, action)) => Faults::parse(s, action).unwrap(),
        None => Faults::none(),
    };
}

fn rename(lab: &Lab, name: &str) -> Result<PathBuf, CoreError> {
    lab.core
        .dispatch(
            "project_rename",
            json!({"path": lab.project_dir.to_string_lossy(), "name": name}),
        )
        .map(|v| PathBuf::from(v["path"].as_str().unwrap()))
}

fn get(lab: &Lab) -> Value {
    lab.core.dispatch("project_get", json!({})).unwrap()
}

fn put_draft(lab: &Lab, prompt: &str) -> Result<Value, CoreError> {
    lab.core.dispatch(
        "project_put_draft",
        json!({"draft": {"final_prompt": prompt}}),
    )
}

/// A served URL of the open project's original, as the UI holds it.
fn image_url(lab: &Lab) -> String {
    let snap = get(lab);
    let img = lab
        .core
        .dispatch(
            "edt_image_open",
            json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"]}),
        )
        .unwrap();
    let url = img["url"].as_str().unwrap();
    url[url.find("/i/").unwrap()..].to_owned()
}

fn sibling(lab: &Lab, name: &str) -> PathBuf {
    lab.project_dir.parent().unwrap().join(name)
}

/// What a refused rename must leave: the folder where it was and the project open in it.
fn assert_unchanged(lab: &Lab, before: &Value) {
    assert!(lab.project_dir.is_dir(), "the folder stays");
    let now = get(lab);
    assert_eq!(now["project_dir"], before["project_dir"]);
    assert_eq!(now["project_id"], before["project_id"]);
    assert_eq!(now["revisions"], before["revisions"]);
}

#[test]
fn fr_prj_56_rename_while_open_keeps_the_project_and_stays_open() {
    let lab = Lab::new();
    put_draft(&lab, "moss on the stones").unwrap();
    lab.core
        .dispatch("project_save_revision", json!({}))
        .unwrap();
    let before = get(&lab);
    let url = image_url(&lab);
    assert_eq!(lab.core.serve_scheme(&url).status, 200);

    let new = rename(&lab, "Vault").unwrap();
    assert_eq!(new, sibling(&lab, "Vault"));
    assert!(new.join("project.json").is_file() && !lab.project_dir.exists());

    let after = get(&lab);
    assert_eq!(after["name"], "Vault");
    assert_eq!(
        after["project_dir"].as_str().unwrap(),
        new.to_string_lossy()
    );
    for same in ["project_id", "revisions", "head_revision", "candidates"] {
        assert_eq!(after[same], before[same], "{same}");
    }
    assert_eq!(after["draft"]["final_prompt"], "moss on the stones");
    assert_eq!(after["draft"]["mask_id"], before["draft"]["mask_id"]);
    assert_eq!(after["read_only"], Value::Null);
    assert!(
        !after["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "folder_moved"),
        "no folder-moved banner"
    );
    assert_eq!(
        lab.core.serve_scheme(&url).status,
        200,
        "a URL issued before the rename still serves"
    );
    // It is open for writing where it is now.
    put_draft(&lab, "ivy").unwrap();
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(new.join("project.json")).unwrap()).unwrap();
    assert_eq!(on_disk["draft"]["final_prompt"], "ivy");
    assert!(!lab.project_dir.exists(), "the old folder isn't recreated");
}

#[test]
fn fr_prj_56_the_same_name_changes_nothing() {
    let lab = Lab::new();
    let before = get(&lab);
    assert_eq!(rename(&lab, "rooms").unwrap(), lab.project_dir);
    assert_unchanged(&lab, &before);
}

#[test]
fn fr_prj_56_invalid_and_taken_names_are_refused_with_nothing_changed() {
    let lab = Lab::new();
    std::fs::create_dir(sibling(&lab, "Taken")).unwrap();
    let before = get(&lab);
    let url = image_url(&lab);
    for (name, code) in [
        ("a:b", ErrorCode::InvalidName),
        ("CON", ErrorCode::InvalidName),
        ("", ErrorCode::InvalidName),
        ("taken", ErrorCode::NameTaken),
    ] {
        let err = rename(&lab, name).unwrap_err();
        assert_eq!(err.code, code, "{name:?}");
        assert_eq!(err.field.as_deref(), Some("name"), "{name:?}");
        assert_unchanged(&lab, &before);
    }
    assert_eq!(lab.core.serve_scheme(&url).status, 200);
}

#[test]
fn fr_prj_56_an_active_job_refuses_the_rename() {
    let lab = Lab::new();
    let job = lab.submit("a mossy well");
    let before = get(&lab);
    let err = rename(&lab, "Vault").unwrap_err();
    assert_eq!(err.code, ErrorCode::JobActive);
    assert_eq!(err.detail["job_id"], json!(job));
    assert_unchanged(&lab, &before);
    // Once the job has finished the rename goes through.
    lab.drive();
    let new = rename(&lab, "Vault").unwrap();
    assert!(new.is_dir());
}

#[test]
fn fr_prj_56_a_waiting_save_that_cant_commit_refuses_the_rename() {
    let mut lab = Lab::new();
    let before = get(&lab);
    set_faults(
        &mut lab,
        Some(("after_temp:project.json", FaultAction::Locked)),
    );
    assert_eq!(
        put_draft(&lab, "waits").unwrap_err().code,
        ErrorCode::Locked
    );
    let err = rename(&lab, "Vault").unwrap_err();
    assert_eq!(err.code, ErrorCode::Unsaved);
    assert_eq!(err.detail["reason"], "locked");
    assert_unchanged(&lab, &before);
    assert!(
        crate::project::has_unsaved(lab.core.state().project.as_ref().unwrap()),
        "the draft still waits: no force"
    );
    // The lock lifts: the rename saves the waiting draft first, then renames.
    set_faults(&mut lab, None);
    let new = rename(&lab, "Vault").unwrap();
    assert_eq!(get(&lab)["draft"]["final_prompt"], "waits");
    assert!(new.is_dir());
}

#[test]
fn fr_prj_56_a_folder_already_moved_refuses_the_rename() {
    let lab = Lab::new();
    let elsewhere = sibling(&lab, "Elsewhere");
    rename_dir(&lab.project_dir, &elsewhere);
    let err = rename(&lab, "Vault").unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(err.detail["reason"], "folder_moved");
    assert!(elsewhere.is_dir() && !sibling(&lab, "Vault").exists());
    assert!(lab.core.state().project.is_some(), "still open in memory");
}

#[test]
fn fr_prj_56_a_failed_folder_rename_leaves_it_open_under_the_old_name() {
    let mut lab = Lab::new();
    put_draft(&lab, "kept").unwrap();
    let before = get(&lab);
    let url = image_url(&lab);
    set_faults(&mut lab, Some(("on_move:rooms", FaultAction::Locked)));
    let err = rename(&lab, "Vault").unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    set_faults(&mut lab, None);
    assert!(!sibling(&lab, "Vault").exists());
    assert_unchanged(&lab, &before);
    let after = get(&lab);
    assert_eq!(after["name"], "rooms");
    assert_eq!(after["draft"], before["draft"]);
    assert_eq!(lab.core.serve_scheme(&url).status, 200);
    put_draft(&lab, "still writable").unwrap();
}

#[test]
fn fr_prj_56_a_read_only_project_can_be_renamed() {
    let lab = Lab::new();
    lab.close();
    let manifest = lab.project_dir.join("project.json");
    let mut m: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    m["schema_version"] = json!(99);
    std::fs::write(&manifest, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    let bytes = std::fs::read(&manifest).unwrap();
    assert_eq!(lab.open()["read_only"], "newer_version");

    let new = rename(&lab, "Vault").unwrap();
    let after = get(&lab);
    assert_eq!(after["read_only"], "newer_version");
    assert_eq!(after["name"], "Vault");
    assert_eq!(
        std::fs::read(new.join("project.json")).unwrap(),
        bytes,
        "nothing inside is written"
    );
}

/// The brief's Windows check: with the real background threads (tile build, decode, thumbnail)
/// the folder's files are released before the rename, or waited out by its lock retry.
#[test]
fn fr_prj_56_rename_right_after_tiles_and_decode_started_on_real_threads() {
    let lab = Lab::with_context(|c| c.inline_background = false);
    let snap = get(&lab);
    lab.core
        .dispatch(
            "edt_tiles_open",
            json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"]}),
        )
        .unwrap();
    lab.core
        .dispatch("project_prefetch_original", json!({}))
        .unwrap();
    let new = rename(&lab, "Vault").unwrap();
    assert!(new.join("project.json").is_file() && !lab.project_dir.exists());
    let after = get(&lab);
    assert_eq!(after["project_id"], snap["project_id"]);
    assert_eq!(after["revisions"], snap["revisions"]);
    // The decode the UI asked for once is running again for the new folder.
    assert!(
        lab.core
            .state()
            .project
            .as_ref()
            .unwrap()
            .decoded
            .is_started()
    );
    // Let the background work finish before the temp folder goes.
    for _ in 0..200 {
        if lab
            .core
            .state()
            .project
            .as_ref()
            .unwrap()
            .decoded
            .is_ready()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    lab.close();
    assert!(!lab.project_dir.exists(), "nothing wrote to the old path");
}

#[test]
fn fr_prj_56_a_late_chat_save_never_recreates_the_old_folder() {
    use crate::prompt::chat::thread;
    let lab = Lab::new();
    let old = lab.project_dir.clone();
    let t = thread::Thread::new(lab.core.ctx.new_id(), lab.core.ctx.now(), "fake");
    thread::save(&lab.core.ctx, &old, &t).unwrap();
    rename(&lab, "Vault").unwrap();
    let err = thread::save(&lab.core.ctx, &old, &t).unwrap_err();
    assert!(crate::project::is_folder_moved(&err), "{err:?}");
    assert!(!Path::new(&old).exists());
}
