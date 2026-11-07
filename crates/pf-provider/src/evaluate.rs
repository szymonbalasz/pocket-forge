//! `Capabilities::evaluate` (FR-PRV-30, #37): pure. Applies `rules[]` to a set of control
//! values and returns the visible controls with their effective allowed values and required
//! flags, plus per-control errors. RGN calls it at its command boundary.
//!
//! `values` maps control names to values (the prompt, count and seed under their control
//! names too). Source and mask images are bound by role (R28) and never checked here.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::{Capabilities, Control, ControlRole, ControlType, RuleOp};

/// A control as it applies to these values.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EffectiveControl {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ControlType,
    pub role: ControlRole,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub allowed: Vec<Value>,
    pub default: Option<Value>,
    pub required: bool,
    pub cost_affecting: bool,
}

/// Why a value doesn't fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlErrorCode {
    Required,
    WrongType,
    OutOfRange,
    Step,
    NotAllowed,
    UnknownControl,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ControlError {
    pub control: String,
    pub code: ControlErrorCode,
    pub message: String,
}

/// Result of [`Capabilities::evaluate`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evaluation {
    /// Visible controls only, in capability order.
    pub controls: Vec<EffectiveControl>,
    pub errors: Vec<ControlError>,
}

impl Evaluation {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

fn effective(c: &Control) -> EffectiveControl {
    EffectiveControl {
        name: c.name.clone(),
        kind: c.kind,
        role: c.role,
        min: c.min,
        max: c.max,
        step: c.step,
        allowed: c.allowed.clone(),
        default: c.default.clone(),
        required: c.required,
        cost_affecting: c.cost_affecting,
    }
}

fn err(control: &str, code: ControlErrorCode, message: impl Into<String>) -> ControlError {
    ControlError {
        control: control.to_owned(),
        code,
        message: message.into(),
    }
}

/// Loose equality: numbers compare by value (`2` == `2.0`).
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn check_value(c: &EffectiveControl, v: &Value) -> Option<ControlError> {
    use ControlErrorCode as E;
    let n = &c.name;
    let number = |integer: bool| -> Option<ControlError> {
        let Some(f) = v.as_f64() else {
            return Some(err(n, E::WrongType, "expected a number"));
        };
        if integer && f.fract() != 0.0 {
            return Some(err(n, E::WrongType, "expected a whole number"));
        }
        if c.min.is_some_and(|m| f < m) || c.max.is_some_and(|m| f > m) {
            return Some(err(
                n,
                E::OutOfRange,
                format!("must be between {:?} and {:?}", c.min, c.max),
            ));
        }
        if let Some(step) = c.step.filter(|s| *s > 0.0) {
            let k = (f - c.min.unwrap_or(0.0)) / step;
            if (k - k.round()).abs() > 1e-6 {
                return Some(err(n, E::Step, format!("must be a multiple of {step}")));
            }
        }
        None
    };
    let found = match c.kind {
        ControlType::Int => number(true),
        ControlType::Float => number(false),
        ControlType::Seed => match v.as_u64() {
            Some(_) => number(true),
            None => Some(err(
                n,
                E::WrongType,
                "a seed is a non-negative whole number",
            )),
        },
        ControlType::Bool if !v.is_boolean() => {
            Some(err(n, E::WrongType, "expected true or false"))
        }
        ControlType::String if !v.is_string() => Some(err(n, E::WrongType, "expected text")),
        ControlType::Enum | ControlType::Bool | ControlType::String => None,
        ControlType::Image | ControlType::Mask => None,
    };
    found.or_else(|| {
        (!c.allowed.is_empty() && !c.allowed.iter().any(|a| same(a, v)))
            .then(|| err(n, E::NotAllowed, "not one of the allowed values"))
    })
}

impl Capabilities {
    /// Applies the rules to `values` (FR-PRV-30). Pure; < 10 ms (spec §12).
    pub fn evaluate(&self, values: &BTreeMap<String, Value>) -> Evaluation {
        let mut controls: Vec<(EffectiveControl, bool)> =
            self.controls.iter().map(|c| (effective(c), true)).collect();
        let value_of = |name: &str| -> Option<Value> {
            values.get(name).cloned().or_else(|| {
                self.controls
                    .iter()
                    .find(|c| c.name == name)
                    .and_then(|c| c.default.clone())
            })
        };
        for rule in &self.rules {
            let Some(current) = value_of(&rule.when.control) else {
                continue;
            };
            let holds = match rule.when.op {
                RuleOp::Eq => same(&current, &rule.when.value),
                RuleOp::In => rule
                    .when
                    .value
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| same(x, &current))),
            };
            if !holds {
                continue;
            }
            if let Some((c, visible)) = controls
                .iter_mut()
                .find(|(c, _)| c.name == rule.then.control)
            {
                if let Some(r) = rule.then.required {
                    c.required = r;
                }
                if let Some(a) = &rule.then.allowed {
                    c.allowed = a.clone();
                }
                if let Some(h) = rule.then.hidden {
                    *visible = !h;
                }
            }
        }

