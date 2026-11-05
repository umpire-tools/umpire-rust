use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::Value;

use crate::checkops::parse_check_op;
use crate::error::{schema_err, Result};
use crate::expr::compile_expr;
use crate::field::{FieldDef, IsEmptyStrategy};
use crate::rule::{Dep, Rule};
use crate::schema_validate::validate_schema_object;

/// Declares a runtime condition's type.
#[derive(Clone, Debug, Default)]
pub struct ConditionDef {
    pub type_: String,
    pub description: String,
}

/// A field-bound validator spec.
#[derive(Clone, Debug)]
pub struct ValidatorEntry {
    pub op: crate::checkops::CheckOp,
    pub error: String,
    /// Distinguishes an absent error message from an empty one.
    pub has_error: bool,
}

/// A hydrated Umpire JSON schema.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    pub field_order: Vec<String>,
    pub fields: IndexMap<String, FieldDef>,
    pub rules: Vec<Rule>,
    pub validators: IndexMap<String, ValidatorEntry>,
    pub conditions: HashMap<String, ConditionDef>,
    pub excluded: Vec<Value>,
    pub warnings: Vec<String>,
}

/// Validates and hydrates a schema from JSON text.
pub fn parse_schema(data: &[u8]) -> Result<Schema> {
    let raw: Value =
        serde_json::from_slice(data).map_err(|err| schema_err(format!("Invalid JSON: {}", err)))?;
    schema_from_raw(&raw)
}

/// Hydrates from an already-decoded JSON value.
pub fn schema_from_object(raw: &Value) -> Result<Schema> {
    schema_from_raw(raw)
}

fn schema_from_raw(raw: &Value) -> Result<Schema> {
    let Some(root) = raw.as_object() else {
        return Err(schema_err("[@umpire/json] Schema must be an object"));
    };
    validate_schema_object(root)?;
    hydrate_schema(root)
}

// hydrateSchema builds a Schema from an already-validated root object.
fn hydrate_schema(root: &serde_json::Map<String, Value>) -> Result<Schema> {
    let empty = serde_json::Map::new();
    let fields_obj = root
        .get("fields")
        .and_then(Value::as_object)
        .unwrap_or(&empty);

    let mut conditions: HashMap<String, ConditionDef> = HashMap::new();
    if let Some(raw_conditions) = root.get("conditions") {
        let conditions_obj = raw_conditions.as_object().unwrap_or(&empty);
        for (name, def_value) in conditions_obj {
            let def_obj = def_value.as_object().unwrap_or(&empty);
            let type_ = def_obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut def = ConditionDef {
                type_: type_.to_string(),
                description: String::new(),
            };
            if let Some(desc) = def_obj.get("description").and_then(Value::as_str) {
                if !desc.is_empty() {
                    def.description = desc.to_string();
                }
            }
            conditions.insert(name.clone(), def);
        }
    }

    let mut field_order: Vec<String> = Vec::new();
    let mut fields: IndexMap<String, FieldDef> = IndexMap::new();
    for (name, def_value) in fields_obj {
        field_order.push(name.clone());
        let def_obj = def_value.as_object().unwrap_or(&empty);
        let mut def = FieldDef::default();
        if def_obj
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            def.required = true;
        }
        if let Some(default_value) = def_obj.get("default") {
            def.default = Some(default_value.clone());
        }
        if let Some(strategy) = def_obj.get("isEmpty").and_then(Value::as_str) {
            if !strategy.is_empty() {
                def.is_empty = IsEmptyStrategy::from_name(strategy).unwrap_or_default();
            }
        }
        fields.insert(name.clone(), def);
    }

    let mut warnings: Vec<String> = Vec::new();
    let mut excluded: Vec<Value> = Vec::new();
    if let Some(raw_excluded) = root.get("excluded").and_then(Value::as_array) {
        for entry in raw_excluded {
            let entry_obj = entry.as_object().unwrap_or(&empty);
            let rule_type = entry_obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let description = entry_obj
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            warnings.push(format!("Excluded rule {}: {}", rule_type, description));
            excluded.push(entry.clone());
        }
    }

    let mut rules: Vec<Rule> = Vec::new();
    if let Some(raw_rules) = root.get("rules").and_then(Value::as_array) {
        for raw_rule in raw_rules {
            let rule_obj = raw_rule.as_object().unwrap_or(&empty);
            let rule = build_rule(rule_obj, fields_obj, &conditions)?;
            rules.push(rule);
        }
    }

    let mut validators: IndexMap<String, ValidatorEntry> = IndexMap::new();
    if let Some(raw_validators) = root.get("validators").and_then(Value::as_object) {
        for (field, spec) in raw_validators {
            let op = parse_check_op(spec)?;
            let mut entry = ValidatorEntry {
                op,
                error: String::new(),
                has_error: false,
            };
            if let Some(err_value) = spec.get("error") {
                entry.error = err_value.as_str().unwrap_or_default().to_string();
                entry.has_error = true;
            }
            validators.insert(field.clone(), entry);
        }
    }

    Ok(Schema {
        field_order,
        fields,
        rules,
        validators,
        conditions,
        excluded,
        warnings,
    })
}

