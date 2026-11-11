//! The capability normaliser (FR-PRV-10/11, #37): Scenario `GET /models/{id}` → generic
//! [`Capabilities`]. Lenient by design (the schema drifts, research doc §Gaps): unknown fields
//! are ignored, an unknown input type is omitted with a warning, and only a structurally broken
//! body is an error. Never panics.
//!
//! Scenario fields read: `inputs[].{name, type, kind, min, max, step, allowedValues, default,
//! maxSize, maskFrom, costImpact, required.{always, conditional, conditionalValues,
//! ifDefined, ifNotDefined}}`. Shapes follow the research doc; recordings confirm them.
//!
//! Reference inputs (`role: reference`) also become `slots[]` (FR-PRV-31, R31). A required
//! slot no longer blocks the model; meeting its `min` is the caller's job (FR-PRV-32).
//!
//! The overlay row's operation picks the mode (GEN OQ-GEN-01). `edit_masked` never reads
//! Scenario's `capabilities[]`. `generate` is offered when it lists `txt2img`; source and mask
//! inputs are dropped (never controls, never sent), and the row's size inputs become
//! `output_size` (`wh` from the width input's min/max/step, `aspect` from the ratio input's
//! `w:h` values, or the row's ratios whose named size the input lists) and its sketch slot
//! `structure`. Inputs the row ignores are never controls; its `drop_values` leave `allowed`.

use serde_json::{Map, Value};

use crate::scenario::overlay::OverlayRow;
use crate::{
    Capabilities, Control, ControlRole, ControlType, MaskSupport, Operation, OutputSize,
    ProviderError, ProviderResult, Rule, RuleCondition, RuleEffect, RuleOp, Slot, StructureSlot,
};

/// Is this an `a:b` ratio (`16:9`), not `auto` / `source` / `None`?
pub fn is_ratio(v: &str) -> bool {
    let mut parts = v.split(':');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && p != "0"
                && !p.starts_with('0')
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// The input names a `required.ifDefined` / `ifNotDefined` value refers to.
fn referenced_inputs(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Object(m) => m.keys().cloned().collect(),
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => vec![],
    }
}

/// A normalised model.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalised {
    pub name: String,
    pub capabilities: Capabilities,
    /// Inputs that take a list of files (the body sends an array for these, even for the
    /// source image).
    pub array_inputs: Vec<String>,
}

fn malformed(why: &str) -> ProviderError {
    ProviderError::ProviderFailure {
        hint: format!("malformed model capabilities: {why}"),
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|f| f.is_finite())
}

fn integral(f: f64) -> bool {
    f.fract() == 0.0
}

/// The role of an input: the overlay first (R28), then what Scenario says.
fn role_of(
    name: &str,
    input: &Map<String, Value>,
    ty: &str,
    row: Option<&OverlayRow>,
) -> ControlRole {
    if let Some(role) = row.and_then(|r| r.role_of(name)) {
        return role;
    }
    let kind = input.get("kind").and_then(Value::as_str).unwrap_or("");
    if input.get("maskFrom").is_some_and(|v| !v.is_null()) || kind == "mask" {
        return ControlRole::Mask;
    }
    if name == "seed" || ty == "seed" {
        return ControlRole::Seed;
    }
    if name == "prompt" {
        return ControlRole::Prompt;
    }
    if matches!(
        ty,
        "file" | "image" | "file_array" | "files" | "image_array"
    ) {
        return ControlRole::Reference;
    }
    ControlRole::Other
}

fn control_type(ty: &str, role: ControlRole, input: &Map<String, Value>) -> Option<ControlType> {
    let allowed = input
        .get("allowedValues")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    Some(match ty {
        "string" | "text" if allowed => ControlType::Enum,
        "string" | "text" => ControlType::String,
        "boolean" | "bool" => ControlType::Bool,
        "seed" => ControlType::Seed,
        "number" | "integer" | "int" | "float" if role == ControlRole::Seed => ControlType::Seed,
        "integer" | "int" => ControlType::Int,
        "float" => ControlType::Float,
        "number" => {
            let step = num(input.get("step"));
            let ends_integral = [num(input.get("min")), num(input.get("max"))]
                .iter()
                .all(|v| v.is_none_or(integral));
            match step {
                Some(s) if integral(s) && s >= 1.0 && ends_integral => ControlType::Int,
                None if role == ControlRole::CandidateCount => ControlType::Int,
                _ => ControlType::Float,
            }
        }
        "number_enum" | "enum" => ControlType::Enum,
        "file" | "image" | "file_array" | "files" | "image_array" if role == ControlRole::Mask => {
            ControlType::Mask
        }
        "file" | "image" | "file_array" | "files" | "image_array" => ControlType::Image,
        _ => return None,
    })
}

