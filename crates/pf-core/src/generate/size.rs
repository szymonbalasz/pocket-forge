//! The Size block's fit (FR-GEN-07–10, OQ-GEN-06): a grid (cols × rows × px per square) or a
//! ratio with a long side, fitted to the model's `output_size`. Pure: no I/O, no provider call.
//!
//! - **wh models**: pps is lowered at the model limit (notice `model_limit`), each side is then
//!   rounded down to `step` and kept within `min`–`max`. Under the model's `min_pixels` area,
//!   both sides are scaled up by the same factor and rounded up to `step` (notice
//!   `model_minimum`; a grid's pps follows the sent size).
//! - **aspect models**: the allowed ratio nearest to the asked one (smallest |ln a − ln t|);
//!   a grid more than 1 % off gets the notice `cells_not_square`.

use pf_provider::{OutputRequest, OutputSize};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Grid limits (FR-GEN-07).
pub const MAX_CELLS: u32 = 200;
pub const MIN_PPS: u32 = 16;
pub const MAX_PPS: u32 = 512;
/// The longest long side asked for (FR-REF-22's 16384 px input cap).
pub const MAX_LONG_SIDE: u32 = 16_384;
/// "Off by > 1 %" (FR-GEN-09).
pub const SQUARE_TOLERANCE: f64 = 0.01;
/// The ratios a wh model's Tokens and Items select offers (FR-GEN-10).
pub const WH_RATIOS: [&str; 9] = [
    "1:1", "4:5", "5:4", "3:4", "4:3", "2:3", "3:2", "9:16", "16:9",
];

/// What the New image tab asks for (`draft.generate.size`, schema §4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum GenSize {
    /// Maps: cols × rows squares at `pps` px per square.
    Grid { cols: u32, rows: u32, pps: u32 },
    /// Tokens and Items: a `w:h` ratio and the long side in px.
    Ratio { aspect: String, long_side: u32 },
}

