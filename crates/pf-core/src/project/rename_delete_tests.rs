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

/// Two faults at once.
fn set_two_faults(lab: &mut Lab, a: &str, b: &str, action: FaultAction) {
    Arc::get_mut(&mut lab.core.ctx).unwrap().faults = Faults::parse(a, action)
        .unwrap()
        .and(Faults::parse(b, action).unwrap());
}

fn rename_at(lab: &Lab, dir: &Path, name: &str) -> Result<PathBuf, CoreError> {
    lab.core
        .dispatch(
            "project_rename",
            json!({"path": dir.to_string_lossy(), "name": name}),
        )
        .map(|v| PathBuf::from(v["path"].as_str().unwrap()))
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
    // R111: a rename has no force, so its message never offers to close.
    assert!(!err.message.contains("Close anyway"), "{}", err.message);
    assert!(
        err.message
            .contains("The name can't change until they are saved"),
        "{}",
        err.message
    );
    // A close still asks.
    let close = lab.core.dispatch("project_close", json!({})).unwrap_err();
    assert_eq!(close.code, ErrorCode::Unsaved);
    assert!(
        close.message.ends_with("Close anyway?"),
        "{}",
        close.message
    );
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
    // The tile build that was in flight when the folder was renamed finishes by itself: nothing
    // here asks for the tiles again, which would start another build.
    let decoded = |lab: &Lab| {
        lab.core
            .state()
            .project
            .as_ref()
            .unwrap()
            .decoded
            .is_ready()
    };
    for _ in 0..400 {
        let built = !lab.events_named("edt:tiles-ready").is_empty()
            || !lab.events_named("edt:tiles-failed").is_empty();
        if built && decoded(&lab) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(lab.events_named("edt:tiles-failed"), Vec::<Value>::new());
    assert_eq!(lab.events_named("edt:tiles-ready").len(), 1, "one build");
    assert!(decoded(&lab), "the decode started again and finished");
    // Every level is there, so asking now starts no build.
    let t = lab
        .core
        .dispatch(
            "edt_tiles_open",
            json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"]}),
        )
        .unwrap();
    assert!(
        t["levels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["complete"] == true),
        "{t}"
    );
    assert!(
        !lab.core.served().is_building(t["key"].as_str().unwrap()),
        "no second build"
    );
    let base = t["base_url"].as_str().unwrap();
    let ext = t["levels"][0]["ext"].as_str().unwrap();
    let tile = format!("{}/0/0_0.{ext}", &base[base.find("/t/").unwrap()..]);
    assert_eq!(lab.core.serve_scheme(&tile).status, 200);
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

#[test]
fn fr_prj_56_the_chat_context_and_export_settings_survive_the_rename() {
    let lab = Lab::new();
    let pid = get(&lab)["project_id"].clone();
    lab.core
        .dispatch(
            "prm_context_publish",
            json!({"project_id": pid, "host": "rgn", "input": lab.draft("moss")}),
        )
        .unwrap();
    let settings = json!({
        "grid": {"mode": "px", "source_px_per_square": 16.0},
        "target_px_per_square": 9,
        "format": "jpeg",
    });
    lab.core
        .dispatch("export_set_settings", json!({ "settings": settings }))
        .unwrap();
    let export = lab.core.dispatch("export_get_settings", json!({})).unwrap();

    rename(&lab, "Vault").unwrap();
    // The UI publishes a context only when it changes: the core still has the one it got.
    let context = lab.events_named("prm:chat-context");
    assert_eq!(context.last().unwrap()["host"], "rgn", "{context:?}");
    assert_eq!(context.last().unwrap()["project_id"], pid);
    assert_eq!(
        lab.core.dispatch("export_get_settings", json!({})).unwrap(),
        export
    );
    // A close still drops it.
    lab.close();
    lab.core
        .dispatch(
            "project_open",
            json!({"path": sibling(&lab, "Vault").to_string_lossy()}),
        )
        .unwrap();
    assert_eq!(
        lab.events_named("prm:chat-context").last().unwrap()["host"],
        Value::Null
    );
}

/// Walk L: a fetched (Ready) job reads as `resumable` at an open, and the UI drops that when
/// it loads the project. The reopen inside a rename must not bring it back.
#[test]
fn fr_prj_56_the_rename_adds_no_job_warning_a_normal_continue_wouldnt_show() {
    let lab = Lab::new();
    let job = lab.run("a mossy well");
    assert_eq!(lab.job(job).state, crate::job::JobState::Fetched);
    let before = get(&lab);
    assert_eq!(before["warnings"], json!([]));
    rename(&lab, "Vault").unwrap();
    assert_eq!(get(&lab)["warnings"], json!([]), "after the rename");
    // A failed rename opens the old folder again: the same.
    let mut lab = lab;
    set_faults(&mut lab, Some(("on_move:Vault", FaultAction::Locked)));
    rename_at(&lab, &sibling(&lab, "Vault"), "Crypt").unwrap_err();
    set_faults(&mut lab, None);
    assert_eq!(get(&lab)["warnings"], json!([]), "after a failed rename");
    // A warning the project showed before the rename is kept.
    lab.close();
    let reopened = lab
        .core
        .dispatch(
            "project_open",
            json!({"path": sibling(&lab, "Vault").to_string_lossy()}),
        )
        .unwrap();
    assert_eq!(reopened["warnings"][0]["code"], "resumable");
    rename_at(&lab, &sibling(&lab, "Vault"), "Crypt").unwrap();
    assert_eq!(get(&lab)["warnings"], reopened["warnings"]);
}

/// R111, walk M: a job outlives a close. Renaming the closed project would leave the runner
/// writing to a folder that is gone (the update is spooled and quarantined on reopen).
#[test]
fn fr_prj_45_a_closed_rename_with_a_tracked_job_is_job_active() {
    let lab = Lab::new();
    let job = lab.submit("a mossy well");
    lab.close();
    let files = tree(&lab.project_dir);
    let err = rename(&lab, "Vault").unwrap_err();
    assert_eq!(err.code, ErrorCode::JobActive);
    assert_eq!(err.detail["job_id"], json!(job));
    assert!(err.message.contains("Rename"), "{}", err.message);
    assert_eq!(tree(&lab.project_dir), files, "nothing changed");
    assert!(!sibling(&lab, "Vault").exists());
    // The name checks come first, and the same name is still nothing to do.
    assert_eq!(
        rename(&lab, "a:b").unwrap_err().code,
        ErrorCode::InvalidName
    );
    assert_eq!(rename(&lab, "rooms").unwrap(), lab.project_dir);

    // The runner keeps driving it while the project is closed, and the refusal holds.
    lab.drive();
    assert_eq!(
        rename(&lab, "Vault").unwrap_err().code,
        ErrorCode::JobActive
    );
    // Once the job is fetched its results are in the folder, where they belong, and the
    // rename goes through.
    lab.open();
    lab.drive();
    assert_eq!(lab.job(job).state, crate::job::JobState::Fetched);
    lab.close();
    assert!(lab.events_named("project:spool-dropped").is_empty());
    let new = rename(&lab, "Vault").unwrap();
    assert!(new.join("project.json").is_file() && !lab.project_dir.exists());
    let opened = lab
        .core
        .dispatch("project_open", json!({"path": new.to_string_lossy()}))
        .unwrap();
    assert!(
        opened["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["job_id"] == json!(job)),
        "the candidates came with it: {opened}"
    );
}

#[test]
fn fr_prj_45_a_closed_rename_with_an_active_job_file_is_job_active_after_a_restart() {
    let lab = Lab::new();
    let job = lab.submit("a mossy well");
    lab.close();
    // Nothing drives the job after a restart: its job file still says it is running.
    let lab = lab.restart();
    assert!(lab.core.state().rgn.runner.tracked().is_empty());
    let err = rename(&lab, "Vault").unwrap_err();
    assert_eq!(err.code, ErrorCode::JobActive);
    assert_eq!(err.detail["job_id"], json!(job));
    assert!(lab.project_dir.is_dir() && !sibling(&lab, "Vault").exists());
    // Once the job has finished the rename goes through.
    lab.open();
    lab.drive();
    lab.close();
    assert!(rename(&lab, "Vault").unwrap().is_dir());
}

// ---------------------------------------------------------------- FR-PRJ-57, 58

use crate::commands::{LibraryTrashList, LibraryTrashRestore, ProjectDelete};
use crate::core::{Command, dispatch_json};
use crate::library::trash;
use crate::manifest::Kind;
use crate::test_support::tree;

const DAY: std::time::Duration = std::time::Duration::from_secs(86_400);

fn lib(lab: &Lab) -> PathBuf {
    lab.dir.path().join("lib")
}

fn run<C: Command>(lab: &Lab, input: Value) -> Result<Value, CoreError> {
    dispatch_json::<C>(&lab.core, input)
}

fn delete(lab: &Lab, dir: &Path) -> Result<String, CoreError> {
    run::<ProjectDelete>(lab, json!({"path": dir.to_string_lossy()}))
        .map(|v| v["item_id"].as_str().unwrap().to_owned())
}

fn trashed(lab: &Lab, section: &str) -> Vec<Value> {
    run::<LibraryTrashList>(lab, json!({"section": section}))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn restore(lab: &Lab, id: &str) -> Result<PathBuf, CoreError> {
    run::<LibraryTrashRestore>(lab, json!({"id": id}))
        .map(|v| PathBuf::from(v["path"].as_str().unwrap()))
}

fn cards(lab: &Lab, section: &str) -> Vec<String> {
    lab.core
        .dispatch("library_list_projects", json!({"section": section}))
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Another project, from a generated image; returns its folder.
fn new_project(lab: &Lab, kind: Kind, name: &str, shade: u8) -> PathBuf {
    let src = lab.dir.path().join("outside").join(format!("{name}.png"));
    image::RgbImage::from_pixel(40, 30, image::Rgb([shade, 90, 40]))
        .save(&src)
        .unwrap();
    let settings = lab.core.state().settings.clone();
    let items = crate::create::create_projects(
        &lab.core.ctx,
        &settings,
        kind,
        &[src],
        &std::sync::atomic::AtomicBool::new(false),
        None,
    )
    .unwrap();
    match &items[0] {
        crate::create::CreateItem::Created { path, .. } => path.clone(),
        other => panic!("{other:?}"),
    }
}

fn item_dir(lab: &Lab, id: &str) -> PathBuf {
    trash::trash_dir(&lib(lab)).join(id)
}

fn item_json(lab: &Lab, id: &str) -> Value {
    serde_json::from_slice(&std::fs::read(item_dir(lab, id).join("item.json")).unwrap()).unwrap()
}

/// The item folders in the library trash.
fn item_dirs(lab: &Lab) -> Vec<String> {
    std::fs::read_dir(trash::trash_dir(&lib(lab)))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn fr_prj_57_delete_moves_a_closed_project_whole_to_the_library_trash() {
    let lab = Lab::new();
    lab.run("a mossy well");
    let project_id = get(&lab)["project_id"].clone();
    lab.close();
    let before = tree(&lab.project_dir);
    assert!(before.iter().any(|f| f.starts_with("working/")));

    let id = delete(&lab, &lab.project_dir).unwrap();
    assert!(!lab.project_dir.exists(), "gone from its section");
    assert_eq!(
        tree(&item_dir(&lab, &id).join("rooms")),
        before,
        "the folder is in the trash, whole"
    );
    let item = item_json(&lab, &id);
    assert_eq!(item["schema_version"], 1);
    assert_eq!(item["id"].as_str(), Some(id.as_str()));
    assert_eq!(item["kind"], "project");
    assert_eq!(item["project_id"], project_id);
    assert_eq!(item["section"], "maps");
    assert_eq!(item["name"], "rooms");
    assert!(item["trashed_at"].is_string());
    assert!(item.get("restored_at").is_none());
    assert!(cards(&lab, "maps").is_empty());
}

#[test]
fn fr_prj_57_delete_of_the_open_project_closes_it_and_drops_a_waiting_save() {
    let mut lab = Lab::new();
    let url = image_url(&lab);
    set_faults(
        &mut lab,
        Some(("after_temp:project.json", FaultAction::Locked)),
    );
    assert_eq!(
        put_draft(&lab, "waits").unwrap_err().code,
        ErrorCode::Locked
    );
    // The UI asked: the waiting save is tried once more, then dropped.
    let id = delete(&lab, &lab.project_dir).unwrap();
    set_faults(&mut lab, None);
    assert_eq!(get(&lab), Value::Null, "nothing is open");
    assert_eq!(lab.core.serve_scheme(&url).status, 404);
    assert!(!lab.project_dir.exists());
    assert!(item_dir(&lab, &id).join("rooms/project.json").is_file());
    assert_eq!(trashed(&lab, "maps").len(), 1);
}

#[test]
fn fr_prj_57_an_active_job_refuses_the_delete_open_and_closed() {
    let lab = Lab::new();
    let job = lab.submit("a mossy well");
    let before = get(&lab);
    let err = delete(&lab, &lab.project_dir).unwrap_err();
    assert_eq!(err.code, ErrorCode::JobActive);
    assert_eq!(err.detail["job_id"], json!(job));
    assert_unchanged(&lab, &before);
    // Closed, with the runner still driving the job.
    lab.close();
    let err = delete(&lab, &lab.project_dir).unwrap_err();
    assert_eq!(err.code, ErrorCode::JobActive);
    // After a restart nothing drives it: its job file still says so.
    let lab = lab.restart();
    let err = delete(&lab, &lab.project_dir).unwrap_err();
    assert_eq!(err.code, ErrorCode::JobActive);
    assert_eq!(err.detail["job_id"], json!(job));
    assert!(lab.project_dir.is_dir() && item_dirs(&lab).is_empty());
    // Once the job has finished the delete goes through.
    lab.open();
    lab.drive();
    delete(&lab, &lab.project_dir).unwrap();
}

/// R111: a waiting draft is dropped only once the move has succeeded. Here the draft is really
/// waiting (the manifest stays locked) when the move fails.
#[test]
fn fr_prj_57_a_failed_delete_of_the_open_project_keeps_its_waiting_draft() {
    let mut lab = Lab::new();
    let before = get(&lab);
    let url = image_url(&lab);
    set_two_faults(
        &mut lab,
        "after_temp:project.json",
        "on_move:rooms",
        FaultAction::Locked,
    );
    assert_eq!(
        put_draft(&lab, "waits").unwrap_err().code,
        ErrorCode::Locked
    );
    let waiting = |lab: &Lab| {
        let state = lab.core.state();
        let p = state.project.as_ref().expect("the project is open");
        p.pending_draft
            .as_ref()
            .map(|d| d.draft.final_prompt.clone())
    };
    assert_eq!(waiting(&lab).as_deref(), Some("waits"));

    let err = delete(&lab, &lab.project_dir).unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    assert_unchanged(&lab, &before);
    assert_eq!(
        waiting(&lab).as_deref(),
        Some("waits"),
        "still waiting in memory"
    );
    assert!(item_dirs(&lab).is_empty(), "the item is dropped");
    assert_ne!(lab.manifest()["draft"]["final_prompt"], "waits");
    assert_eq!(lab.core.serve_scheme(&url).status, 200, "still served");

    // The lock lifts: the draft that waited is saved, not lost.
    set_faults(&mut lab, None);
    crate::project::commands::retry_pending_saves(&lab.core);
    assert_eq!(lab.manifest()["draft"]["final_prompt"], "waits");
    assert!(!crate::project::has_unsaved(
        lab.core.state().project.as_ref().unwrap()
    ));
}

#[test]
fn fr_prj_57_a_failed_move_is_rolled_back_and_the_open_project_stays_open() {
    let mut lab = Lab::new();
    put_draft(&lab, "kept").unwrap();
    let before = get(&lab);
    let files = tree(&lab.project_dir);
    set_faults(&mut lab, Some(("on_move:rooms", FaultAction::Locked)));

    let err = delete(&lab, &lab.project_dir).unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    assert_unchanged(&lab, &before);
    assert_eq!(get(&lab)["draft"], before["draft"]);
    assert!(item_dirs(&lab).is_empty(), "the item is dropped");

    lab.close();
    let err = delete(&lab, &lab.project_dir).unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    assert!(item_dirs(&lab).is_empty());
    set_faults(&mut lab, None);

    // Nothing is left behind for a startup.
    let lab = lab.restart();
    assert_eq!(tree(&lab.project_dir), files);
    assert!(trashed(&lab, "maps").is_empty());
    assert_eq!(cards(&lab, "maps"), ["rooms"]);
}

/// The state a kill between the `item.json` write and the folder move leaves
/// (`PF_FAULT=after_rename:item.json`): the item on disk, the folder still in its section.
fn killed_delete(lab: &Lab, dir: &Path) -> String {
    let settings = lab.core.state().settings.clone();
    let loc = trash::locate(&lab.core.ctx, &settings, dir).unwrap();
    let (item_dir, item) = trash::begin_delete(&lab.core.ctx, &loc).unwrap();
    assert!(item_dir.join("item.json").is_file() && dir.is_dir());
    item.id.to_string()
}

/// R111: startup never moves a project to match an item. The killed delete leaves the project
/// in its section; its item has no folder, so it is kept, never listed, and ages out.
#[test]
fn fr_prj_57_a_kill_between_item_json_and_the_move_leaves_the_project_in_its_section() {
    let lab = Lab::new();
    lab.close();
    let files = tree(&lab.project_dir);
    let id = killed_delete(&lab, &lab.project_dir);

    let lab = lab.restart();
    assert_eq!(tree(&lab.project_dir), files, "still in its section, whole");
    assert_eq!(cards(&lab, "maps"), ["rooms"]);
    assert!(item_dir(&lab, &id).join("item.json").is_file(), "kept");
    assert!(!item_dir(&lab, &id).join("rooms").exists(), "never both");
    assert!(trashed(&lab, "maps").is_empty(), "never listed");
    // It opens and can be deleted for real.
    lab.open();
    lab.close();
    let real = delete(&lab, &lab.project_dir).unwrap();
    assert_eq!(trashed(&lab, "maps").len(), 1);
    assert_eq!(trashed(&lab, "maps")[0]["id"].as_str(), Some(real.as_str()));
}

#[test]
fn fr_prj_58_an_item_without_its_folder_is_kept_unlisted_and_its_restore_is_not_found() {
    let lab = Lab::new();
    lab.close();
    // A killed delete: the project is still in its section.
    let killed_id = killed_delete(&lab, &lab.project_dir);
    // An item whose folder hasn't arrived (a sync still under way).
    let gone = new_project(&lab, Kind::Map, "Gone", 10);
    let gone_id = delete(&lab, &gone).unwrap();
    std::fs::remove_dir_all(item_dir(&lab, &gone_id).join("Gone")).unwrap();
    let files = tree(&lab.project_dir);

    let check = |lab: &Lab| {
        assert!(trashed(lab, "maps").is_empty(), "never listed");
        for id in [&killed_id, &gone_id] {
            let err = restore(lab, id).unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound, "{id}");
            let item = item_json(lab, id);
            assert!(item.get("restored_at").is_none(), "{item}");
            assert!(item.get("purging_at").is_none(), "{item}");
        }
        assert_eq!(tree(&lab.project_dir), files, "the project is untouched");
        assert_eq!(cards(lab, "maps"), ["rooms"]);
    };
    check(&lab);
    let lab = lab.restart();
    check(&lab);
    let mut kept = item_dirs(&lab);
    kept.sort();
    let mut want = vec![killed_id.clone(), gone_id.clone()];
    want.sort();
    assert_eq!(kept, want, "startup drops no item");

    // Only the purge removes them, 30 days after `trashed_at`; the project stays.
    lab.clock.advance(31 * DAY);
    let lab = lab.restart();
    assert!(item_dirs(&lab).is_empty());
    assert_eq!(tree(&lab.project_dir), files);
    assert_eq!(cards(&lab, "maps"), ["rooms"]);
}

/// R111: a folder taken out of the trash by hand (or restored on another machine, the item
/// not synced yet) is a project in its section again. Startup doesn't trash it a second time.
#[test]
fn fr_prj_58_a_project_taken_back_by_hand_is_not_trashed_again_at_startup() {
    let lab = Lab::new();
    lab.close();
    let files = tree(&lab.project_dir);
    let id = delete(&lab, &lab.project_dir).unwrap();
    rename_dir(&item_dir(&lab, &id).join("rooms"), &lab.project_dir);

    let lab = lab.restart();
    assert_eq!(tree(&lab.project_dir), files, "still in its section");
    assert_eq!(cards(&lab, "maps"), ["rooms"]);
    assert!(trashed(&lab, "maps").is_empty());
    assert_eq!(
        item_dirs(&lab),
        std::slice::from_ref(&id),
        "the item is kept"
    );
    assert_eq!(restore(&lab, &id).unwrap_err().code, ErrorCode::NotFound);
    let lab = lab.restart();
    assert_eq!(tree(&lab.project_dir), files, "nor at the next startup");
    assert_eq!(lab.open()["name"], "rooms");
}

#[test]
fn fr_prj_57_delete_is_allowed_on_an_unreadable_project_and_refuses_other_folders() {
    let lab = Lab::new();
    lab.close();
    std::fs::write(lab.project_dir.join("project.json"), b"{ not json").unwrap();
    let id = delete(&lab, &lab.project_dir).unwrap();
    assert_eq!(item_json(&lab, &id)["project_id"], Value::Null);
    assert_eq!(trashed(&lab, "maps")[0]["name"], "rooms");

    let code = |p: PathBuf| delete(&lab, &p).unwrap_err().code;
    let plain = lib(&lab).join("Maps/Notes");
    std::fs::create_dir(&plain).unwrap();
    assert_eq!(code(plain), ErrorCode::NotAProject);
    assert_eq!(code(lib(&lab).join("Maps")), ErrorCode::NotAProject);
    assert_eq!(
        code(item_dir(&lab, &id).join("rooms")),
        ErrorCode::NotAProject,
        "a trashed project is not deleted again"
    );
    assert_eq!(code(lib(&lab).join("Maps/Nowhere")), ErrorCode::NotFound);
    let outside = lab.dir.path().join("outside");
    assert_eq!(code(outside), ErrorCode::OutsideLibrary);
    assert_eq!(
        run::<ProjectDelete>(&lab, json!({"path": " "}))
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    assert_eq!(item_dirs(&lab), [id]);
}

#[test]
fn fr_prj_58_list_is_per_section_newest_first_with_purge_dates() {
    let lab = Lab::new();
    lab.close();
    let token = new_project(&lab, Kind::Token, "Goblin", 30);
    let second = new_project(&lab, Kind::Map, "Crypt", 40);
    let first_id = delete(&lab, &lab.project_dir).unwrap();
    lab.clock.advance(DAY);
    let second_id = delete(&lab, &second).unwrap();
    let token_id = delete(&lab, &token).unwrap();

    let maps = trashed(&lab, "maps");
    let ids: Vec<_> = maps.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [second_id.as_str(), first_id.as_str()]);
    assert_eq!(maps[0]["name"], "Crypt");
    assert_eq!(maps[1]["section"], "maps");
    for i in &maps {
        let at: crate::time::Timestamp = i["trashed_at"].as_str().unwrap().parse().unwrap();
        let purge: crate::time::Timestamp = i["purge_at"].as_str().unwrap().parse().unwrap();
        assert_eq!(purge, at.plus(30 * DAY));
    }
    let tokens = trashed(&lab, "tokens");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["id"].as_str(), Some(token_id.as_str()));
    assert!(trashed(&lab, "items").is_empty());
}

#[test]
fn fr_prj_58_the_library_trash_is_never_listed_as_a_project() {
    let lab = Lab::new();
    lab.close();
    delete(&lab, &lab.project_dir).unwrap();
    for section in ["maps", "tokens", "items"] {
        assert!(cards(&lab, section).is_empty(), "{section}");
    }
    let packs = lab.core.dispatch("ref_list_packs", json!({})).unwrap();
    assert!(!packs.to_string().contains("rooms"), "{packs}");
}

#[test]
fn fr_prj_58_restore_puts_it_back_with_a_suffix_when_the_name_is_taken() {
    let lab = Lab::new();
    lab.run("a mossy well");
    let before = get(&lab);
    lab.close();
    let files = tree(&lab.project_dir);
    let id = delete(&lab, &lab.project_dir).unwrap();

    let back = restore(&lab, &id).unwrap();
    assert_eq!(back, lab.project_dir, "back in its section, same root");
    assert_eq!(tree(&back), files);
    assert!(item_dirs(&lab).is_empty() && trashed(&lab, "maps").is_empty());
    let again = lab.open();
    for same in ["project_id", "revisions", "head_revision", "layers"] {
        assert_eq!(again[same], before[same], "{same}");
    }
    lab.close();
    assert_eq!(restore(&lab, &id).unwrap_err().code, ErrorCode::UnknownId);

    // Deleted again; the name is taken by the time it is restored.
    let files = tree(&lab.project_dir);
    let id = delete(&lab, &lab.project_dir).unwrap();
    let other = new_project(&lab, Kind::Map, "rooms", 50);
    let back = restore(&lab, &id).unwrap();
    assert_eq!(back, sibling(&lab, "rooms (2)"));
    assert_eq!(tree(&back), files);
    assert!(other.join("project.json").is_file());
    let mut names = cards(&lab, "maps");
    names.sort();
    assert_eq!(names, ["rooms", "rooms (2)"]);
}

/// The move is refused before it starts (the fault seam): `restored_at`, written first, is
/// cleared again.
#[test]
fn fr_prj_58_a_restore_whose_move_is_refused_is_listed_again() {
    let mut lab = Lab::new();
    lab.close();
    let id = delete(&lab, &lab.project_dir).unwrap();
    set_faults(&mut lab, Some(("on_move:rooms", FaultAction::Locked)));
    assert_eq!(restore(&lab, &id).unwrap_err().code, ErrorCode::Locked);
    set_faults(&mut lab, None);
    assert!(!lab.project_dir.exists());
    assert_eq!(trashed(&lab, "maps")[0]["id"].as_str(), Some(id.as_str()));
    assert!(item_json(&lab, &id).get("restored_at").is_none());
    assert_eq!(restore(&lab, &id).unwrap(), lab.project_dir);
}

/// Makes the real folder move of a restore fail, with `item.json` still writable: on Windows a
/// file in the folder is held open without delete sharing (like a scanner); elsewhere the
/// section folder, where it would go, is made read-only. Returns what undoes it, or `None`
/// where the OS wouldn't refuse (a root user).
#[cfg(windows)]
fn block_move(item: &Path, name: &str, _section: &Path) -> Option<Box<dyn FnOnce()>> {
    use std::os::windows::fs::OpenOptionsExt;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(item.join(name).join("project.json"))
        .unwrap();
    Some(Box::new(move || drop(held)))
}

#[cfg(unix)]
fn block_move(_item: &Path, _name: &str, section: &Path) -> Option<Box<dyn FnOnce()>> {
    use std::os::unix::fs::PermissionsExt;
    let section = section.to_path_buf();
    let set = |dir: &Path, mode: u32| {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    set(&section, 0o555);
    if std::fs::write(section.join("probe"), b"").is_ok() {
        // Permissions don't bind this user (root).
        let _ = std::fs::remove_file(section.join("probe"));
        set(&section, 0o755);
        return None;
    }
    Some(Box::new(move || set(&section, 0o755)))
}

/// A restore that fails part-way: `restored_at` is on disk and the real folder move fails.
#[test]
fn fr_prj_58_a_restore_that_fails_is_listed_again() {
    let lab = Lab::new();
    lab.close();
    let files = tree(&lab.project_dir);
    let id = delete(&lab, &lab.project_dir).unwrap();
    let section = lab.project_dir.parent().unwrap().to_path_buf();
    let Some(unblock) = block_move(&item_dir(&lab, &id), "rooms", &section) else {
        return;
    };
    let err = restore(&lab, &id).unwrap_err();
    assert!(
        matches!(err.code, ErrorCode::Locked | ErrorCode::Io),
        "{err:?}"
    );
    assert!(!lab.project_dir.exists(), "not in its section");
    assert_eq!(tree(&item_dir(&lab, &id).join("rooms")), files, "whole");
    assert!(item_json(&lab, &id).get("restored_at").is_none());
    unblock();
    assert_eq!(trashed(&lab, "maps")[0]["id"].as_str(), Some(id.as_str()));
    assert_eq!(restore(&lab, &id).unwrap(), lab.project_dir);
    assert_eq!(tree(&lab.project_dir), files);
}

/// Marks an item as being restored (`restored_at` set), as a restore does first.
fn mark_restoring(lab: &Lab, id: &str) {
    let mut item = item_json(lab, id);
    item["restored_at"] = item["trashed_at"].clone();
    std::fs::write(
        item_dir(lab, id).join("item.json"),
        serde_json::to_vec_pretty(&item).unwrap(),
    )
    .unwrap();
}

#[test]
fn fr_prj_58_an_interrupted_restore_is_finished_at_startup() {
    let lab = Lab::new();
    lab.close();
    let files = tree(&lab.project_dir);
    let id = delete(&lab, &lab.project_dir).unwrap();
    mark_restoring(&lab, &id);
    assert!(trashed(&lab, "maps").is_empty(), "not listed meanwhile");
    let lab = lab.restart();
    assert_eq!(tree(&lab.project_dir), files);
    assert!(item_dirs(&lab).is_empty());
}

#[test]
fn fr_prj_58_purge_at_31_days_with_the_fake_clock() {
    let lab = Lab::new();
    lab.close();
    let kept = new_project(&lab, Kind::Map, "Crypt", 60);
    let restoring = new_project(&lab, Kind::Token, "Goblin", 70);
    let old_id = delete(&lab, &lab.project_dir).unwrap();
    let restoring_id = delete(&lab, &restoring).unwrap();
    lab.clock.advance(29 * DAY);
    let kept_id = delete(&lab, &kept).unwrap();
    let settings = lab.core.state().settings.clone();
    assert_eq!(
        trash::purge(&lab.core.ctx, &settings, &[]),
        (0, 0),
        "day 29"
    );

    lab.clock.advance(2 * DAY);
    // An item whose restore is in flight is skipped.
    mark_restoring(&lab, &restoring_id);
    let (count, bytes) = trash::purge(&lab.core.ctx, &settings, &[]);
    assert_eq!(count, 1);
    assert!(bytes > 0);
    assert_eq!(item_dirs(&lab).len(), 2);
    assert!(!item_dir(&lab, &old_id).exists(), "the old item is gone");
    assert!(item_dir(&lab, &restoring_id).join("Goblin").is_dir());
    let left = trashed(&lab, "maps");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["id"].as_str(), Some(kept_id.as_str()));
}

#[test]
fn fr_prj_58_startup_purges_and_emits_trash_purged() {
    let lab = Lab::new();
    lab.close();
    let id = delete(&lab, &lab.project_dir).unwrap();
    lab.clock.advance(31 * DAY);
    let lab = lab.restart();
    assert!(!item_dir(&lab, &id).exists());
    assert!(trashed(&lab, "maps").is_empty());
    let purged = lab.events_named("trash:purged");
    assert_eq!(purged.len(), 1);
    assert_eq!(purged[0]["count"], 1);
    assert!(purged[0]["bytes"].as_u64().unwrap() > 0);
}

/// R111: an interrupted restore that can't be finished at startup has its `restored_at`
/// cleared. Although it is older than 30 days, that startup's purge leaves it: the user was
/// getting it back.
#[test]
fn fr_prj_58_an_interrupted_restore_older_than_30_days_survives_that_startup() {
    let mut lab = Lab::new();
    lab.close();
    let files = tree(&lab.project_dir);
    let id = delete(&lab, &lab.project_dir).unwrap();
    mark_restoring(&lab, &id);
    lab.clock.advance(31 * DAY);
    // The startup can't finish the restore: the move is refused.
    set_faults(&mut lab, Some(("on_move:rooms", FaultAction::Locked)));
    let settings = lab.core.state().settings.clone();
    trash::sweep_startup(&lab.core.ctx, &settings);
    set_faults(&mut lab, None);

    assert!(lab.events_named("trash:purged").is_empty(), "not purged");
    assert_eq!(tree(&item_dir(&lab, &id).join("rooms")), files, "whole");
    let item = item_json(&lab, &id);
    assert!(item.get("restored_at").is_none(), "{item}");
    assert!(item.get("purging_at").is_none(), "{item}");
    assert_eq!(trashed(&lab, "maps")[0]["id"].as_str(), Some(id.as_str()));
    // It can be restored in this session.
    assert_eq!(restore(&lab, &id).unwrap(), lab.project_dir);
    assert_eq!(tree(&lab.project_dir), files);
}

/// R111: the same, when it can't be finished because the folder isn't in the item (already
/// moved back, or not synced yet): the item is kept, not dropped and not purged.
#[test]
fn fr_prj_58_an_interrupted_restore_without_its_folder_is_kept_at_startup() {
    let lab = Lab::new();
    lab.close();
    let id = delete(&lab, &lab.project_dir).unwrap();
    rename_dir(&item_dir(&lab, &id).join("rooms"), &lab.project_dir);
    mark_restoring(&lab, &id);
    lab.clock.advance(31 * DAY);
    let lab = lab.restart();
    assert_eq!(
        item_dirs(&lab),
        std::slice::from_ref(&id),
        "kept in that startup"
    );
    assert!(item_json(&lab, &id).get("restored_at").is_none());
    assert!(trashed(&lab, "maps").is_empty());
    assert_eq!(cards(&lab, "maps"), ["rooms"]);
    // The next startup purges the leftover item; the project stays.
    let lab = lab.restart();
    assert!(item_dirs(&lab).is_empty());
    assert_eq!(cards(&lab, "maps"), ["rooms"]);
}

/// Marks an item as a purge does first (`purging_at` set).
fn mark_purging(lab: &Lab, id: &str) {
    let mut item = item_json(lab, id);
    item["purging_at"] = item["trashed_at"].clone();
    std::fs::write(
        item_dir(lab, id).join("item.json"),
        serde_json::to_vec_pretty(&item).unwrap(),
    )
    .unwrap();
}

/// R111: a purge that stopped half-way (a kill, a locked file) left the item marked and the
/// project folder partly deleted. It is never offered for Restore, and the next startup
/// finishes it, whatever its age.
#[test]
fn fr_prj_58_a_purge_that_cant_finish_is_unlisted_retried_and_never_restorable() {
    let lab = Lab::new();
    lab.close();
    let id = delete(&lab, &lab.project_dir).unwrap();
    // Half a purge: marked, and part of the project already gone.
    mark_purging(&lab, &id);
    std::fs::remove_file(item_dir(&lab, &id).join("rooms/project.json")).unwrap();

    assert!(trashed(&lab, "maps").is_empty(), "never listed");
    assert_eq!(restore(&lab, &id).unwrap_err().code, ErrorCode::UnknownId);
    assert!(item_dir(&lab, &id).join("rooms").is_dir(), "nothing moved");
    assert!(
        !lab.project_dir.exists(),
        "half a project is never put back"
    );

    let lab = lab.restart();
    assert!(!item_dir(&lab, &id).exists(), "the purge is finished");
    assert_eq!(lab.events_named("trash:purged")[0]["count"], 1);
    assert!(cards(&lab, "maps").is_empty());
}

#[test]
fn fr_prj_58_the_purge_marks_the_item_before_it_deletes() {
    let mut lab = Lab::new();
    lab.close();
    let id = delete(&lab, &lab.project_dir).unwrap();
    lab.clock.advance(31 * DAY);
    let files = tree(&item_dir(&lab, &id).join("rooms"));
    // The mark can't be written: nothing is deleted.
    set_faults(
        &mut lab,
        Some(("after_temp:item.json", FaultAction::Locked)),
    );
    let settings = lab.core.state().settings.clone();
    assert_eq!(trash::purge(&lab.core.ctx, &settings, &[]).0, 0);
    set_faults(&mut lab, None);
    assert_eq!(tree(&item_dir(&lab, &id).join("rooms")), files);
    assert!(item_json(&lab, &id).get("purging_at").is_none());
    assert_eq!(trashed(&lab, "maps").len(), 1, "still restorable");
    // An item this startup spared is left too.
    let spared = [id.parse().unwrap()];
    assert_eq!(trash::purge(&lab.core.ctx, &settings, &spared).0, 0);
    assert_eq!(trashed(&lab, "maps").len(), 1);
    assert_eq!(trash::purge(&lab.core.ctx, &settings, &[]).0, 1);
    assert!(!item_dir(&lab, &id).exists());
}

/// "A purge that couldn't finish": a file in the trashed folder is held open.
#[cfg(windows)]
#[test]
fn fr_prj_58_purge_keeps_an_item_it_cant_fully_delete() {
    use std::os::windows::fs::OpenOptionsExt;
    let lab = Lab::new();
    lab.close();
    let id = delete(&lab, &lab.project_dir).unwrap();
    lab.clock.advance(31 * DAY);
    // FILE_SHARE_READ | FILE_SHARE_WRITE, no delete: like a scanner reading the file.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(item_dir(&lab, &id).join("rooms/project.json"))
        .unwrap();
    let settings = lab.core.state().settings.clone();
    assert_eq!(trash::purge(&lab.core.ctx, &settings, &[]).0, 0);
    assert!(item_json(&lab, &id)["purging_at"].is_string(), "marked");
    assert!(trashed(&lab, "maps").is_empty(), "never listed again");
    assert_eq!(restore(&lab, &id).unwrap_err().code, ErrorCode::UnknownId);
    drop(held);
    assert_eq!(trash::purge(&lab.core.ctx, &settings, &[]).0, 1);
    assert!(!item_dir(&lab, &id).exists());
}
