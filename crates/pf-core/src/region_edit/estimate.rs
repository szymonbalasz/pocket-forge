//! The estimate (FR-RGN-12/13, R15): the estimate id hashes the cost-affecting inputs (model,
//! sent size, `cost_affecting` control values). The provider is asked again only when that id
//! changes, so edits that don't affect cost (the prompt, the mask, the region position at the
//! same sent size) never re-estimate.

use std::collections::BTreeMap;

use pf_provider::Cost;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::fit::RegionFit;
use crate::time::Timestamp;

/// How long a cached estimate is reused for the same id.
pub const ESTIMATE_TTL_MS: i64 = 10 * 60 * 1000;
/// Cached estimates kept per session.
const MAX_CACHED: usize = 32;

/// FR-RGN-13/42: `est-` + SHA-256 of the canonical JSON of the cost-affecting inputs. `refs` =
/// REF's `estimate_inputs()`, the ordered `(slot, sent hash)` list of cost-affecting slots
/// (FR-REF-24); an empty list leaves the id as it was before #51.
pub fn estimate_id(
    model_id: &str,
    fit: &RegionFit,
    cost_inputs: &BTreeMap<String, Value>,
    refs: &[(String, String)],
) -> String {
    let mut canonical = serde_json::json!({
        "model": model_id,
        "sent": [fit.sent_w, fit.sent_h],
        "controls": cost_inputs,
    });
    if !refs.is_empty() {
        canonical["refs"] = serde_json::json!(refs);
    }
    hashed(&canonical)
}

/// FR-GEN-20: a generate job's estimate id hashes exactly the model, the sent size or ratio,
/// the `cost_affecting` control values and the ordered `(slot, sent hash)` of cost-affecting
/// slots (the sketch included, FR-REF-24). Never equal to a region edit's id.
pub fn generate_estimate_id(
    model_id: &str,
    output: &pf_provider::OutputRequest,
    cost_inputs: &BTreeMap<String, Value>,
    refs: &[(String, String)],
) -> String {
    hashed(&serde_json::json!({
        "operation": "generate",
        "model": model_id,
        "output": output,
        "controls": cost_inputs,
        "refs": refs,
    }))
}

fn hashed(canonical: &Value) -> String {
    let digest = Sha256::digest(canonical.to_string().as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("est-{hex}")
}

#[derive(Debug, Clone, PartialEq)]
struct Cached {
    id: String,
    cost: Cost,
    at: Timestamp,
}

/// Estimates of this session by id (memory only).
#[derive(Debug, Default, Clone)]
pub struct EstimateCache {
    entries: Vec<Cached>,
}

impl EstimateCache {
    /// A fresh cached cost for `id`.
    pub fn get(&self, id: &str, now: Timestamp) -> Option<Cost> {
        self.entries
            .iter()
            .rev()
            .find(|c| c.id == id && now.millis() - c.at.millis() <= ESTIMATE_TTL_MS)
            .map(|c| c.cost.clone())
    }

    /// The last estimate made for `id`, however old (submit compares against it, FR-RGN-13).
    pub fn last(&self, id: &str) -> Option<Cost> {
        self.entries
            .iter()
            .rev()
            .find(|c| c.id == id)
            .map(|c| c.cost.clone())
    }

    pub fn put(&mut self, id: String, cost: Cost, at: Timestamp) {
        self.entries.retain(|c| c.id != id);
        self.entries.push(Cached { id, cost, at });
        if self.entries.len() > MAX_CACHED {
            self.entries.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region_edit::fit::RectPx;
    use pf_provider::CostAmount;

    fn fit(w: u32, h: u32) -> RegionFit {
        let r = RectPx { x: 0, y: 0, w, h };
        RegionFit {
            inner: r,
            outer: r,
            margin: 0,
            sent_w: w,
            sent_h: h,
            scale: 1.0,
        }
    }

    #[test]
    fn fr_rgn_13_estimate_id_hashes_only_cost_affecting_inputs() {
        let mut c = BTreeMap::new();
        c.insert("num_samples".to_owned(), serde_json::json!(3));
        let a = estimate_id("fake-white-edit", &fit(512, 512), &c, &[]);
        assert!(a.starts_with("est-") && a.len() == 36);
        assert_eq!(a, estimate_id("fake-white-edit", &fit(512, 512), &c, &[]));
        // Region position doesn't matter, the sent size does.
        let mut moved = fit(512, 512);
        moved.outer.x = 99;
        assert_eq!(a, estimate_id("fake-white-edit", &moved, &c, &[]));
        assert_ne!(a, estimate_id("fake-white-edit", &fit(512, 520), &c, &[]));
        assert_ne!(a, estimate_id("fake-black-edit", &fit(512, 512), &c, &[]));
        c.insert("num_samples".to_owned(), serde_json::json!(4));
        assert_ne!(a, estimate_id("fake-white-edit", &fit(512, 512), &c, &[]));
    }

    #[test]
    fn fr_rgn_42_estimate_id_hashes_the_ordered_cost_affecting_refs() {
        let c = BTreeMap::new();
        let f = fit(512, 512);
        let r = |s: &str, h: &str| (s.to_owned(), h.to_owned());
        let id = |refs: &[(String, String)]| estimate_id("fake-black-edit", &f, &c, refs);
        let none = id(&[]);
        let a = id(&[r("style_refs", "sha256:a")]);
        assert_ne!(none, a);
        assert_ne!(a, id(&[r("style_refs", "sha256:c")]));
        let ab = id(&[r("style_refs", "sha256:a"), r("style_refs", "sha256:b")]);
        let ba = id(&[r("style_refs", "sha256:b"), r("style_refs", "sha256:a")]);
        assert_ne!(ab, ba, "the list is ordered");
    }

    #[test]
    fn fr_rgn_12_cached_estimates_expire() {
        let cost = Cost {
            amount: CostAmount::Known(3.0),
            unit: "fake".into(),
        };
        let mut cache = EstimateCache::default();
        cache.put("est-a".into(), cost.clone(), Timestamp(1000));
        assert_eq!(
            cache.get("est-a", Timestamp(1000 + ESTIMATE_TTL_MS)),
            Some(cost.clone())
        );
        assert_eq!(cache.get("est-a", Timestamp(1001 + ESTIMATE_TTL_MS)), None);
        assert_eq!(cache.last("est-a"), Some(cost));
        assert_eq!(cache.get("est-b", Timestamp(1000)), None);
    }
}
