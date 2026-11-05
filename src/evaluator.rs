use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::error::Result;
use crate::field::{is_satisfied, FieldDef};
pub use crate::rule::RuleEvaluation;
use crate::rule::{evaluate_rule, Constraint, Rule};

/// Carries one Check evaluation's shared read-only state.
pub(crate) struct Ctx<'a> {
    pub fields: &'a IndexMap<String, FieldDef>,
    pub values: &'a Map<String, Value>,
    pub conditions: &'a Map<String, Value>,
    pub prev: Option<&'a Map<String, Value>>,
}

/// Carries the evaluation's mutable state: the availability output and the
/// oneOf cache.
pub(crate) struct State {
    pub availability: IndexMap<String, FieldAvailability>,
    pub cache: HashMap<usize, HashMap<String, RuleEvaluation>>,
}

/// Per-field availability as reported by [`Umpire::check`].
#[derive(Clone, Debug, PartialEq)]
pub struct FieldAvailability {
    pub enabled: bool,
    pub satisfied: bool,
    pub fair: bool,
    pub required: bool,
    /// The first failure reason, or "" when available.
    pub reason: String,
    /// Every failure reason in rule order.
    pub reasons: Vec<String>,
    pub valid: Option<bool>,
    pub error: String,
}

/// The availability of every field, keyed by field name.
pub type AvailabilityMap = IndexMap<String, FieldAvailability>;

// evaluate runs all rules over the fields in topological order, mirroring
// the TS evaluator: gate rules first, fair rules only for still-enabled
// fields, first-failure reason plus full reason collection.
pub(crate) fn evaluate(
    fields: &IndexMap<String, FieldDef>,
    rules: &[Rule],
    topo_order: &[String],
    values: &Map<String, Value>,
    conditions: &Map<String, Value>,
    prev: Option<&Map<String, Value>>,
) -> Result<AvailabilityMap> {
    let ctx = Ctx {
        fields,
        values,
        conditions,
        prev,
    };
    let mut state = State {
        availability: IndexMap::new(),
        cache: HashMap::new(),
    };

    // Partition rules by phase, then index them by target.
    let mut rules_by_target_gate: HashMap<&str, Vec<&Rule>> = HashMap::new();
    let mut rules_by_target_fair: HashMap<&str, Vec<&Rule>> = HashMap::new();
    for rule in rules {
        let index = if rule.constraint == Constraint::Fair {
            &mut rules_by_target_fair
        } else {
            &mut rules_by_target_gate
        };
        for target in &rule.targets {
            index.entry(target.as_str()).or_default().push(rule);
        }
    }

    for field in topo_order {
        let mut reasons: Vec<String> = Vec::new();
        let mut enabled = true;
        let mut fair = true;
        let mut reason = String::new();

        if let Some(gate_rules) = rules_by_target_gate.get(field.as_str()) {
            for rule in gate_rules {
                let result = evaluate_rule(rule, field, &ctx, &mut state)?;
                if result.enabled {
                    continue;
                }
                enabled = false;
                if reason.is_empty() {
                    reason = result.reason.clone();
                }
                append_composite_failure_reasons(&result, &mut reasons);
            }
        }

        if enabled {
            if let Some(fair_rules) = rules_by_target_fair.get(field.as_str()) {
                for rule in fair_rules {
                    let result = evaluate_rule(rule, field, &ctx, &mut state)?;
                    if result.fair.is_none_or(|f| f) {
                        continue;
                    }
                    fair = false;
                    if reason.is_empty() {
                        reason = result.reason.clone();
                    }
                    append_composite_failure_reasons(&result, &mut reasons);
                }
            }
        }

        let def = fields.get(field).cloned().unwrap_or_default();
        state.availability.insert(
            field.clone(),
            FieldAvailability {
                enabled,
                satisfied: is_satisfied(values.get(field).unwrap_or(&Value::Null), &def),
                fair,
                required: enabled && def.required,
                reason,
                reasons,
                valid: None,
                error: String::new(),
            },
        );
    }

    Ok(state.availability)
}

fn append_composite_failure_reasons(result: &RuleEvaluation, reasons: &mut Vec<String>) {
    if !result.reasons.is_empty() {
        reasons.extend(result.reasons.iter().cloned());
        return;
    }
    if !result.reason.is_empty() {
        reasons.push(result.reason.clone());
    }
}
