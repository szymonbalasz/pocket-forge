//! Export size maths and limits (FR-EXP-06, FR-EXP-07, FR-EXP-08, FR-EXP-11). Pure functions:
//! `pf-core` validates settings and builds the plan with them.

use crate::probe::ImageFormat;
use crate::webp::WEBP_MAX_SIDE;

/// Source and target px per square range (FR-EXP-06, FR-EXP-07).
pub const MIN_PX_PER_SQUARE: u32 = 8;
pub const MAX_PX_PER_SQUARE: u32 = 1024;
/// Grid size range in cells, for `cols` and `rows` (FR-EXP-06).
pub const MAX_CELLS: u32 = 1000;
/// Largest scale (FR-EXP-08, OQ-EXP-05).
pub const MAX_SCALE: f64 = 4.0;
/// Largest output side (FR-EXP-08).
pub const MAX_OUTPUT_SIDE: u32 = 16384;
/// Width ÷ cols and height ÷ rows may differ by this fraction before "Grid isn't square".
const SQUARE_TOLERANCE: f64 = 0.01;

/// How the owner describes the grid (FR-EXP-06).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridInput {
    /// Source px per square, 8–1024.
    PxPerSquare(f64),
    /// Grid size in cells: `cols` 1–1000, optional `rows` 1–1000.
    Cells { cols: u32, rows: Option<u32> },
}

/// The bound a setting broke (`invalid_settings{field, limit}`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Limit {
    Min(f64),
    Max(f64),
    /// The setting needs another one first (target needs a source, FR-EXP-07).
    Requires(&'static str),
}

/// A setting outside its limits (`invalid_settings{field, limit}`).
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{field} is out of range ({limit:?})")]
pub struct InvalidSetting {
    pub field: &'static str,
    pub limit: Limit,
}

/// Non-blocking plan warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeWarning {
    /// Width ÷ cols and height ÷ rows differ by more than 1 % (FR-EXP-06).
    GridNotSquare,
    /// Scale > 1 (FR-EXP-08).
    Upscaled,
}

impl SizeWarning {
    /// UI text.
    pub fn message(self) -> &'static str {
        match self {
            SizeWarning::GridNotSquare => "Grid isn't square",
            SizeWarning::Upscaled => "Upscaled, softer",
        }
    }
}

/// The export size for a source image and settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Sizing {
    pub source_px_per_square: Option<f64>,
    pub target_px_per_square: Option<u32>,
    /// target ÷ source, or 1 ("Full resolution").
    pub scale: f64,
    pub width: u32,
    pub height: u32,
    pub warnings: Vec<SizeWarning>,
}

impl Sizing {
    /// Whether the export must resample (any scale ≠ 1, FR-EXP-09).
    pub fn rescales(&self) -> bool {
        self.scale != 1.0
    }
}

fn invalid(field: &'static str, limit: Limit) -> InvalidSetting {
    InvalidSetting { field, limit }
}

fn check_range(field: &'static str, value: f64, min: f64, max: f64) -> Result<(), InvalidSetting> {
    if value.is_nan() || value < min {
        Err(invalid(field, Limit::Min(min)))
    } else if value > max {
        Err(invalid(field, Limit::Max(max)))
    } else {
        Ok(())
    }
}

/// Source px per square from the grid input, with the "Grid isn't square" warning (FR-EXP-06).
pub fn source_px_per_square(
    grid: GridInput,
    width: u32,
    height: u32,
) -> Result<(f64, Option<SizeWarning>), InvalidSetting> {
    let (min, max) = (f64::from(MIN_PX_PER_SQUARE), f64::from(MAX_PX_PER_SQUARE));
    match grid {
        GridInput::PxPerSquare(v) => {
            check_range("source_px_per_square", v, min, max)?;
            Ok((v, None))
        }
        GridInput::Cells { cols, rows } => {
            check_range("cols", f64::from(cols), 1.0, f64::from(MAX_CELLS))?;
            let source = f64::from(width) / f64::from(cols);
            let mut warning = None;
            if let Some(rows) = rows {
                check_range("rows", f64::from(rows), 1.0, f64::from(MAX_CELLS))?;
                let by_rows = f64::from(height) / f64::from(rows);
                if (source - by_rows).abs() > SQUARE_TOLERANCE * source {
                    warning = Some(SizeWarning::GridNotSquare);
                }
            }
            Ok((source, warning))
        }
    }
}

