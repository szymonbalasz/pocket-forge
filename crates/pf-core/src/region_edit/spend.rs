//! The spend guard (FR-RGN-14/15/16, #23, ADR 0001): session spend in memory, cap and per-job
//! threshold in `settings.json` `spend` (PRJ's settings code, R8).

use pf_provider::CostAmount;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::settings::SpendSettings;

/// One submitted job's share of the session spend.
#[derive(Debug, Clone, PartialEq)]
pub struct SpendEntry {
    pub job_id: Ulid,
    pub estimate: CostAmount,
    /// The provider's actual cost once known.
    pub actual: Option<CostAmount>,
}

impl SpendEntry {
    /// Actual when known, else the estimate; unknown counts 0 (a `lost` job keeps its
    /// estimate, FR-RGN-14).
    pub fn amount(&self) -> f64 {
        match (self.actual, self.estimate) {
            (Some(CostAmount::Known(a)), _) => a,
            (_, CostAmount::Known(e)) => e,
            _ => 0.0,
        }
    }
}

/// The session ledger: jobs submitted since app start (memory only, spec §8).
#[derive(Debug, Default, Clone)]
pub struct SpendLedger {
    entries: Vec<SpendEntry>,
}

impl SpendLedger {
    pub fn total(&self) -> f64 {
        self.entries.iter().map(SpendEntry::amount).sum()
    }

    /// Called by submit once the provider accepted the job (slice 3).
    pub fn record_submit(&mut self, job_id: Ulid, estimate: CostAmount) {
        self.entries.push(SpendEntry {
            job_id,
            estimate,
            actual: None,
        });
    }

    /// The actual cost reported by `status` (slice 3).
    pub fn record_actual(&mut self, job_id: Ulid, actual: CostAmount) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.job_id == job_id) {
            e.actual = Some(actual);
        }
    }
}

/// A confirmation the confirm dialog must collect before submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// The estimate is above the per-job threshold (FR-RGN-15).
    Threshold,
    /// The cost is unknown: always confirmed (ADR 0001).
    UnknownCost,
    /// No cap or threshold set yet: the first submit asks for both (FR-RGN-16).
    SetCap,
}

/// The confirmations a submit carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmations {
    #[serde(default)]
    pub threshold: bool,
    #[serde(default)]
    pub unknown_cost: bool,
}

/// What the confirm dialog shows (part of the estimate result).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpendView {
    pub spend: f64,
    pub cap: Option<f64>,
    pub threshold: Option<f64>,
    pub needs: Vec<Need>,
    /// Spend + estimate > cap: submit fails with `over_spend_cap` until the cap is raised.
    pub over_cap: bool,
}

/// FR-RGN-14/15/16: the confirmations an estimate needs and whether it breaks the cap.
pub fn assess(spend: f64, settings: &SpendSettings, estimate: CostAmount) -> SpendView {
    let mut needs = vec![];
    if settings.cap.is_none() || settings.threshold.is_none() {
        needs.push(Need::SetCap);
    }
    match estimate {
        CostAmount::Known(e) => {
            if settings.threshold.is_some_and(|t| e > t) {
                needs.push(Need::Threshold);
            }
        }
        CostAmount::Unknown => needs.push(Need::UnknownCost),
    }
    let add = match estimate {
        CostAmount::Known(e) => e,
        CostAmount::Unknown => 0.0,
    };
    SpendView {
        spend,
        cap: settings.cap,
        threshold: settings.threshold,
        needs,
        over_cap: settings.cap.is_some_and(|c| spend + add > c),
    }
}

/// The submit-time guard (FR-RGN-14/15/16). Slice 3's `rgn_job_submit` calls it after the
/// estimate id matched.
pub fn guard(
    view: &SpendView,
    estimate: CostAmount,
    confirmations: Confirmations,
) -> CoreResult<()> {
    if view.needs.contains(&Need::SetCap) {
        return Err(CoreError::new(
            ErrorCode::ConfirmationMissing,
            "set a spend cap and a per-job threshold first",
        )
        .with_detail(serde_json::json!({ "needs": [Need::SetCap] })));
    }
    if view.over_cap {
        let est = match estimate {
            CostAmount::Known(e) => serde_json::json!(e),
            CostAmount::Unknown => serde_json::json!("unknown"),
        };
        return Err(CoreError::new(
            ErrorCode::OverSpendCap,
            "this job would go over the session spend cap",
        )
        .with_detail(
            serde_json::json!({ "spend": view.spend, "estimate": est, "cap": view.cap }),
        ));
    }
    let missing: Vec<Need> = view
        .needs
        .iter()
        .copied()
        .filter(|n| match n {
            Need::Threshold => !confirmations.threshold,
            Need::UnknownCost => !confirmations.unknown_cost,
            Need::SetCap => true,
        })
        .collect();
    if !missing.is_empty() {
        return Err(CoreError::new(
            ErrorCode::ConfirmationMissing,
            "this job needs another confirmation",
        )
        .with_detail(serde_json::json!({ "needs": missing })));
    }
    Ok(())
}