        let mut errors = vec![];
        for name in values.keys() {
            if !self.controls.iter().any(|c| &c.name == name) {
                errors.push(err(
                    name,
                    ControlErrorCode::UnknownControl,
                    "this model has no such control",
                ));
            }
        }
        let visible: Vec<EffectiveControl> = controls
            .into_iter()
            .filter_map(|(c, v)| v.then_some(c))
            .collect();
        for c in &visible {
            if matches!(c.role, ControlRole::Source | ControlRole::Mask) {
                continue;
            }
            match values.get(&c.name) {
                Some(v) if !v.is_null() => errors.extend(check_value(c, v)),
                _ if c.required && c.default.is_none() => {
                    errors.push(err(&c.name, ControlErrorCode::Required, "required"))
                }
                _ => {}
            }
        }
        Evaluation {
            controls: visible,
            errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InputPx, Rule, RuleCondition, RuleEffect};
    use serde_json::json;

    fn ctl(name: &str, kind: ControlType) -> Control {
        Control {
            name: name.into(),
            kind,
            role: ControlRole::Other,
            min: None,
            max: None,
            step: None,
            allowed: vec![],
            default: None,
            required: false,
            cost_affecting: false,
        }
    }

    fn caps(controls: Vec<Control>, rules: Vec<Rule>) -> Capabilities {
        Capabilities {
            model_id: "m".into(),
            operations: vec![],
            controls,
            rules,
            mask: None,
            input_px: InputPx {
                min: 64,
                max: 4096,
                multiple_of: 8,
            },
            max_bytes: 1,
            seed: false,
            prompt_required: false,
            slots: vec![],
            warnings: vec![],
        }
    }

    fn when(control: &str, op: RuleOp, value: Value, then: RuleEffect) -> Rule {
        Rule {
            when: RuleCondition {
                control: control.into(),
                op,
                value,
            },
            then,
        }
    }

    fn effect(control: &str) -> RuleEffect {
        RuleEffect {
            control: control.into(),
            required: None,
            allowed: None,
            hidden: None,
        }
    }

    fn vals(v: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(v).unwrap()
    }

    fn model() -> Capabilities {
        let mut mode = ctl("mode", ControlType::Enum);
        mode.allowed = vec![json!("edit"), json!("inpaint"), json!("remix")];
        mode.default = Some(json!("edit"));
        let mut strength = ctl("strength", ControlType::Float);
        (strength.min, strength.max, strength.step) = (Some(0.0), Some(1.0), Some(0.25));
        let mut count = ctl("count", ControlType::Int);
        (count.min, count.max) = (Some(1.0), Some(4.0));
        count.role = ControlRole::CandidateCount;
        let mut quality = ctl("quality", ControlType::Enum);
        quality.allowed = vec![json!("low"), json!("high")];
        let mut source = ctl("image", ControlType::Image);
        source.role = ControlRole::Source;
        source.required = true;
        caps(
            vec![
                mode,
                strength,
                count,
                quality,
                ctl("region", ControlType::String),
                ctl("magic", ControlType::Bool),
                ctl("seed", ControlType::Seed),
                source,
            ],
            vec![
                when(
                    "mode",
                    RuleOp::In,
                    json!(["edit", "remix"]),
                    RuleEffect {
                        required: Some(true),
                        ..effect("strength")
                    },
                ),
                when(
                    "mode",
                    RuleOp::Eq,
                    json!("inpaint"),
                    RuleEffect {
                        hidden: Some(true),
                        ..effect("strength")
                    },
                ),
                when(
                    "mode",
                    RuleOp::Eq,
                    json!("remix"),
                    RuleEffect {
                        allowed: Some(vec![json!("high")]),
                        ..effect("quality")
                    },
                ),
            ],
        )
    }