/// Output size and scale for a `width × height` source (FR-EXP-06..08).
pub fn plan_size(
    width: u32,
    height: u32,
    grid: Option<GridInput>,
    target: Option<u32>,
) -> Result<Sizing, InvalidSetting> {
    let mut warnings = Vec::new();
    let source = match grid {
        Some(g) => {
            let (s, w) = source_px_per_square(g, width, height)?;
            warnings.extend(w);
            Some(s)
        }
        None => None,
    };
    let scale = match (source, target) {
        (_, None) => 1.0,
        (None, Some(_)) => {
            return Err(invalid(
                "target_px_per_square",
                Limit::Requires("source_px_per_square"),
            ));
        }
        (Some(s), Some(t)) => {
            check_range(
                "target_px_per_square",
                f64::from(t),
                f64::from(MIN_PX_PER_SQUARE),
                f64::from(MAX_PX_PER_SQUARE),
            )?;
            f64::from(t) / s
        }
    };
    if scale > MAX_SCALE {
        return Err(invalid("scale", Limit::Max(MAX_SCALE)));
    }
    let side = |field: &'static str, v: u32| -> Result<u32, InvalidSetting> {
        let out = (f64::from(v) * scale).round();
        check_range(field, out, 1.0, f64::from(MAX_OUTPUT_SIDE))?;
        Ok(out as u32)
    };
    let (out_w, out_h) = (side("width", width)?, side("height", height)?);
    if scale > 1.0 {
        warnings.push(SizeWarning::Upscaled);
    }
    Ok(Sizing {
        source_px_per_square: source,
        target_px_per_square: target,
        scale,
        width: out_w,
        height: out_h,
        warnings,
    })
}