/// Conditions from `required.conditional` / `required.conditionalValues`: a map
/// `{input: value | [values]}` (real models wrap it as `{values: {input: [values]}}`) or a list
/// of `{input|name|field, value | values}`.
fn conditions(v: &Value) -> Option<Vec<(String, Value)>> {
    if let Some(inner @ Value::Object(_)) = v
        .as_object()
        .filter(|m| m.len() == 1)
        .and_then(|m| m.get("values"))
    {
        return conditions(inner);
    }
    match v {
        Value::Object(m) => Some(m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        Value::Array(items) => items
            .iter()
            .map(|it| {
                let m = it.as_object()?;
                let control = ["input", "name", "field"]
                    .iter()
                    .find_map(|k| m.get(*k).and_then(Value::as_str))?;
                let value = m.get("values").or_else(|| m.get("value"))?.clone();
                Some((control.to_owned(), value))
            })
            .collect(),
        _ => None,
    }
}

fn rule(when: &str, value: Value, then: &str) -> Rule {
    let (op, value) = match value {
        Value::Array(a) if a.len() == 1 => {
            (RuleOp::Eq, a.into_iter().next().unwrap_or(Value::Null))
        }
        Value::Array(a) => (RuleOp::In, Value::Array(a)),
        v => (RuleOp::Eq, v),
    };
    Rule {
        when: RuleCondition {
            control: when.to_owned(),
            op,
            value,
        },
        then: RuleEffect {
            control: then.to_owned(),
            required: Some(true),
            allowed: None,
            hidden: None,
        },
    }
}

/// A reference slot (FR-PRV-31): `file` → max 1; `file_array` → the input's `max` (real models
/// say `maxLength`), else the overlay's `slot_max`, else 1 with a warning. `min` is 1 if the input is required.
fn slot(
    name: &str,
    ty: &str,
    required: bool,
    max: Option<f64>,
    cost_affecting: bool,
    row: Option<&OverlayRow>,
    warnings: &mut Vec<String>,
) -> Slot {
    let array = matches!(ty, "file_array" | "files" | "image_array");
    let max = if !array {
        1
    } else if let Some(m) = max.filter(|m| *m >= 1.0) {
        m.min(f64::from(u32::MAX)) as u32
    } else if let Some(m) = row.and_then(|r| r.slot_max(name)) {
        m
    } else {
        warnings.push(format!(
            "reference input {name:?} lists no max and the overlay has none; 1 assumed"
        ));
        1
    };
    Slot {
        name: name.to_owned(),
        role: row.and_then(|r| r.slot_role(name)),
        min: u32::from(required),
        max,
        cost_affecting,
        array,
    }
}

/// Normalises a `GET /models/{id}` body (`{"model": {...}}` or the model itself).
pub fn normalise(
    model_id: &str,
    body: &Value,
    row: Option<&OverlayRow>,
) -> ProviderResult<Normalised> {
    let model = body.get("model").unwrap_or(body);
    let model = model
        .as_object()
        .ok_or_else(|| malformed("not an object"))?;
    let inputs = match model.get("inputs") {
        Some(Value::Array(a)) => a.as_slice(),
        None | Some(Value::Null) => &[],
        Some(_) => return Err(malformed("inputs is not a list")),
    };
    let name = model
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(model_id)
        .to_owned();

    let operation = row.map_or(Operation::EditMasked, |r| r.operation);
    let generate = operation == Operation::Generate;
    // Every named input's role, for conditions that refer to other inputs.
    let roles: std::collections::HashMap<&str, ControlRole> = inputs
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|i| {
            let name = i.get("name").and_then(Value::as_str)?;
            let ty = i.get("type").and_then(Value::as_str).unwrap_or("");
            Some((name, role_of(name, i, ty, row)))
        })
        .collect();
    // Source and mask: always sent together when editing, never sent when generating.
    // Ignored inputs are never sent.
    let unsent = |name: &str| {
        row.is_some_and(|r| r.ignores(name))
            || generate
                && roles
                    .get(name)
                    .is_some_and(|r| matches!(r, ControlRole::Source | ControlRole::Mask))
    };

    let mut warnings = vec![];
    let mut blocked = false;
    let mut controls: Vec<Control> = vec![];
    let mut slots: Vec<Slot> = vec![];
    let mut array_inputs: Vec<String> = vec![];
    let mut rules = vec![];
    let mut max_bytes = row.map_or(50 * 1024 * 1024, |r| r.max_bytes);
    let mut input_px = row.map_or(
        crate::InputPx {
            min: 64,
            max: 4096,
            multiple_of: 8,
        },
        |r| r.input_px,
    );

    for (i, input) in inputs.iter().enumerate() {
        let Some(input) = input.as_object() else {
            warnings.push(format!("input #{i} is not an object; ignored"));
            continue;
        };
        let Some(name) = input
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
        else {
            warnings.push(format!("input #{i} has no name; ignored"));
            continue;
        };
        // The adapter fills these itself (overlay `fixed` / `size_inputs`) or never sends them
        // (`ignored`): never a control.
        if row.is_some_and(|r| r.skips(name)) {
            continue;
        }
        let ty = input.get("type").and_then(Value::as_str).unwrap_or("");
        let array = matches!(ty, "file_array" | "files" | "image_array");
        if array {
            array_inputs.push(name.to_owned());
        }
        let role = role_of(name, input, ty, row);
        // A generate job sends no source and no mask: never controls (FR-GEN-06).
        if generate && matches!(role, ControlRole::Source | ControlRole::Mask) {
            continue;
        }
        let required_v = input.get("required");
        let always = match required_v {
            Some(Value::Bool(b)) => *b,
            Some(Value::Object(m)) => m.get("always").and_then(Value::as_bool).unwrap_or(false),
            _ => false,
        };
        let Some(kind) = control_type(ty, role, input) else {
            warnings.push(format!(
                "unknown input type {ty:?} for {name:?}; control omitted"
            ));
            if always {
                warnings.push(format!(
                    "required input {name:?} has an unknown type; model not offered"
                ));
                blocked = true;
            }
            continue;
        };
        if let Some(Value::Object(req)) = required_v {
            for key in ["conditional", "conditionalValues"] {
                if let Some(v) = req.get(key).filter(|v| !v.is_null()) {
                    match conditions(v) {
                        Some(conds) => {
                            rules.extend(conds.into_iter().map(|(c, v)| rule(&c, v, name)))
                        }
                        None => {
                            warnings.push(format!("{name:?}: unreadable required.{key}; ignored"))
                        }
                    }
                }
            }
            // The adapter always sends the source and the mask together, so "required if the
            // other is defined" on either of them always holds (real V3, Character, GPT bodies).
            let sent_together =
                !generate && matches!(role, ControlRole::Source | ControlRole::Mask);
            for key in ["ifDefined", "ifNotDefined"] {
                let Some(v) = req.get(key).filter(|v| !v.is_null()) else {
                    continue;
                };
                // "Required if the mask is defined" never applies to a generate job (GPT).
                let refs = referenced_inputs(v);
                let never = !refs.is_empty() && refs.iter().all(|r| unsent(r));
                if !sent_together && !never {
                    warnings.push(format!(
                        "{name:?}: required.{key} is not mapped; the control is optional"
                    ));
                }
            }
        }
        let is_file = matches!(kind, ControlType::Image | ControlType::Mask);
        let (min, max) = (num(input.get("min")), num(input.get("max")));
        let cost_affecting = input
            .get("costImpact")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if role == ControlRole::Reference {
            slots.push(slot(
                name,
                ty,
                always,
                max.or_else(|| num(input.get("maxLength"))),
                cost_affecting,
                row,
                &mut warnings,
            ));
        }
        if is_file {
            if let Some(b) = num(input.get("maxSize")).filter(|b| *b > 0.0) {
                max_bytes = max_bytes.min(b as u64);
            }
            // An array's min/max are counts, not pixels.
            if role == ControlRole::Source && !array {
                if let Some(m) = min.filter(|m| *m > 0.0) {
                    input_px.min = input_px.min.max(m as u32);
                }
                if let Some(m) = max.filter(|m| *m > 0.0) {
                    input_px.max = input_px.max.min(m as u32);
                }
            }
        }
        controls.push(Control {
            name: name.to_owned(),
            kind,
            role,
            // Px limits of file inputs live in `input_px`, not on the control.
            min: if is_file { None } else { min },
            max: if is_file { None } else { max },
            step: if is_file {
                None
            } else {
                num(input.get("step"))
            },
            allowed: input
                .get("allowedValues")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|v| !row.is_some_and(|r| r.drops(name, v)))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            default: input
                .get("default")
                .filter(|v| !v.is_null() && !row.is_some_and(|r| r.drops(name, v)))
                .cloned(),
            required: always,
            cost_affecting,
        });
    }

    rules.retain(|r: &Rule| {
        let filled =
            row.is_some_and(|row| row.skips(&r.when.control) || row.skips(&r.then.control));
        let known = controls.iter().any(|c| c.name == r.when.control);
        if !known && !filled {
            warnings.push(format!(
                "rule on unknown input {:?} for {:?}; ignored",
                r.when.control, r.then.control
            ));
        }
        known && !filled
    });

    let has = |role| controls.iter().any(|c| c.role == role);
    let mask = match row {
        _ if generate => None,
        None => {
            warnings.push(
                "no model overlay row (mask convention unknown); masked edit not offered".into(),
            );
            None
        }
        Some(_) if !has(ControlRole::Mask) => {
            warnings.push("the model lists no mask input; masked edit not offered".into());
            None
        }
        Some(_) if !has(ControlRole::Source) => {
            warnings.push("the model lists no source image input; masked edit not offered".into());
            None
        }
        Some(r) => Some(MaskSupport {
            convention: r.convention,
            soft: r.soft,
        }),
    };
    let mut output_size = None;
    let mut structure = None;
    let operations = if generate {
        let lists_txt2img = model
            .get("capabilities")
            .and_then(Value::as_array)
            .is_some_and(|c| c.iter().any(|x| x.as_str() == Some("txt2img")));
        if !lists_txt2img {
            warnings.push("the model doesn't list txt2img; generate not offered".into());
        }
        if let Some(r) = row {
            output_size = output_size_of(r, inputs, &mut warnings);
            structure = r.structure_slot.and_then(|s| {
                let found = slots.iter().any(|x| x.name == s);
                if !found {
                    warnings.push(format!("sketch slot {s:?} is not an input of the model"));
                }
                found.then(|| StructureSlot { slot: s.to_owned() })
            });
        } else {
            warnings.push("no generate overlay row; generate not offered".into());
        }
        if lists_txt2img && output_size.is_some() && !blocked {
            vec![Operation::Generate]
        } else {
            vec![]
        }
    } else if mask.is_some() && !blocked {
        vec![Operation::EditMasked]
    } else {
        vec![]
    };
    if input_px.min > input_px.max {
        warnings.push("the model's size limits leave no valid input size".into());
    }
    Ok(Normalised {
        name,
        capabilities: Capabilities {
            model_id: model_id.to_owned(),
            operations,
            seed: has(ControlRole::Seed),
            prompt_required: controls
                .iter()
                .any(|c| c.role == ControlRole::Prompt && c.required),
            controls,
            rules,
            mask,
            input_px,
            max_bytes,
            slots,
            warnings,
            output_size,
            structure,
        },
        array_inputs,
    })
}

