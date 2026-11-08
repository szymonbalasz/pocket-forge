//! `<data-dir>/settings.json` (manifest-schema §5, FR-PRJ-39): per machine, never synced.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::context::Context;
use crate::error::{CoreError, CoreResult};
use crate::fsx;

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// `library` section (PRJ).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// Absolute paths, per machine; the first receives new projects (FR-PRJ-01).
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// Unknown fields, including the retired `import_dest` (#44): kept, ignored.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// `spend` section (meaning owned by RGN). Provider units; `None` = not set yet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpendSettings {
    #[serde(default)]
    pub cap: Option<f64>,
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// `provider` section (meaning owned by PRV). Never credentials (FR-PRV-06).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ProviderSettings {
    /// The account part PRV needs (FR-PRV-29).
    pub fn account(&self) -> pf_provider::ProviderAccount {
        pf_provider::ProviderAccount {
            team_id: self.team_id.clone(),
            project_id: self.project_id.clone(),
        }
    }
}

/// The whole settings file. Unknown sections and fields are kept (written after known ones).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub schema_version: u32,
    pub install_id: Ulid,
    #[serde(default)]
    pub library: LibrarySettings,
    #[serde(default)]
    pub spend: SpendSettings,
    #[serde(default)]
    pub provider: ProviderSettings,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Settings {
    pub fn defaults(ctx: &Context) -> Self {
        Settings {
            schema_version: SETTINGS_SCHEMA_VERSION,
            install_id: ctx.new_id(),
            library: LibrarySettings::default(),
            spend: SpendSettings::default(),
            provider: ProviderSettings::default(),
            extra: BTreeMap::new(),
        }
    }
}

/// A warning from loading settings, shown once in the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsWarning {
    pub code: &'static str,
    pub message: String,
}

pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

/// Loads settings, creating them on first run (that is when `install_id` is made).
///
/// A corrupt or unreadable file → defaults plus a warning; the bad file is kept as
/// `settings.corrupt-<UTC>.json`. A newer `schema_version` is treated the same way, so a
/// newer app's file is never overwritten in place.
pub fn load_or_init(ctx: &Context) -> CoreResult<(Settings, Vec<SettingsWarning>)> {
    std::fs::create_dir_all(&ctx.data_dir)
        .map_err(|e| CoreError::io(format!("creating {}", ctx.data_dir.display()), &e))?;
    let path = settings_path(&ctx.data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let s = Settings::defaults(ctx);
            save(ctx, &s)?;
            return Ok((s, vec![]));
        }
        Err(e) => return Err(CoreError::io("reading settings.json", &e)),
    };
    let parsed = serde_json::from_slice::<Settings>(&bytes)
        .map_err(|e| e.to_string())
        .and_then(|s| {
            if s.schema_version > SETTINGS_SCHEMA_VERSION {
                Err(format!(
                    "made by a newer Pocket Forge (schema {})",
                    s.schema_version
                ))
            } else {
                Ok(s)
            }
        });
    match parsed {
        Ok(s) => Ok((s, vec![])),
        Err(reason) => {
            let aside = ctx
                .data_dir
                .join(format!("settings.corrupt-{}.json", ctx.now().file_stamp()));
            std::fs::rename(&path, &aside)
                .map_err(|e| CoreError::io("keeping the corrupt settings.json aside", &e))?;
            let s = Settings::defaults(ctx);
            save(ctx, &s)?;
            tracing::warn!(%reason, aside = %aside.display(), "settings.json unreadable; using defaults");
            Ok((
                s,
                vec![SettingsWarning {
                    code: "settings_corrupt",
                    message: format!(
                        "Settings were unreadable ({reason}) and were reset. The old file is kept as {}.",
                        aside.file_name().unwrap_or_default().to_string_lossy()
                    ),
                }],
            ))
        }
    }
}

/// Writes settings atomically (temp → rename).
pub fn save(ctx: &Context, settings: &Settings) -> CoreResult<()> {
    fsx::write_json_atomic(ctx, &settings_path(&ctx.data_dir), settings)
}

/// The sections other features read and write through PRJ (R4, R8).
pub const SECTIONS: [&str; 3] = ["library", "spend", "provider"];

fn unknown_section(name: &str) -> CoreError {
    CoreError::invalid(
        "section",
        format!("{name:?} is not one of {}", SECTIONS.join(", ")),
    )
}

