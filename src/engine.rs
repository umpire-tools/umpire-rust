use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::error::{Result, UmpireError};
use crate::evaluator::{evaluate, AvailabilityMap};
use crate::field::FieldDef;
use crate::graph::build_graph;
use crate::rule::Rule;
use crate::schema::{Schema, ValidatorEntry};
use crate::values::json_equal;

/// One foul recommended by [`Umpire::play`].
#[derive(Clone, Debug, PartialEq)]
pub struct Foul {
    pub field: String,
    pub reason: String,
    pub suggested_value: Option<Value>,
}

/// Pairs a field name with its definition, preserving schema order.
#[derive(Clone, Debug)]
pub struct FieldEntry {
    pub name: String,
    pub def: FieldDef,
}

/// The evaluation engine: fields plus rules plus validators, mirroring the
/// TS core's createUmpire surface.
#[derive(Debug)]
pub struct Umpire {
    fields: IndexMap<String, FieldDef>,
    rules: Vec<Rule>,
    validators: IndexMap<String, ValidatorEntry>,
    topo_order: Vec<String>,
}

impl Umpire {
    /// Builds an engine from field entries (in declaration order), rules, and
    /// validators. It rejects unknown field references and dependency cycles.
    pub fn new(
        fields: Vec<FieldEntry>,
        rules: Vec<Rule>,
        validators: IndexMap<String, ValidatorEntry>,
    ) -> Result<Umpire> {
        let mut field_map: IndexMap<String, FieldDef> = IndexMap::new();
        for entry in fields {
            if field_map.contains_key(&entry.name) {
                return Err(UmpireError::new(format!(
                    "[@umpire/core] Field {:?} is defined more than once",
                    entry.name
                )));
            }
            field_map.insert(entry.name, entry.def);
        }

        let field_names: std::collections::HashSet<&String> = field_map.keys().collect();

        for rule in &rules {
            for target in &rule.targets {
                if !field_names.contains(target) {
                    return Err(UmpireError::new(format!(
                        "[@umpire/core] Unknown field {:?} referenced by {} rule",
                        target,
                        rule.kind.as_str()
                    )));
                }
            }
            let (ordering, _) = rule.graph_sources();
            for source in &ordering {
                if !field_names.contains(source) {
                    return Err(UmpireError::new(format!(
                        "[@umpire/core] Unknown field {:?} referenced by {} rule",
                        source,
                        rule.kind.as_str()
                    )));
                }
            }
        }
        for field in validators.keys() {
            if !field_names.contains(field) {
                return Err(UmpireError::new(format!(
                    "[@umpire/core] Unknown field {:?} referenced by validators",
                    field
                )));
            }
        }

        let graph = build_graph(&field_map.keys().cloned().collect::<Vec<String>>(), &rules);
        graph.detect_cycles()?;
        let topo_order = graph.topological_sort()?;

        Ok(Umpire {
            fields: field_map,
            rules,
            validators,
            topo_order,
        })
    }

    /// Builds an engine from a hydrated schema.
    pub fn from_schema(schema: Schema) -> Result<Umpire> {
        let entries: Vec<FieldEntry> = schema
            .field_order
            .iter()
            .map(|name| FieldEntry {
                name: name.clone(),
                def: schema.fields.get(name).cloned().unwrap_or_default(),
            })
            .collect();
        Umpire::new(entries, schema.rules, schema.validators)
    }

    /// Evaluates every rule and returns per-field availability. `prev`, when
    /// supplied, assists oneOf branch resolution.
    pub fn check(
        &self,
        values: &Map<String, Value>,
        conditions: &Map<String, Value>,
        prev: Option<&Map<String, Value>>,
    ) -> Result<AvailabilityMap> {
        let mut availability = evaluate(
            &self.fields,
            &self.rules,
            &self.topo_order,
            values,
            conditions,
            prev,
        )?;

        for (field, entry) in &self.validators {
            let Some(status) = availability.get_mut(field) else {
                continue;
            };
            if !status.enabled || !status.satisfied {
                continue;
            }
            let value = values.get(field).unwrap_or(&Value::Null);
            let valid = entry.op.validate(value);
            status.valid = Some(valid);
            if !valid && entry.has_error {
                status.error = entry.error.clone();
            }
        }

        Ok(availability)
    }

    /// Recommends fouls for a before → after availability transition.
    pub fn play(
        &self,
        before: &AvailabilityMap,
        after: &AvailabilityMap,
        values: &Map<String, Value>,
    ) -> Vec<Foul> {
        let mut fouls = Vec::new();
        for field in self.fields.keys() {
            let (Some(before_status), Some(after_status)) = (before.get(field), after.get(field))
            else {
                continue;
            };

            let disabled_transition = before_status.enabled && !after_status.enabled;
            let foul_transition = before_status.fair && !after_status.fair;
            if !disabled_transition && !foul_transition {
                continue;
            }
            if !after_status.satisfied {
                continue;
            }

            let current = values.get(field).unwrap_or(&Value::Null);
            let suggested = self.fields.get(field).and_then(|def| def.default.clone());
            if json_equal(current, suggested.as_ref().unwrap_or(&Value::Null)) {
                continue;
            }

            let mut reason = after_status.reason.clone();
            if reason.is_empty() {
                reason = if disabled_transition {
                    "field disabled".to_string()
                } else {
                    "field fouled".to_string()
                };
            }
            fouls.push(Foul {
                field: field.clone(),
                reason,
                suggested_value: suggested,
            });
        }
        fouls
    }

    /// Produces starting values from field defaults, with overrides applied.
    pub fn init(&self, overrides: &Map<String, Value>) -> Map<String, Value> {
        let mut values = Map::new();
        for field in self.fields.keys() {
            let value = overrides.get(field).cloned().unwrap_or_else(|| {
                self.fields
                    .get(field)
                    .and_then(|def| def.default.clone())
                    .unwrap_or(Value::Null)
            });
            values.insert(field.clone(), value);
        }
        values
    }

    /// The evaluation order of fields.
    pub fn topo_order(&self) -> Vec<String> {
        self.topo_order.clone()
    }
}