/// A generate row's output size: `wh` from the width input's `min`/`max`/`step` (the row's
/// `input_px` where the model lists none), `aspect` from the ratio input's `w:h` values. A
/// row's size input the model doesn't list → a warning and no output size (not offered).
fn output_size_of(
    row: &OverlayRow,
    inputs: &[Value],
    warnings: &mut Vec<String>,
) -> Option<OutputSize> {
    let input = |name: &str| {
        inputs
            .iter()
            .filter_map(Value::as_object)
            .find(|i| i.get("name").and_then(Value::as_str) == Some(name))
    };
    if let Some((w, h)) = row.size_inputs {
        let (Some(wi), Some(_)) = (input(w), input(h)) else {
            warnings.push(format!(
                "size inputs {w:?}/{h:?} not listed; generate not offered"
            ));
            return None;
        };
        let get = |k: &str, d: u32| num(wi.get(k)).filter(|v| *v > 0.0).map_or(d, |v| v as u32);
        let px = row.input_px;
        let (min, max) = (get("min", px.min), get("max", px.max));
        let step = get("step", px.multiple_of.max(1));
        if min > max {
            warnings.push("the output size limits leave no valid size".into());
            return None;
        }
        return Some(OutputSize::Wh { min, max, step });
    }
    if let Some(a) = row.aspect_input {
        let values: Vec<&str> = input(a)
            .and_then(|i| i.get("allowedValues"))
            .and_then(Value::as_array)
            .map(|v| v.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // Named sizes (`imageSize`): the ratios whose name the model lists, in the row's order.
        let allowed: Vec<String> = if row.aspect_names.is_empty() {
            values
                .into_iter()
                .filter(|s| is_ratio(s))
                .map(str::to_owned)
                .collect()
        } else {
            row.aspect_names
                .iter()
                .filter(|(_, n)| values.contains(n))
                .map(|(r, _)| (*r).to_owned())
                .collect()
        };
        if allowed.is_empty() {
            warnings.push(format!(
                "ratio input {a:?} lists no ratio; generate not offered"
            ));
            return None;
        }
        return Some(OutputSize::Aspect { allowed });
    }
    warnings.push("the generate row names no size input; generate not offered".into());
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::overlay::{FAKE_ROWS, SCENARIO_ROWS, row_for, row_in};
    use crate::{MaskConvention, ReferenceRole, SoftRule};
    use serde_json::json;

    fn row(id: &str) -> Option<&'static OverlayRow> {
        row_in(&SCENARIO_ROWS, id)
    }

    fn control<'a>(c: &'a Capabilities, name: &str) -> &'a Control {
        c.controls.iter().find(|x| x.name == name).unwrap()
    }

    /// Every input type and role (FR-PRV-10, R28).
    #[test]
    fn fr_prv_10_every_input_type_and_role() {
        let body = json!({"model": {"id": "x", "name": "X", "inputs": [
            {"name": "prompt", "type": "string", "required": {"always": true}},
            {"name": "image", "type": "file", "kind": "image", "required": {"always": true}, "min": 300, "max": 2048, "maxSize": 20000000},
            {"name": "mask", "type": "file", "maskFrom": "image"},
            {"name": "numOutputs", "type": "number", "min": 1, "max": 4, "step": 1, "default": 2, "costImpact": true},
            {"name": "seed", "type": "number", "min": 0, "max": 2147483647},
            {"name": "strength", "type": "number", "min": 0, "max": 1, "step": 0.05, "default": 0.5},
            {"name": "steps", "type": "integer", "min": 1, "max": 50},
            {"name": "style", "type": "string", "allowedValues": ["AUTO", "REALISTIC"], "default": "AUTO"},
            {"name": "magic", "type": "boolean", "default": false},
            {"name": "refs", "type": "file_array", "kind": "image", "max": 3},
            {"name": "note", "type": "string", "futureField": {"x": 1}}
        ]}});
        let n = normalise("x", &body, row("model_ideogram-v3-balanced")).unwrap();
        let c = &n.capabilities;
        assert_eq!(n.name, "X");
        let kinds: Vec<_> = c
            .controls
            .iter()
            .map(|x| (x.name.as_str(), x.kind, x.role))
            .collect();
        use ControlRole as R;
        use ControlType as T;
        assert_eq!(
            kinds,
            [
                ("prompt", T::String, R::Prompt),
                ("image", T::Image, R::Source),
                ("mask", T::Mask, R::Mask),
                ("numOutputs", T::Int, R::CandidateCount),
                ("seed", T::Seed, R::Seed),
                ("strength", T::Float, R::Other),
                ("steps", T::Int, R::Other),
                ("style", T::Enum, R::Other),
                ("magic", T::Bool, R::Other),
                ("refs", T::Image, R::Reference),
                ("note", T::String, R::Other),
            ]
        );
        let count = control(c, "numOutputs");
        assert_eq!(
            (count.min, count.max, count.step),
            (Some(1.0), Some(4.0), Some(1.0))
        );
        assert_eq!(count.default, Some(json!(2)));
        assert!(count.cost_affecting && !control(c, "strength").cost_affecting);
        assert_eq!(
            control(c, "style").allowed,
            [json!("AUTO"), json!("REALISTIC")]
        );
        assert!(control(c, "prompt").required && c.prompt_required && c.seed);
        assert_eq!(c.input_px.min, 300);
        assert_eq!(c.input_px.max, 2048);
        assert_eq!(c.max_bytes, 20_000_000);
        assert_eq!(
            c.slots,
            [Slot {
                name: "refs".into(),
                role: None,
                min: 0,
                max: 3,
                cost_affecting: false,
                array: true,
            }]
        );
        assert_eq!(c.operations, [Operation::EditMasked]);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn fr_prv_10_both_conditional_forms_become_rules() {
        let body = json!({"inputs": [
            {"name": "image", "type": "file"}, {"name": "mask", "type": "file", "maskFrom": "image"},
            {"name": "mode", "type": "string", "allowedValues": ["edit", "inpaint", "remix"]},
            {"name": "strength", "type": "number", "required": {"conditional": [{"input": "mode", "values": ["edit", "remix"]}]}},
            {"name": "region", "type": "string", "required": {"conditionalValues": {"mode": "inpaint"}}},
            {"name": "extra", "type": "string", "required": {"ifDefined": "mode"}}
        ]});
        let c = normalise("m", &body, row("model_ideogram-v3-balanced"))
            .unwrap()
            .capabilities;
        assert_eq!(
            c.rules,
            [
                rule("mode", json!(["edit", "remix"]), "strength"),
                rule("mode", json!("inpaint"), "region"),
            ]
        );
        assert_eq!(c.rules[0].when.op, RuleOp::In);
        assert_eq!(c.rules[1].when.op, RuleOp::Eq);
        assert!(c.warnings.iter().any(|w| w.contains("ifDefined")));
    }

    #[test]
    fn fr_prv_10_unknown_type_is_omitted_and_a_required_one_blocks_the_model() {
        let base = json!([{"name": "image", "type": "file"}, {"name": "mask", "type": "file", "maskFrom": "image"}]);
        let mut inputs = base.as_array().unwrap().clone();
        inputs.push(json!({"name": "lora", "type": "model_ref"}));
        let c = normalise(
            "m",
            &json!({"inputs": inputs}),
            row("model_ideogram-v3-balanced"),
        )
        .unwrap()
        .capabilities;
        assert!(c.controls.iter().all(|x| x.name != "lora"));
        assert_eq!(c.operations, [Operation::EditMasked]);
        assert!(c.warnings.iter().any(|w| w.contains("model_ref")));

        inputs
            .push(json!({"name": "palette", "type": "colour_list", "required": {"always": true}}));
        let c = normalise(
            "m",
            &json!({"inputs": inputs}),
            row("model_ideogram-v3-balanced"),
        )
        .unwrap()
        .capabilities;
        assert!(
            c.operations.is_empty(),
            "a required unknown input → not offered"
        );
    }

    #[test]
    fn fr_prv_32_a_required_reference_slot_is_offered() {
        let body = json!({"inputs": [
            {"name": "image", "type": "file"}, {"name": "mask", "type": "file", "maskFrom": "image"},
            {"name": "character", "type": "file", "required": {"always": true}}
        ]});
        let c = normalise("m", &body, row("model_ideogram-character"))
            .unwrap()
            .capabilities;
        assert_eq!((c.slots[0].min, c.slots[0].max), (1, 1));
        assert_eq!(c.operations, [Operation::EditMasked]);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        // `evaluate` doesn't ask for a reference value: counts are checked on send.
        assert!(c.evaluate(&Default::default()).is_valid());
    }

    fn slot_of(c: &Capabilities, name: &str) -> Slot {
        c.slots.iter().find(|s| s.name == name).cloned().unwrap()
    }

    #[test]
    fn fr_prv_31_slot_min_max_role_and_cost_rules() {
        let body = json!({"inputs": [
            {"name": "image", "type": "file"}, {"name": "mask", "type": "file", "maskFrom": "image"},
            {"name": "one", "type": "file", "max": 4096},
            {"name": "listed", "type": "file_array", "max": 5, "costImpact": true},
            {"name": "styleReferenceImages", "type": "file_array"},
            {"name": "unbounded", "type": "file_array", "required": {"always": true}},
            {"name": "pick", "type": "file_array", "max": 0}
        ]});
        static ROW: std::sync::LazyLock<OverlayRow> = std::sync::LazyLock::new(|| OverlayRow {
            slot_max: &[("styleReferenceImages", 6)],
            ..*row_in(&SCENARIO_ROWS, "model_ideogram-v3-balanced").unwrap()
        });
        let c = normalise("m", &body, Some(&ROW)).unwrap().capabilities;
        let s = |n| {
            let s = slot_of(&c, n);
            (s.role, s.min, s.max, s.cost_affecting, s.array)
        };
        use ReferenceRole::Style;
        // `file` → 1, whatever its (px) max says.
        assert_eq!(s("one"), (None, 0, 1, false, false));
        // `file_array` → the input's max; cost from `costImpact`.
        assert_eq!(s("listed"), (None, 0, 5, true, true));
        // No max → the overlay's `slot_max`; the role from `slot_roles`.
        assert_eq!(s("styleReferenceImages"), (Some(Style), 0, 6, false, true));
        // No max anywhere → 1 with a warning; required → min 1.
        assert_eq!(s("unbounded"), (None, 1, 1, false, true));
        assert_eq!(s("pick"), (None, 0, 1, false, true));
        let lists_no_max: Vec<_> = c
            .warnings
            .iter()
            .filter(|w| w.contains("lists no max"))
            .collect();
        assert_eq!(lists_no_max.len(), 2, "{:?}", c.warnings);
        assert_eq!(c.operations, [Operation::EditMasked]);
        // The control stays a plain image control: its max is a count, not a size.
        assert_eq!(control(&c, "listed").max, None);
        assert_eq!(c.input_px, ROW.input_px);
    }

    /// One fixture test per overlay row (FR-PRV-31): the slots each committed `get-model`
    /// fixture gives, with the row's roles.
    #[test]
    fn fr_prv_31_slots_per_overlay_row() {
        use ReferenceRole::{Identity, Style};
        type Want = &'static [(&'static str, Option<ReferenceRole>, u32, u32, bool)];
        let style4: Want = &[("styleReferenceImages", Some(Style), 0, 4, false)];
        let table: [(&str, Want); 13] = [
            ("model_openai-gpt-image-2", &[]),
            ("model_openai-gpt-image-2-5-flare", &[]),
            ("model_openai-gpt-image-2-5-sunburst", &[]),
            // 4.5's `referenceImages` carries the source (overlay role), so it is no slot.
            ("model_ideogram-v4-5", &[]),
            (
                "model_ideogram-v4-5-precise-edit",
                &[("referenceImages", Some(Style), 0, 4, false)],
            ),
            ("model_ideogram-v3-balanced", style4),
            ("model_ideogram-v3-quality", style4),
            ("model_ideogram-v3-turbo", style4),
            // Real Character: one required `file` identity, no style references.
            (
                "model_ideogram-character",
                &[("characterReferenceImage", Some(Identity), 1, 1, false)],
            ),
            // FLUX.1 Pro Fill: `image` is the source; no reference input.
            ("model_bfl-flux-1-pro-fill", &[]),
            (
                "fake-white-edit",
                &[("style_refs", Some(Style), 0, 3, false)],
            ),
            (
                "fake-black-edit",
                &[
                    ("style_refs", Some(Style), 0, 2, true),
                    ("identity_ref", Some(Identity), 0, 1, false),
                ],
            ),
            ("fake-alpha-edit", &[]),
        ];
        let rows: Vec<_> = SCENARIO_ROWS
            .iter()
            .chain(FAKE_ROWS.iter())
            .filter(|r| r.operation == Operation::EditMasked)
            .collect();
        assert_eq!(rows.len(), table.len(), "every overlay row is in the table");
        for (model, want) in table {
            let row = rows.iter().find(|r| r.model_id == model).unwrap();
            let rec = crate::scenario::fixture_recording(model, "get-model").unwrap();
            let c = normalise(model, &rec.exchanges[0].response.body, Some(row))
                .unwrap()
                .capabilities;
            let got: Vec<_> = c
                .slots
                .iter()
                .map(|s| (s.name.as_str(), s.role, s.min, s.max, s.cost_affecting))
                .collect();
            assert_eq!(got, want, "{model}");
        }
    }

    #[test]
    fn fr_prv_10_malformed_json_is_an_error_never_a_panic() {
        for bad in [
            json!(null),
            json!([1, 2]),
            json!({"inputs": "x"}),
            json!("text"),
        ] {
            assert_eq!(
                normalise("m", &bad, None).unwrap_err().code(),
                "provider_failure"
            );
        }
        // Garbage entries inside a valid list are warnings.
        let c = normalise("m", &json!({"inputs": [1, {"type": "string"}, null]}), None)
            .unwrap()
            .capabilities;
        assert_eq!(c.warnings.len(), 4);
        assert!(c.controls.is_empty());
    }

    #[test]
    fn fr_prv_11_masked_edit_needs_an_overlay_row_and_a_mask_input() {
        let with_mask = json!({"inputs": [{"name": "image", "type": "file"}, {"name": "mask", "type": "file", "maskFrom": "image"}]});
        let no_mask = json!({"inputs": [{"name": "image", "type": "file"}, {"name": "prompt", "type": "string"}]});
        assert!(
            normalise("m", &with_mask, None)
                .unwrap()
                .capabilities
                .operations
                .is_empty()
        );
        assert!(
            normalise("m", &no_mask, row("model_ideogram-v3-balanced"))
                .unwrap()
                .capabilities
                .operations
                .is_empty()
        );
        let ok = normalise("m", &with_mask, row("model_ideogram-v3-balanced"))
            .unwrap()
            .capabilities;
        assert_eq!(
            ok.mask,
            Some(MaskSupport {
                convention: MaskConvention::BlackEdit,
                soft: SoftRule::Threshold(128)
            })
        );
    }

    /// Each overlay row has a fixture test (FR-PRV-11): its committed `get-model` fixture,
    /// through the normaliser, offers masked edit in the row's convention.
    #[test]
    fn fr_prv_11_every_overlay_row_against_its_fixture() {
        for row in SCENARIO_ROWS
            .iter()
            .chain(FAKE_ROWS.iter())
            .filter(|r| r.operation == Operation::EditMasked)
        {
            let rec = crate::scenario::fixture_recording(row.model_id, "get-model")
                .unwrap_or_else(|| panic!("no get-model fixture for {}", row.model_id));
            let body = &rec.exchanges[0].response.body;
            let c = normalise(row.model_id, body, Some(row))
                .unwrap()
                .capabilities;
            assert_eq!(
                c.operations,
                [Operation::EditMasked],
                "{}: {:?}",
                row.model_id,
                c.warnings
            );
            assert_eq!(
                c.mask.map(|m| (m.convention, m.soft)),
                Some((row.convention, row.soft)),
                "{}",
                row.model_id
            );
            // Role names the model lists get their role; a source and a mask are always there.
            for (name, role) in row.roles {
                if let Some(x) = c.controls.iter().find(|x| x.name == *name) {
                    assert_eq!(x.role, *role, "{}: {name}", row.model_id);
                }
            }
            for role in [ControlRole::Source, ControlRole::Mask] {
                assert!(
                    c.controls.iter().any(|x| x.role == role),
                    "{}",
                    row.model_id
                );
            }
            // Inputs the adapter fills are never controls.
            assert!(
                c.controls.iter().all(|x| !row.fills(&x.name)),
                "{}",
                row.model_id
            );
            assert!(c.warnings.is_empty(), "{}: {:?}", row.model_id, c.warnings);
        }
    }

    /// The real recordings (2026-11-08, 2026-11-11) list `txt2img`/`img2img` and never
    /// `inpaint`: masked edit rests on a mask input alone (`maskFrom`, or the overlay's mask
    /// role where the model marks none: FLUX.1 Pro Fill). Without it a real model is not
    /// offered, and GPT's mask says `backgroundBehavior: transparent`, matching the alpha
    /// convention.
    #[test]
    fn fr_prv_11_real_models_offer_masked_edit_only_with_a_mask_input() {
        for row in SCENARIO_ROWS
            .iter()
            .filter(|r| r.operation == Operation::EditMasked)
        {
            let rec = crate::scenario::fixture_recording(row.model_id, "get-model").unwrap();
            let body = &rec.exchanges[0].response.body;
            let inputs = body["model"]["inputs"].as_array().unwrap();
            let is_mask = |i: &Value| {
                i.get("maskFrom").is_some()
                    || i["name"]
                        .as_str()
                        .is_some_and(|n| row.role_of(n) == Some(ControlRole::Mask))
            };
            let mask = inputs.iter().find(|i| is_mask(i));
            let mask = mask.unwrap_or_else(|| panic!("{}: no mask input", row.model_id));
            assert_eq!(
                mask["backgroundBehavior"] == "transparent",
                row.convention == MaskConvention::AlphaTransparentEdit,
                "{}",
                row.model_id
            );
            let mut stripped = body.clone();
            stripped["model"]["inputs"]
                .as_array_mut()
                .unwrap()
                .retain(|i| !is_mask(i));
            stripped["model"]["capabilities"] = json!(["txt2img", "img2img", "inpaint"]);
            let c = normalise(row.model_id, &stripped, Some(row))
                .unwrap()
                .capabilities;
            assert!(c.operations.is_empty(), "{}", row.model_id);
            assert!(c.mask.is_none(), "{}", row.model_id);
        }
    }

    /// Owner decision 2026-11-08: the real single-output models list no `numOutputs`, so their
    /// capabilities have no count control (RGN shows "1 result per job") and the recorded
    /// dry-run (the estimate path) asks for no count; the multi-output ones keep theirs.
    #[test]
    fn owner_2026_11_08_single_output_models_have_no_count_control() {
        let single = [
            "model_ideogram-v3-balanced",
            "model_ideogram-v3-quality",
            "model_ideogram-v3-turbo",
            "model_ideogram-character",
            "model_bfl-flux-1-pro-fill",
        ];
        for row in SCENARIO_ROWS
            .iter()
            .filter(|r| r.operation == Operation::EditMasked)
        {
            let rec = crate::scenario::fixture_recording(row.model_id, "get-model").unwrap();
            let c = normalise(row.model_id, &rec.exchanges[0].response.body, Some(row))
                .unwrap()
                .capabilities;
            let has_count = c
                .controls
                .iter()
                .any(|x| x.role == ControlRole::CandidateCount);
            assert_eq!(
                has_count,
                !single.contains(&row.model_id),
                "{}",
                row.model_id
            );
        }
        for model in single {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/provider")
                .join(model)
                .join("dry-run.json");
            let rec =
                crate::replay::Recording::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
            let generate = rec
                .exchanges
                .iter()
                .find(|e| e.request.path.starts_with("/generate/"))
                .unwrap_or_else(|| panic!("{model}: no generate call"));
            let body = generate.request.body.as_ref().unwrap();
            assert!(body.get("numOutputs").is_none(), "{model}: {body}");
        }
    }

    /// FLUX.1 Pro Fill's `mask` has no `maskFrom`: the overlay's mask role is enough to offer
    /// masked edit (FR-PRV-11); without the row the same body is not offered.
    #[test]
    fn fr_prv_11_an_overlay_mask_role_needs_no_mask_from() {
        let id = "model_bfl-flux-1-pro-fill";
        let rec = crate::scenario::fixture_recording(id, "get-model").unwrap();
        let body = &rec.exchanges[0].response.body;
        assert!(
            body["model"]["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i.get("maskFrom").is_none())
        );
        let c = normalise(id, body, row(id)).unwrap().capabilities;
        assert_eq!(c.operations, [Operation::EditMasked], "{:?}", c.warnings);
        assert_eq!(
            c.mask,
            Some(MaskSupport {
                convention: MaskConvention::WhiteEdit,
                soft: SoftRule::Threshold(128)
            })
        );
        assert_eq!(c.max_bytes, 10 * 1024 * 1024);
        assert!(
            normalise(id, body, None)
                .unwrap()
                .capabilities
                .operations
                .is_empty()
        );
    }

    /// A named-size ratio input (FR-PRV-37), an ignored input and a dropped value (FR-PRV-10)
    /// over a synthetic body.
    #[test]
    fn fr_prv_37_named_sizes_ignored_inputs_and_dropped_values() {
        use crate::scenario::overlay::NAMED_SIZES;
        let body = json!({"capabilities": ["txt2img"], "inputs": [
            {"name": "prompt", "type": "string", "required": {"always": true}},
            {"name": "imageSize", "type": "string", "allowedValues": ["auto", "square", "square_hd", "portrait_16_9", "landscape_4_3"], "default": "auto"},
            {"name": "resolution", "type": "string", "allowedValues": ["match_input_image", "1 MP", "2 MP"], "default": "match_input_image"},
            {"name": "palette", "type": "inputs_array", "required": {"always": true}},
            {"name": "tint", "type": "number", "required": {"conditional": {"palette": ["x"]}}}
        ]});
        static ROW: std::sync::LazyLock<OverlayRow> = std::sync::LazyLock::new(|| OverlayRow {
            aspect_input: Some("imageSize"),
            aspect_names: NAMED_SIZES,
            size_inputs: None,
            ignored: &["palette"],
            drop_values: &[("resolution", "match_input_image")],
            ..*row_for(&FAKE_ROWS, "fake-gen-aspect", Operation::Generate).unwrap()
        });
        let c = normalise("m", &body, Some(&ROW)).unwrap().capabilities;
        // Only listed names, in the row's order; `auto` and `square` are not offered.
        assert_eq!(
            c.output_size,
            Some(OutputSize::Aspect {
                allowed: vec!["1:1".into(), "4:3".into(), "9:16".into()]
            })
        );
        assert_eq!(ROW.aspect_value("9:16"), Some("portrait_16_9"));
        assert_eq!(ROW.aspect_value("2:3"), None);
        // A required unknown-type input that the row ignores doesn't block the model, and
        // rules on it are dropped silently.
        assert_eq!(c.operations, [Operation::Generate], "{:?}", c.warnings);
        assert!(
            c.controls
                .iter()
                .all(|x| x.name != "palette" && x.name != "imageSize")
        );
        assert!(c.rules.is_empty());
        let res = control(&c, "resolution");
        assert_eq!(res.allowed, [json!("1 MP"), json!("2 MP")]);
        assert_eq!(res.default, None, "a dropped default is no default");
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn normalising_is_fast() {
        let rec =
            crate::scenario::fixture_recording("model_ideogram-v3-balanced", "get-model").unwrap();
        let body = &rec.exchanges[0].response.body;
        let t = std::time::Instant::now();
        for _ in 0..100 {
            normalise(
                "model_ideogram-v3-balanced",
                body,
                row("model_ideogram-v3-balanced"),
            )
            .unwrap();
        }
        assert!(t.elapsed().as_millis() < 1000, "< 10 ms each (spec §12)");
    }
}