/// Why `format` can't encode a `width × height` export, or `None` when it can (FR-EXP-11).
pub fn format_unavailable(format: ImageFormat, width: u32, height: u32) -> Option<&'static str> {
    match format {
        ImageFormat::Webp if width > WEBP_MAX_SIDE || height > WEBP_MAX_SIDE => {
            Some("WebP max 16383 px")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(cols: u32, rows: Option<u32>) -> Option<GridInput> {
        Some(GridInput::Cells { cols, rows })
    }

    #[test]
    fn fr_exp_06_cells_give_source_as_width_over_cols() {
        let (s, w) = source_px_per_square(
            GridInput::Cells {
                cols: 30,
                rows: Some(40),
            },
            4200,
            5600,
        )
        .unwrap();
        assert_eq!((s, w), (140.0, None));
    }

    #[test]
    fn fr_exp_06_non_square_grid_warns_without_blocking() {
        // 4200/30 = 140 vs 5600/39 ≈ 143.6: 2.6 % apart.
        let s = plan_size(4200, 5600, cells(30, Some(39)), None).unwrap();
        assert_eq!(s.warnings, [SizeWarning::GridNotSquare]);
        // Within 1 %: 4200/30 = 140 vs 5650/40 = 141.25 (0.9 % apart).
        let s = plan_size(4200, 5650, cells(30, Some(40)), None).unwrap();
        assert!(s.warnings.is_empty(), "0.9 % apart is square enough");
    }

    #[test]
    fn fr_exp_06_validates_grid_ranges() {
        let table: [(GridInput, &str, Limit); 6] = [
            (
                GridInput::PxPerSquare(7.9),
                "source_px_per_square",
                Limit::Min(8.0),
            ),
            (
                GridInput::PxPerSquare(1024.5),
                "source_px_per_square",
                Limit::Max(1024.0),
            ),
            (
                GridInput::PxPerSquare(f64::NAN),
                "source_px_per_square",
                Limit::Min(8.0),
            ),
            (
                GridInput::Cells {
                    cols: 0,
                    rows: None,
                },
                "cols",
                Limit::Min(1.0),
            ),
            (
                GridInput::Cells {
                    cols: 1001,
                    rows: None,
                },
                "cols",
                Limit::Max(1000.0),
            ),
            (
                GridInput::Cells {
                    cols: 10,
                    rows: Some(0),
                },
                "rows",
                Limit::Min(1.0),
            ),
        ];
        for (grid, field, limit) in table {
            assert_eq!(
                plan_size(4200, 5600, Some(grid), None).unwrap_err(),
                InvalidSetting { field, limit },
                "{grid:?}"
            );
        }
        assert!(plan_size(4200, 5600, Some(GridInput::PxPerSquare(8.0)), None).is_ok());
        assert!(plan_size(4200, 5600, Some(GridInput::PxPerSquare(1024.0)), None).is_ok());
    }

    #[test]
    fn fr_exp_07_no_target_is_full_resolution() {
        let s = plan_size(4200, 5600, cells(30, None), None).unwrap();
        assert_eq!((s.scale, s.width, s.height), (1.0, 4200, 5600));
        assert!(!s.rescales());
        let s = plan_size(4200, 5600, None, None).unwrap();
        assert_eq!((s.scale, s.source_px_per_square), (1.0, None));
    }

    #[test]
    fn fr_exp_07_target_needs_a_source_and_its_range() {
        assert_eq!(
            plan_size(4200, 5600, None, Some(70)).unwrap_err(),
            invalid(
                "target_px_per_square",
                Limit::Requires("source_px_per_square")
            )
        );
        for (t, limit) in [(7, Limit::Min(8.0)), (1025, Limit::Max(1024.0))] {
            assert_eq!(
                plan_size(4200, 5600, cells(30, None), Some(t)).unwrap_err(),
                invalid("target_px_per_square", limit)
            );
        }
    }

    #[test]
    fn fr_exp_08_output_size_is_rounded_side_times_scale() {
        // The user story: 30×40 cells at 140 px, target 70 → 2100×2800.
        let s = plan_size(4200, 5600, cells(30, Some(40)), Some(70)).unwrap();
        assert_eq!((s.scale, s.width, s.height), (0.5, 2100, 2800));
        assert!(s.rescales() && s.warnings.is_empty());
        // Rounding: 1001 × 0.5 = 500.5 → 501.
        let s = plan_size(1001, 999, Some(GridInput::PxPerSquare(140.0)), Some(70)).unwrap();
        assert_eq!((s.width, s.height), (501, 500));
    }

    #[test]
    fn fr_exp_08_upscale_warns_and_limits_apply() {
        let s = plan_size(1000, 800, Some(GridInput::PxPerSquare(70.0)), Some(140)).unwrap();
        assert_eq!(
            (s.width, s.height, s.warnings.as_slice()),
            (2000, 1600, &[SizeWarning::Upscaled][..])
        );
        // Scale > 4.
        assert_eq!(
            plan_size(100, 100, Some(GridInput::PxPerSquare(8.0)), Some(40)).unwrap_err(),
            invalid("scale", Limit::Max(4.0))
        );
        // Side > 16384.
        assert_eq!(
            plan_size(10000, 100, Some(GridInput::PxPerSquare(100.0)), Some(200)).unwrap_err(),
            invalid("width", Limit::Max(16384.0))
        );
        // A side rounding to 0.
        assert_eq!(
            plan_size(1, 4000, Some(GridInput::PxPerSquare(1000.0)), Some(8)).unwrap_err(),
            invalid("width", Limit::Min(1.0))
        );
    }

    #[test]
    fn fr_exp_11_webp_unavailable_above_16383_px() {
        assert_eq!(format_unavailable(ImageFormat::Webp, 16383, 16383), None);
        assert_eq!(
            format_unavailable(ImageFormat::Webp, 16384, 10),
            Some("WebP max 16383 px")
        );
        assert_eq!(format_unavailable(ImageFormat::Png, 16384, 16384), None);
        assert_eq!(format_unavailable(ImageFormat::Jpeg, 16384, 16384), None);
    }
}