/// Validates a cap or threshold for `rgn_spend_set`: finite and ≥ 0, or `null` (= not set).
pub fn check_amount(field: &str, v: Option<f64>) -> CoreResult<()> {
    match v {
        Some(x) if !x.is_finite() || x < 0.0 || x > 1e12 => Err(CoreError::new(
            ErrorCode::InvalidValue,
            format!("the {field} must be a number of provider units, 0 or more"),
        )
        .with_field(field.to_owned())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(cap: Option<f64>, threshold: Option<f64>) -> SpendSettings {
        SpendSettings {
            cap,
            threshold,
            ..SpendSettings::default()
        }
    }

    fn ulid(n: u128) -> Ulid {
        Ulid::from_parts(0, n)
    }

    #[test]
    fn fr_rgn_14_session_spend_uses_actual_else_estimate() {
        let mut l = SpendLedger::default();
        l.record_submit(ulid(1), CostAmount::Known(3.0));
        l.record_submit(ulid(2), CostAmount::Known(2.0));
        l.record_submit(ulid(3), CostAmount::Unknown);
        assert_eq!(l.total(), 5.0);
        l.record_actual(ulid(1), CostAmount::Known(4.5));
        l.record_actual(ulid(3), CostAmount::Known(1.0));
        assert_eq!(l.total(), 7.5, "actual replaces the estimate");
        // A lost job never gets an actual: its estimate keeps counting.
        l.record_actual(ulid(2), CostAmount::Unknown);
        assert_eq!(l.total(), 7.5);
    }

    #[test]
    fn fr_rgn_14_over_cap_is_refused_with_spend_estimate_and_cap() {
        let s = settings(Some(10.0), Some(8.0));
        let v = assess(7.0, &s, CostAmount::Known(4.0));
        assert!(v.over_cap);
        let e = guard(&v, CostAmount::Known(4.0), Confirmations::default()).unwrap_err();
        assert_eq!(e.code, ErrorCode::OverSpendCap);
        assert_eq!(
            e.detail,
            serde_json::json!({"spend": 7.0, "estimate": 4.0, "cap": 10.0})
        );
        let v = assess(6.0, &s, CostAmount::Known(4.0));
        assert!(!v.over_cap, "exactly at the cap is allowed");
        guard(&v, CostAmount::Known(4.0), Confirmations::default()).unwrap();
    }

    #[test]
    fn fr_rgn_15_threshold_and_unknown_cost_need_a_second_confirmation() {
        let s = settings(Some(100.0), Some(3.0));
        let v = assess(0.0, &s, CostAmount::Known(4.0));
        assert_eq!(v.needs, vec![Need::Threshold]);
        let e = guard(&v, CostAmount::Known(4.0), Confirmations::default()).unwrap_err();
        assert_eq!(e.code, ErrorCode::ConfirmationMissing);
        assert_eq!(e.detail["needs"], serde_json::json!(["threshold"]));
        let ok = Confirmations {
            threshold: true,
            ..Confirmations::default()
        };
        guard(&v, CostAmount::Known(4.0), ok).unwrap();
        assert!(assess(0.0, &s, CostAmount::Known(3.0)).needs.is_empty());

        let v = assess(0.0, &s, CostAmount::Unknown);
        assert_eq!(v.needs, vec![Need::UnknownCost]);
        assert!(guard(&v, CostAmount::Unknown, ok).is_err());
        let ok = Confirmations {
            unknown_cost: true,
            ..Confirmations::default()
        };
        guard(&v, CostAmount::Unknown, ok).unwrap();
    }

    #[test]
    fn fr_rgn_16_no_cap_or_threshold_asks_for_both_first() {
        for s in [
            settings(None, None),
            settings(Some(5.0), None),
            settings(None, Some(5.0)),
        ] {
            let v = assess(0.0, &s, CostAmount::Known(1.0));
            assert!(v.needs.contains(&Need::SetCap), "{s:?}");
            let all = Confirmations {
                threshold: true,
                unknown_cost: true,
            };
            let e = guard(&v, CostAmount::Known(1.0), all).unwrap_err();
            assert_eq!(e.detail["needs"], serde_json::json!(["set_cap"]));
        }
    }

    #[test]
    fn fr_rgn_16_amounts_are_validated() {
        assert!(check_amount("cap", Some(0.0)).is_ok());
        assert!(check_amount("cap", None).is_ok());
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                check_amount("cap", Some(bad)).unwrap_err().code,
                ErrorCode::InvalidValue
            );
        }
    }
}
