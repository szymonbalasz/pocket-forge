//! Export settings (manifest-schema §4.10), their limits and the plan (FR-EXP-06..08, 11, 13,
//! 14, 24). The size maths is `pf-imaging`'s `sizing`; this maps it to commands and the manifest.

use std::path::Path;

use pf_imaging::{
    GridInput, ImageFormat, InvalidSetting, Limit, MAX_CELLS, STRIP_H, Sizing, format_unavailable,
    plan_size,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::manifest::EXPORTS_DIR;
use crate::time::Timestamp;

/// How the grid is entered (FR-EXP-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridMode {
    /// Source px per square.
    Px,
    /// Grid size in cells.
    Cells,
}

/// Grid input `{mode, source_px_per_square?, cols?, rows?}`. Both modes keep their values; only
/// the active mode's are checked. Whole numbers are `i64` so an out-of-range value reaches the
/// limit check (`invalid_settings`) instead of failing to parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    pub mode: GridMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_px_per_square: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<i64>,
}

/// Output format (FR-EXP-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Jpeg,
    Webp,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::Webp];

    /// File extension (FR-EXP-14).
    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Webp => "webp",
        }
    }

    pub fn image_format(self) -> ImageFormat {
        match self {
            ExportFormat::Png => ImageFormat::Png,
            ExportFormat::Jpeg => ImageFormat::Jpeg,
            ExportFormat::Webp => ImageFormat::Webp,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpeg",
            ExportFormat::Webp => "webp",
        }
    }
}

/// The settings the owner edits (EXP §9 `export_set_settings`): manifest `export` minus `last`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportSettings {
    pub grid: Grid,
    #[serde(default)]
    pub target_px_per_square: Option<i64>,
    pub format: ExportFormat,
}

/// The limit check for whole numbers given as `i64` (a `u32` parameter of `plan_size`).
fn whole(field: &'static str, v: i64, min: i64, max: i64) -> Result<u32, InvalidSetting> {
    let limit = if v < min {
        Limit::Min(min as f64)
    } else if v > max {
        Limit::Max(max as f64)
    } else {
        return Ok(v as u32);
    };
    Err(InvalidSetting { field, limit })
}

impl ExportSettings {
    /// The settings when nothing was ever chosen: no grid, full resolution, PNG.
    pub fn defaults() -> Self {
        ExportSettings {
            grid: Grid {
                mode: GridMode::Px,
                source_px_per_square: None,
                cols: None,
                rows: None,
            },
            target_px_per_square: None,
            format: ExportFormat::Png,
        }
    }

    /// The settings stored in a manifest `export` record, if it has any.
    pub fn from_record(record: Option<&Value>) -> Option<Self> {
        let r = record?.as_object()?;
        let fields: Map<String, Value> = ["grid", "target_px_per_square", "format"]
            .into_iter()
            .filter_map(|k| r.get(k).map(|v| (k.to_owned(), v.clone())))
            .collect();
        if !fields.contains_key("grid") || !fields.contains_key("format") {
            return None;
        }
        serde_json::from_value(Value::Object(fields)).ok()
    }

    /// The fields PRJ merges into the manifest `export` record (R30, R6).
    pub fn record_fields(&self) -> Map<String, Value> {
        match serde_json::to_value(self) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        }
    }

    fn grid_input(&self) -> Result<Option<GridInput>, InvalidSetting> {
        let max = i64::from(MAX_CELLS);
        Ok(match self.grid.mode {
            GridMode::Px => self.grid.source_px_per_square.map(GridInput::PxPerSquare),
            GridMode::Cells => match self.grid.cols {
                None => None,
                Some(c) => Some(GridInput::Cells {
                    cols: whole("cols", c, 1, max)?,
                    rows: self
                        .grid
                        .rows
                        .map(|r| whole("rows", r, 1, max))
                        .transpose()?,
                }),
            },
        })
    }

    fn sizing_raw(&self, width: u32, height: u32) -> Result<Sizing, InvalidSetting> {
        let grid = self.grid_input()?;
        let target = self
            .target_px_per_square
            .map(|t| {
                if grid.is_none() {
                    // plan_size reports `requires` before the range.
                    return Ok(1);
                }
                let (min, max) = (
                    i64::from(pf_imaging::MIN_PX_PER_SQUARE),
                    i64::from(pf_imaging::MAX_PX_PER_SQUARE),
                );
                whole("target_px_per_square", t, min, max)
            })
            .transpose()?;
        plan_size(width, height, grid, target)
    }

    /// Output size and scale for a `width × height` original (FR-EXP-06..08), or
    /// `invalid_settings{field, detail.limit}` (R43).
    pub fn sizing(&self, width: u32, height: u32) -> CoreResult<Sizing> {
        self.sizing_raw(width, height).map_err(invalid_settings)
    }

    /// `(source, target)` px per square for EDT's display size (FR-EXP-26): both only when the
    /// settings are valid for this image; the source alone when only it is.
    pub fn px_per_square(&self, width: u32, height: u32) -> (Option<f64>, Option<i64>) {
        match self.sizing_raw(width, height) {
            Ok(s) => (s.source_px_per_square, self.target_px_per_square),
            Err(_) => {
                let source = self.grid_input().ok().flatten().and_then(|g| {
                    pf_imaging::source_px_per_square(g, width, height)
                        .ok()
                        .map(|(s, _)| s)
                });
                (source, None)
            }
        }
    }
}

