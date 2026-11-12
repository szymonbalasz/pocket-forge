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
    // The tile build that was in flight finishes: every level is served from the cache.
    let tiles = |lab: &Lab| {
        lab.core
            .dispatch(
                "edt_tiles_open",
                json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"]}),
            )
            .unwrap()
    };
    let done = |v: &Value| {
        v["levels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["complete"] == true)
    };
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
        if done(&tiles(&lab)) && decoded(&lab) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let t = tiles(&lab);
    assert!(done(&t), "{t}");
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

#[test]
fn fr_prj_57_a_failed_move_is_rolled_back_and_the_open_project_opens_again() {
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

    // Nothing is left for the startup to roll forward.
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

#[test]
fn fr_prj_57_a_kill_between_item_json_and_the_move_is_rolled_forward_at_startup() {
    let lab = Lab::new();
    lab.close();
    let files = tree(&lab.project_dir);
    let id = killed_delete(&lab, &lab.project_dir);

    let lab = lab.restart();
    assert!(!lab.project_dir.exists(), "not in its section");
    assert_eq!(tree(&item_dir(&lab, &id).join("rooms")), files);
    assert_eq!(trashed(&lab, "maps")[0]["id"].as_str(), Some(id.as_str()));
    assert!(cards(&lab, "maps").is_empty(), "never both");
    // A second startup changes nothing.
    let lab = lab.restart();
    assert_eq!(tree(&item_dir(&lab, &id).join("rooms")), files);
}

#[test]
fn fr_prj_57_a_killed_delete_whose_folder_is_gone_or_another_project_is_dropped() {
    let lab = Lab::new();
    lab.close();
    let gone = new_project(&lab, Kind::Map, "Gone", 10);
    let gone_id = killed_delete(&lab, &gone);
    std::fs::remove_dir_all(&gone).unwrap();
    // `rooms` was replaced by another project of the same name after the kill.
    let replaced_id = killed_delete(&lab, &lab.project_dir);
    std::fs::remove_dir_all(&lab.project_dir).unwrap();
    let other = new_project(&lab, Kind::Map, "rooms", 20);
    assert_eq!(other, lab.project_dir);
    let files = tree(&other);

    let lab = lab.restart();
    assert!(item_dirs(&lab).is_empty(), "{gone_id} {replaced_id}");
    assert!(trashed(&lab, "maps").is_empty(), "never neither nor both");
    assert_eq!(tree(&other), files, "the other project is not trashed");
    assert_eq!(cards(&lab, "maps"), ["rooms"]);
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

#[test]
fn fr_prj_58_a_restore_that_fails_is_listed_again() {
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
    assert_eq!(trash::purge(&lab.core.ctx, &settings), (0, 0), "day 29");

    lab.clock.advance(2 * DAY);
    // An item whose restore is in flight is skipped.
    mark_restoring(&lab, &restoring_id);
    let (count, bytes) = trash::purge(&lab.core.ctx, &settings);
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

/// "Keeps one it can't fully delete": a file in the trashed folder is held open.
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
    assert_eq!(trash::purge(&lab.core.ctx, &settings).0, 0);
    assert_eq!(trashed(&lab, "maps").len(), 1, "still listed");
    drop(held);
    assert_eq!(trash::purge(&lab.core.ctx, &settings).0, 1);
    assert!(!item_dir(&lab, &id).exists());
}