impl GenSize {
    /// FR-GEN-07/10 bounds.
    pub fn validate(&self, field: &str) -> CoreResult<()> {
        match self {
            GenSize::Grid { cols, rows, pps } => {
                if !(1..=MAX_CELLS).contains(cols) || !(1..=MAX_CELLS).contains(rows) {
                    return Err(CoreError::invalid(
                        field,
                        format!("cols and rows are 1 to {MAX_CELLS}"),
                    ));
                }
                if !(MIN_PPS..=MAX_PPS).contains(pps) {
                    return Err(CoreError::invalid(
                        field,
                        format!("px per square is {MIN_PPS} to {MAX_PPS}"),
                    ));
                }
            }
            GenSize::Ratio { aspect, long_side } => {
                if ratio_value(aspect).is_none() {
                    return Err(CoreError::invalid(field, "the ratio must look like 4:3"));
                }
                if !(1..=MAX_LONG_SIDE).contains(long_side) {
                    return Err(CoreError::invalid(
                        field,
                        format!("the long side is 1 to {MAX_LONG_SIDE} px"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// The asked width : height.
    fn target(&self) -> f64 {
        match self {
            GenSize::Grid { cols, rows, .. } => f64::from(*cols) / f64::from(*rows),
            GenSize::Ratio { aspect, .. } => ratio_value(aspect).unwrap_or(1.0),
        }
    }
}

/// `a:b` → a / b, for positive finite numbers; anything else (`auto`, `source`) → `None`.
pub fn ratio_value(s: &str) -> Option<f64> {
    let (a, b) = s.split_once(':')?;
    let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
    (a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0).then_some(a / b)
}

/// A Size block notice (`model_limit`, `model_minimum`, `cells_not_square`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenNotice {
    pub code: &'static str,
    pub message: String,
}

/// `gen_size_fit` result: what is sent. wh models get `w`/`h`, aspect models `aspect`; `pps`
/// is the grid's px per square after a model-limit lowering (`null` for a ratio size).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenSizeFit {
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    pub pps: Option<u32>,
    pub notices: Vec<GenNotice>,
}

/// A generate job's `output{mode, w?, h?, aspect?, grid?}` (schema §4.6, FR-GEN-22).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenOutput {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Grid>,
}

/// The grid a map was asked at, with the px per square sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grid {
    pub cols: u32,
    pub rows: u32,
    pub pps: u32,
}

impl GenSizeFit {
    /// PRV's `JobRequest.output`.
    pub fn request(&self) -> OutputRequest {
        match (&self.aspect, self.w, self.h) {
            (Some(a), _, _) => OutputRequest::Aspect { aspect: a.clone() },
            (None, w, h) => OutputRequest::Wh {
                w: w.unwrap_or(0),
                h: h.unwrap_or(0),
            },
        }
    }

    /// The job's `output{}` for `size`.
    pub fn output(&self, size: &GenSize) -> GenOutput {
        GenOutput {
            mode: self.mode.to_owned(),
            w: self.w,
            h: self.h,
            aspect: self.aspect.clone(),
            grid: match (size, self.pps) {
                (GenSize::Grid { cols, rows, .. }, Some(pps)) => Some(Grid {
                    cols: *cols,
                    rows: *rows,
                    pps,
                }),
                _ => None,
            },
        }
    }
}

/// FR-GEN-08/09/10: fits `size` to the model's `output_size`. `size` must be valid.
pub fn fit(output: &OutputSize, size: &GenSize) -> CoreResult<GenSizeFit> {
    match output {
        OutputSize::Wh {
            min,
            max,
            step,
            min_pixels,
        } => Ok(fit_wh(*min, *max, *step, *min_pixels, size)),
        OutputSize::Aspect { allowed } => fit_aspect(allowed, size),
    }
}

fn fit_wh(min: u32, max: u32, step: u32, min_pixels: Option<u64>, size: &GenSize) -> GenSizeFit {
    let step = step.max(1);
    let max = max.max(1);
    let min = min.min(max);
    // The largest multiple of `step` within `max` (never under `min`).
    let top = ((max / step) * step).max(min);
    let side = |v: f64| {
        let v = v.max(0.0).floor() as u64;
        let snapped = (v / u64::from(step)) * u64::from(step);
        (snapped.min(u64::from(top)) as u32).max(min)
    };
    let mut notices = vec![];
    match size {
        GenSize::Grid { cols, rows, pps } => {
            let longest = (*cols).max(*rows);
            let mut pps = *pps;
            if u64::from(longest) * u64::from(pps) > u64::from(max) {
                pps = (max / longest).max(1);
                notices.push(GenNotice {
                    code: "model_limit",
                    message: format!("Model limit: {pps} px per square (at most {max} px a side)."),
                });
            }
            let (mut w, mut h) = (side(f64::from(*cols * pps)), side(f64::from(*rows * pps)));
            if let Some((sw, sh)) = scale_to_area(w, h, size.target(), step, top, min_pixels) {
                (w, h) = (sw, sh);
                pps = (w / *cols).min(h / *rows).max(1);
                notices.push(minimum_notice(w, h, min_pixels));
            }
            GenSizeFit {
                mode: "wh",
                w: Some(w),
                h: Some(h),
                aspect: None,
                pps: Some(pps),
                notices,
            }
        }
        GenSize::Ratio { long_side, .. } => {
            let r = size.target();
            let long = f64::from((*long_side).clamp(min, max));
            let (w, h) = if r >= 1.0 {
                (long, long / r)
            } else {
                (long * r, long)
            };
            let (mut w, mut h) = (side(w), side(h));
            if let Some((sw, sh)) = scale_to_area(w, h, r, step, top, min_pixels) {
                (w, h) = (sw, sh);
                notices.push(minimum_notice(w, h, min_pixels));
            }
            GenSizeFit {
                mode: "wh",
                w: Some(w),
                h: Some(h),
                aspect: None,
                pps: None,
                notices,
            }
        }
    }
}

/// Under `min_pixels`: both sides scaled up by the same factor (from the asked ratio `r`),
/// each rounded up to `step` and kept within `top`; a side held at `top` leaves the other to
/// grow. `None` when `w` × `h` is already enough (or there is no minimum).
fn scale_to_area(
    w: u32,
    h: u32,
    r: f64,
    step: u32,
    top: u32,
    min_pixels: Option<u64>,
) -> Option<(u32, u32)> {
    let m = min_pixels?;
    let area = |w: u32, h: u32| u64::from(w) * u64::from(h);
    if area(w, h) >= m {
        return None;
    }
    let up = |v: f64| {
        let v = v.max(1.0).ceil() as u64;
        let snapped = v.div_ceil(u64::from(step)) * u64::from(step);
        snapped.min(u64::from(top)) as u32
    };
    let ideal_h = (m as f64 / r).sqrt();
    let (mut w, mut h) = (up(ideal_h * r).max(w), up(ideal_h).max(h));
    // Float rounding or a side held at `top`: grow the other side a step at a time.
    while area(w, h) < m && (w < top || h < top) {
        if h >= top || (w < top && area(w + step, h) <= area(w, h + step)) {
            w = (w + step).min(top);
        } else {
            h = (h + step).min(top);
        }
    }
    Some((w, h))
}

fn minimum_notice(w: u32, h: u32, min_pixels: Option<u64>) -> GenNotice {
    GenNotice {
        code: "model_minimum",
        message: format!(
            "Model minimum: sent {w} × {h} (at least {} px in all).",
            group(min_pixels.unwrap_or(0))
        ),
    }
}

/// 3981312 → "3 981 312".
fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn fit_aspect(allowed: &[String], size: &GenSize) -> CoreResult<GenSizeFit> {
    let target = size.target();
    let best = allowed
        .iter()
        .filter_map(|a| ratio_value(a).map(|v| (a, v)))
        .min_by(|(_, a), (_, b)| {
            let d = |v: f64| (v.ln() - target.ln()).abs();
            d(*a).total_cmp(&d(*b))
        })
        .ok_or_else(|| CoreError::invalid("model_id", "this model offers no output ratio"))?;
    let mut notices = vec![];
    let pps = match size {
        GenSize::Grid { pps, .. } => {
            if (best.1 / target - 1.0).abs() > SQUARE_TOLERANCE {
                notices.push(GenNotice {
                    code: "cells_not_square",
                    message: "Cells won’t be square.".into(),
                });
            }
            Some(*pps)
        }
        GenSize::Ratio { .. } => None,
    };
    Ok(GenSizeFit {
        mode: "aspect",
        w: None,
        h: None,
        aspect: Some(best.0.clone()),
        pps,
        notices,
    })
}