/// `settings_get(section)` (R4): one section as JSON.
pub fn section_get(settings: &Settings, name: &str) -> CoreResult<Value> {
    let v = match name {
        "library" => serde_json::to_value(&settings.library),
        "spend" => serde_json::to_value(&settings.spend),
        "provider" => serde_json::to_value(&settings.provider),
        other => return Err(unknown_section(other)),
    };
    v.map_err(|e| CoreError::internal(format!("serialising settings: {e}")))
}

/// `settings_put(section)` (R4, FR-PRJ-39): replaces one section, validated by its shape, and
/// writes the file (temp → rename). On any error `settings` is unchanged.
pub fn section_put(
    ctx: &Context,
    settings: &mut Settings,
    name: &str,
    value: Value,
) -> CoreResult<()> {
    let bad = |e: serde_json::Error| CoreError::invalid(name, format!("invalid {name} settings: {e}"));
    let mut next = settings.clone();
    match name {
        "library" => next.library = serde_json::from_value(value).map_err(bad)?,
        "spend" => next.spend = serde_json::from_value(value).map_err(bad)?,
        "provider" => next.provider = serde_json::from_value(value).map_err(bad)?,
        other => return Err(unknown_section(other)),
    }
    save(ctx, &next)?;
    *settings = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Timestamp;

    fn ctx(dir: &Path) -> Context {
        Context::deterministic(dir.to_path_buf(), Timestamp(1_800_000_000_000))
    }

    #[test]
    fn fr_prj_39_first_run_creates_settings_with_install_id() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let (s, w) = load_or_init(&c).unwrap();
        assert!(w.is_empty());
        let (again, _) = load_or_init(&c).unwrap();
        assert_eq!(s.install_id, again.install_id);
        let text = std::fs::read_to_string(settings_path(dir.path())).unwrap();
        let keys: Vec<_> = text
            .lines()
            .filter(|l| l.starts_with("  \""))
            .map(|l| l.trim().split('"').nth(1).unwrap().to_owned())
            .collect();
        assert_eq!(
            keys,
            [
                "schema_version",
                "install_id",
                "library",
                "spend",
                "provider"
            ]
        );
    }

    #[test]
    fn fr_prj_39_corrupt_settings_reset_to_defaults_and_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        std::fs::write(settings_path(dir.path()), b"{ not json").unwrap();
        let (s, w) = load_or_init(&c).unwrap();
        assert_eq!(s.library, LibrarySettings::default());
        assert_eq!(w[0].code, "settings_corrupt");
        let aside = dir.path().join("settings.corrupt-20270115-080000.json");
        assert_eq!(std::fs::read(aside).unwrap(), b"{ not json");
        assert!(settings_path(dir.path()).exists());
    }

    #[test]
    fn fr_prj_39_unknown_sections_and_fields_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let id = c.new_id();
        let text = format!(
            r#"{{"schema_version":1,"install_id":"{id}","library":{{"roots":[],"import_dest":null,"zoom":2}},"spend":{{"cap":5.0,"threshold":null}},"provider":{{}},"future":{{"x":1}}}}"#
        );
        std::fs::write(settings_path(dir.path()), text).unwrap();
        let (s, w) = load_or_init(&c).unwrap();
        assert!(w.is_empty());
        assert_eq!(s.spend.cap, Some(5.0));
        save(&c, &s).unwrap();
        let back: Value =
            serde_json::from_slice(&std::fs::read(settings_path(dir.path())).unwrap()).unwrap();
        assert_eq!(back["future"]["x"], 1);
        assert_eq!(back["library"]["zoom"], 2);
    }

    #[test]
    fn fr_prj_39_retired_import_dest_is_kept_and_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let id = c.new_id();
        let text = format!(
            r#"{{"schema_version":1,"install_id":"{id}","library":{{"roots":[],"import_dest":"/old/Imported"}}}}"#
        );
        std::fs::write(settings_path(dir.path()), text).unwrap();
        let (s, w) = load_or_init(&c).unwrap();
        assert!(w.is_empty());
        assert_eq!(s.library.extra["import_dest"], "/old/Imported");
        save(&c, &s).unwrap();
        let back: Value =
            serde_json::from_slice(&std::fs::read(settings_path(dir.path())).unwrap()).unwrap();
        assert_eq!(back["library"]["import_dest"], "/old/Imported");
    }

    #[test]
    fn fr_prj_39_newer_schema_is_not_overwritten_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let id = c.new_id();
        std::fs::write(
            settings_path(dir.path()),
            format!(r#"{{"schema_version":9,"install_id":"{id}"}}"#),
        )
        .unwrap();
        let (_, w) = load_or_init(&c).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