/// `invalid_settings` with `field` and `detail.limit{min|max|requires}` (R43).
pub fn invalid_settings(e: InvalidSetting) -> CoreError {
    let limit = match e.limit {
        Limit::Min(v) => json!({ "min": v }),
        Limit::Max(v) => json!({ "max": v }),
        Limit::Requires(f) => json!({ "requires": f }),
    };
    CoreError::new(ErrorCode::InvalidSettings, e.to_string())
        .with_field(e.field)
        .with_detail(json!({ "limit": limit }))
}

/// `<project name>_<target>px.<ext>`, or `<project name>.<ext>` (FR-EXP-14).
pub fn file_name(project: &str, target: Option<i64>, format: ExportFormat) -> String {
    match target {
        Some(t) => format!("{project}_{t}px.{}", format.ext()),
        None => format!("{project}.{}", format.ext()),
    }
}

/// The project-relative destination, `exports/<file name>` (FR-EXP-13, R43).
pub fn rel_path(project: &str, target: Option<i64>, format: ExportFormat) -> String {
    format!("{EXPORTS_DIR}/{}", file_name(project, target, format))
}

/// The absolute path of a project-relative `/` path, with the platform's separators (so
/// `export:done.path` reads as one path on Windows).
pub fn abs_path(dir: &Path, rel: &str) -> std::path::PathBuf {
    rel.split('/')
        .fold(dir.to_path_buf(), |p, part| p.join(part))
}

/// `exists{bytes, mtime}` of a destination file, or `None`.
pub fn existing(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    Some(json!({
        "bytes": meta.len(),
        "mtime": crate::project::mtime_of(&meta),
    }))
}

/// One `formats[]` row of the plan (FR-EXP-11).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FormatRow {
    pub format: ExportFormat,
    pub available: bool,
    pub reason: Option<&'static str>,
}

/// A plan warning `{code, message}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanWarning {
    pub code: &'static str,
    pub message: String,
}

/// `export_plan` result (FR-EXP-24). `path` is project-relative (R43).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub formats: Vec<FormatRow>,
    pub path: String,
    pub exists: Option<Value>,
    pub est_peak_mib: u64,
    pub warnings: Vec<PlanWarning>,
}

/// Above this estimated peak the plan warns (FR-EXP-24).
pub const PEAK_WARN_MIB: u64 = 2048;

/// Estimated peak memory of an export in MiB (FR-EXP-24, EXP §12): PRJ's decoded original plus
/// a strip and its encoder copy when streaming; otherwise the full flatten, and for a rescale the
/// resampler's intermediate (output width × source height) and the output, and for WebP the
/// encoder's copy of the output.
pub fn est_peak_mib(width: u32, height: u32, sizing: &Sizing, format: ExportFormat) -> u64 {
    const MIB: f64 = 1024.0 * 1024.0;
    let rgb = |w: u32, h: u32| f64::from(w) * f64::from(h) * 3.0;
    let original = rgb(width, height);
    let streams = !sizing.rescales() && format != ExportFormat::Webp;
    let extra = if streams {
        rgb(width, STRIP_H) * 2.0
    } else {
        let out = rgb(sizing.width, sizing.height);
        let resize = if sizing.rescales() {
            rgb(sizing.width, height) + out
        } else {
            0.0
        };
        let webp = if format == ExportFormat::Webp {
            out * 2.0
        } else {
            0.0
        };
        original + resize + webp
    };
    ((original + extra) / MIB).ceil() as u64
}

/// Why `format` can't be written at this size, or `None` (FR-EXP-11).
pub fn unavailable(format: ExportFormat, sizing: &Sizing) -> Option<&'static str> {
    format_unavailable(format.image_format(), sizing.width, sizing.height)
}

/// The plan for `settings` on a `width × height` original in project `name` at `dir` (pure: it
/// only stats the destination, FR-EXP-24).
pub fn plan(
    settings: &ExportSettings,
    name: &str,
    dir: &Path,
    width: u32,
    height: u32,
) -> CoreResult<Plan> {
    let sizing = settings.sizing(width, height)?;
    let path = rel_path(name, settings.target_px_per_square, settings.format);
    let mut warnings: Vec<PlanWarning> = sizing
        .warnings
        .iter()
        .map(|w| PlanWarning {
            code: match w {
                pf_imaging::SizeWarning::GridNotSquare => "grid_not_square",
                pf_imaging::SizeWarning::Upscaled => "upscaled",
            },
            message: w.message().to_owned(),
        })
        .collect();
    let est = est_peak_mib(width, height, &sizing, settings.format);
    if est > PEAK_WARN_MIB {
        warnings.push(PlanWarning {
            code: "memory",
            message: format!("Needs about {est} MiB of memory"),
        });
    }
    Ok(Plan {
        width: sizing.width,
        height: sizing.height,
        scale: sizing.scale,
        formats: ExportFormat::ALL
            .into_iter()
            .map(|format| {
                let reason = unavailable(format, &sizing);
                FormatRow {
                    format,
                    available: reason.is_none(),
                    reason,
                }
            })
            .collect(),
        exists: existing(&abs_path(dir, &path)),
        path,
        est_peak_mib: est,
        warnings,
    })
}

/// `dest_unavailable` (EXP §9): the project folder is gone (root offline) or `exports/` can't
/// be a folder.
pub fn check_destination(dir: &Path) -> CoreResult<()> {
    let exports = dir.join(EXPORTS_DIR);
    if !dir.is_dir() || (exports.exists() && !exports.is_dir()) {
        return Err(CoreError::new(
            ErrorCode::DestUnavailable,
            "the project folder can't be written (library offline?)",
        ));
    }
    Ok(())
}

/// A `Timestamp` as JSON (for `last.at`).
pub fn stamp(t: Timestamp) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}