    /// Rule tables (FR-PRV-30): `in`, `eq`, required, hidden, allowed; defaults drive rules.
    #[test]
    fn fr_prv_30_rule_table() {
        let m = model();
        // Default mode "edit" → strength required.
        let e = m.evaluate(&vals(json!({})));
        assert_eq!(
            e.errors
                .iter()
                .map(|x| (x.control.as_str(), x.code))
                .collect::<Vec<_>>(),
            [("strength", ControlErrorCode::Required)]
        );
        assert!(
            e.controls
                .iter()
                .any(|c| c.name == "strength" && c.required)
        );

        // inpaint → strength hidden, nothing required.
        let e = m.evaluate(&vals(json!({"mode": "inpaint"})));
        assert!(e.is_valid(), "{:?}", e.errors);
        assert!(e.controls.iter().all(|c| c.name != "strength"));

        // remix → quality narrowed to [high].
        let e = m.evaluate(&vals(
            json!({"mode": "remix", "strength": 0.5, "quality": "low"}),
        ));
        let q = e.controls.iter().find(|c| c.name == "quality").unwrap();
        assert_eq!(q.allowed, [json!("high")]);
        assert_eq!(e.errors[0].code, ControlErrorCode::NotAllowed);
        assert!(
            m.evaluate(&vals(
                json!({"mode": "remix", "strength": 0.5, "quality": "high"})
            ))
            .is_valid()
        );
    }

    #[test]
    fn fr_prv_30_value_checks() {
        let m = model();
        let codes = |v: Value| -> Vec<(String, ControlErrorCode)> {
            m.evaluate(&vals(v))
                .errors
                .into_iter()
                .map(|e| (e.control, e.code))
                .collect()
        };
        use ControlErrorCode as E;
        assert_eq!(
            codes(json!({"strength": 2.0})),
            [("strength".into(), E::OutOfRange)]
        );
        assert_eq!(
            codes(json!({"strength": 0.3})),
            [("strength".into(), E::Step)]
        );
        assert_eq!(
            codes(json!({"strength": "x"})),
            [("strength".into(), E::WrongType)]
        );
        assert_eq!(
            codes(json!({"strength": 0.5, "count": 2.5})),
            [("count".into(), E::WrongType)]
        );
        assert_eq!(
            codes(json!({"strength": 0.5, "count": 9})),
            [("count".into(), E::OutOfRange)]
        );
        assert_eq!(
            codes(json!({"strength": 0.5, "magic": 1})),
            [("magic".into(), E::WrongType)]
        );
        assert_eq!(
            codes(json!({"strength": 0.5, "seed": -1})),
            [("seed".into(), E::WrongType)]
        );
        assert_eq!(
            codes(json!({"strength": 0.5, "mode": "other"})),
            [("mode".into(), E::NotAllowed)]
        );
        assert_eq!(
            codes(json!({"strength": 0.5, "nope": 1})),
            [("nope".into(), E::UnknownControl)]
        );
        assert_eq!(
            codes(json!({"strength": 0.5, "region": 3})),
            [("region".into(), E::WrongType)]
        );
        // The source image is bound by role: never required here.
        assert!(codes(json!({"strength": 0.75, "count": 4, "seed": 12})).is_empty());
    }

    #[test]
    fn fr_prv_30_evaluate_is_pure_and_fast() {
        let m = model();
        let v = vals(json!({"mode": "remix", "strength": 0.5}));
        let a = m.evaluate(&v);
        let t = std::time::Instant::now();
        for _ in 0..1000 {
            assert_eq!(m.evaluate(&v), a);
        }
        assert!(t.elapsed().as_millis() < 1000);
    }
}