// buildRule builds a Rule from a validated rule object. field_names comes
// from the schema's fields object.
fn build_rule(
    rule_obj: &serde_json::Map<String, Value>,
    fields_obj: &serde_json::Map<String, Value>,
    conditions: &HashMap<String, ConditionDef>,
) -> Result<Rule> {
    let empty = serde_json::Map::new();
    let field_names: std::collections::HashSet<String> = fields_obj.keys().cloned().collect();
    let rule_type = rule_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let reason = rule_obj
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);

    let field = || {
        rule_obj
            .get("field")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    match rule_type {
        "requires" => {
            let field = field();
            if rule_obj.contains_key("dependency") {
                let dependency = rule_obj
                    .get("dependency")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                return Ok(Rule::requires_dependency(
                    field,
                    dependency,
                    reason_or_empty(&reason),
                ));
            }
            if rule_obj.contains_key("dependencies") {
                let deps_raw = rule_obj.get("dependencies").and_then(Value::as_array);
                let mut parsed: Vec<Dep> = Vec::new();
                let mut sources: Vec<String> = Vec::new();
                for d in deps_raw.map(|v| v.as_slice()).unwrap_or(&[]) {
                    match d.as_str() {
                        Some(dep_name) => {
                            parsed.push(Dep::Field(dep_name.to_string()));
                            sources.push(dep_name.to_string());
                        }
                        None => {
                            let expr = compile_expr(d, &field_names, conditions)?;
                            if !expr.check_field.is_empty() {
                                sources.push(expr.check_field.clone());
                            }
                            parsed.push(Dep::Expr(expr));
                        }
                    }
                }
                let mut rule = Rule::requires_predicate_owned(field, reason);
                rule.deps = parsed;
                rule.sources = sources;
                return Ok(rule);
            }
            let expr = compile_expr(
                rule_obj.get("when").unwrap_or(&Value::Null),
                &field_names,
                conditions,
            )?;
            let sources: Vec<String> = if expr.check_field.is_empty() {
                Vec::new()
            } else {
                vec![expr.check_field.clone()]
            };
            let mut rule = Rule::requires_predicate_owned(field, reason);
            rule.deps = vec![Dep::Expr(expr)];
            rule.sources = sources;
            Ok(rule)
        }
        "enabledWhen" => {
            let field = field();
            let expr = compile_expr(
                rule_obj.get("when").unwrap_or(&Value::Null),
                &field_names,
                conditions,
            )?;
            let mut rule = Rule::enabled_when(field, crate::expr::Pred::And(Vec::new()), "");
            rule.when = Some(expr);
            rule.reason = reason;
            Ok(rule)
        }
        "fairWhen" => {
            let field = field();
            let expr = compile_expr(
                rule_obj.get("when").unwrap_or(&Value::Null),
                &field_names,
                conditions,
            )?;
            let mut rule = Rule::fair_when(field, crate::expr::Pred::And(Vec::new()), "");
            rule.fair = Some(crate::rule::FairTest::Expr(expr));
            rule.reason = reason;
            Ok(rule)
        }
        "disables" => {
            let targets: Vec<String> = rule_obj
                .get("targets")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .map(|t| t.as_str().unwrap_or_default().to_string())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(source) = rule_obj.get("source").and_then(Value::as_str) {
                return Ok(Rule::disables_field(
                    source,
                    targets,
                    reason_or_empty(&reason),
                ));
            }
            let expr = compile_expr(
                rule_obj.get("when").unwrap_or(&Value::Null),
                &field_names,
                conditions,
            )?;
            let sources: Vec<String> = if expr.check_field.is_empty() {
                Vec::new()
            } else {
                vec![expr.check_field.clone()]
            };
            let mut rule = Rule::disables_when(
                crate::expr::Pred::And(Vec::new()),
                targets,
                reason_or_empty(&reason),
            );
            rule.when = Some(expr);
            rule.sources = sources;
            rule.reason = reason;
            Ok(rule)
        }
        "oneOf" => {
            let group = rule_obj
                .get("group")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let branches_raw = rule_obj.get("branches").and_then(Value::as_object);
            let mut branches: Vec<(String, Vec<String>)> = Vec::new();
            for (branch_name, raw_fields) in branches_raw.into_iter().flatten() {
                let branch_fields = raw_fields
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|f| f.as_str().unwrap_or_default().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                branches.push((branch_name.clone(), branch_fields));
            }
            Ok(Rule::one_of(group, branches, reason_or_empty(&reason)))
        }
        "anyOf" => {
            let rules_raw = rule_obj.get("rules").and_then(Value::as_array);
            let mut inner: Vec<Rule> = Vec::new();
            for raw in rules_raw.map(|v| v.as_slice()).unwrap_or(&[]) {
                let inner_obj = raw.as_object().unwrap_or(&empty);
                inner.push(build_rule(inner_obj, fields_obj, conditions)?);
            }
            Ok(Rule::any_of(inner))
        }
        "eitherOf" => {
            let group = rule_obj
                .get("group")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let branches_raw = rule_obj.get("branches").and_then(Value::as_object);
            let mut branches: Vec<(String, Vec<Rule>)> = Vec::new();
            for (branch_name, raw_rules) in branches_raw.into_iter().flatten() {
                let mut branch_rules: Vec<Rule> = Vec::new();
                for raw in raw_rules.as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                    let inner_obj = raw.as_object().unwrap_or(&empty);
                    branch_rules.push(build_rule(inner_obj, fields_obj, conditions)?);
                }
                branches.push((branch_name.clone(), branch_rules));
            }
            Ok(Rule::either_of(group, branches))
        }
        "check" => {
            let field = field();
            let op = parse_check_op(&Value::Object(rule_obj.clone()))?;
            Ok(Rule::check(field, op, reason_or_empty(&reason)))
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown rule type {:?}",
            rule_obj.get("type").unwrap_or(&Value::Null)
        ))),
    }
}

fn reason_or_empty(reason: &Option<String>) -> &str {
    reason.as_deref().unwrap_or("")
}

impl Rule {
    // Internal constructor for schema-built requires rules: the deps and
    // sources are patched in by build_rule afterwards.
    fn requires_predicate_owned(field: String, reason: Option<String>) -> Rule {
        let mut rule = Rule::requires_predicate(field, crate::expr::Pred::And(Vec::new()), "");
        rule.reason = reason;
        rule
    }
}
