use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::checkops::regex_error_detail;
use crate::error::{schema_err, Result};
use crate::expr::compile_expr;
use crate::field::IsEmptyStrategy;
use crate::schema::ConditionDef;

const RULE_MEMBERS_CONTEXT: &str = "Rule";

fn assert_known_members(o: &Map<String, Value>, allowed: &[&str], context: &str) -> Result<()> {
    let allowed_set: HashSet<&str> = allowed.iter().copied().collect();
    for key in o.keys() {
        if !allowed_set.contains(key.as_str()) {
            return Err(schema_err(format!(
                "[@umpire/json] {} has unknown member {:?}",
                context, key
            )));
        }
    }
    Ok(())
}

fn assert_string(v: &Value, context: &str) -> Result<()> {
    if !v.is_string() {
        return Err(schema_err(format!(
            "[@umpire/json] {} must be a string",
            context
        )));
    }
    Ok(())
}

fn assert_number(v: &Value, context: &str) -> Result<()> {
    match v.as_f64() {
        Some(n) if n.is_finite() => Ok(()),
        _ => Err(schema_err(format!(
            "[@umpire/json] {} must be a finite number",
            context
        ))),
    }
}

fn assert_primitive(v: &Value, context: &str) -> Result<()> {
    if !is_json_primitive(v) {
        return Err(schema_err(format!(
            "[@umpire/json] {} must be a JSON primitive",
            context
        )));
    }
    Ok(())
}

fn assert_primitive_array(v: &Value, context: &str) -> Result<()> {
    let Some(items) = v.as_array() else {
        return Err(schema_err(format!(
            "[@umpire/json] {} must be an array of JSON primitives",
            context
        )));
    };
    for item in items {
        if !is_json_primitive(item) {
            return Err(schema_err(format!(
                "[@umpire/json] {} must be an array of JSON primitives",
                context
            )));
        }
    }
    Ok(())
}

fn assert_string_array(v: &Value, context: &str) -> Result<()> {
    let Some(items) = v.as_array() else {
        return Err(schema_err(format!(
            "[@umpire/json] {} must be an array of strings",
            context
        )));
    };
    for item in items {
        if !item.is_string() {
            return Err(schema_err(format!(
                "[@umpire/json] {} must be an array of strings",
                context
            )));
        }
    }
    Ok(())
}

fn is_json_primitive(v: &Value) -> bool {
    matches!(
        v,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn condition_type_supported(type_: &str) -> bool {
    matches!(
        type_,
        "boolean" | "string" | "number" | "string[]" | "number[]"
    )
}

fn is_empty_strategy_supported(name: &str) -> bool {
    IsEmptyStrategy::from_name(name).is_some()
}

fn validate_condition(name: &str, def: &Value) -> Result<()> {
    let Some(o) = def.as_object() else {
        return Err(schema_err(format!(
            "[@umpire/json] condition {:?} definition must be an object",
            name
        )));
    };
    assert_known_members(o, &["type", "description"], "condition definition")?;
    let type_ = o.get("type").and_then(Value::as_str).unwrap_or_default();
    if !condition_type_supported(type_) {
        return Err(schema_err(format!(
            "[@umpire/json] condition {:?} must use a supported type",
            name
        )));
    }
    if let Some(desc) = o.get("description") {
        if !desc.is_string() {
            return Err(schema_err(
                "[@umpire/json] condition %q description must be a string when provided"
                    .replace("%q", &format!("{:?}", name)),
            ));
        }
    }
    Ok(())
}

fn validate_field_def(name: &str, def: &Value) -> Result<()> {
    let Some(o) = def.as_object() else {
        return Err(schema_err(format!(
            "[@umpire/json] Field {:?} definition must be an object",
            name
        )));
    };
    assert_known_members(o, &["required", "default", "isEmpty"], "field definition")?;
    if let Some(required) = o.get("required") {
        if !required.is_boolean() {
            return Err(schema_err(format!(
                "[@umpire/json] Field {:?} required must be a boolean when provided",
                name
            )));
        }
    }
    if let Some(default_value) = o.get("default") {
        if !is_json_primitive(default_value) {
            return Err(schema_err(format!(
                "[@umpire/json] Field {:?} has a non-serializable default value",
                name
            )));
        }
    }
    if let Some(strategy) = o.get("isEmpty") {
        let s = strategy.as_str().unwrap_or_default();
        if !is_empty_strategy_supported(s) {
            return Err(schema_err(format!(
                "[@umpire/json] Unknown isEmpty strategy {:?}",
                s
            )));
        }
    }
    Ok(())
}

// validatorSpecMembers lists allowed spec members per op.
fn validator_spec_members(op: &Value) -> Vec<&'static str> {
    match op.as_str().unwrap_or_default() {
        "email" | "url" | "integer" => vec!["op"],
        "matches" => vec!["op", "pattern"],
        "minLength" | "maxLength" | "min" | "max" => vec!["op", "value"],
        "range" => vec!["op", "min", "max"],
        _ => vec!["op"],
    }
}

fn validate_validator_spec_members(
    spec: &Map<String, Value>,
    additional_members: &[&str],
) -> Result<()> {
    let mut allowed = validator_spec_members(spec.get("op").unwrap_or(&Value::Null));
    allowed.extend(additional_members.iter().copied());
    assert_known_members(spec, &allowed, "Validator spec")?;

    let op_value = spec.get("op").cloned().unwrap_or(Value::Null);
    match op_value.as_str().unwrap_or_default() {
        "email" | "url" | "integer" => {}
        "matches" => assert_string(
            spec.get("pattern").unwrap_or(&Value::Null),
            "Validator \"matches\" pattern",
        )?,
        "minLength" | "maxLength" | "min" | "max" => assert_number(
            spec.get("value").unwrap_or(&Value::Null),
            &format!(
                "Validator {:?} value",
                op_value.as_str().unwrap_or_default()
            ),
        )?,
        "range" => {
            assert_number(
                spec.get("min").unwrap_or(&Value::Null),
                "Validator \"range\" min",
            )?;
            assert_number(
                spec.get("max").unwrap_or(&Value::Null),
                "Validator \"range\" max",
            )?;
        }
        _ => {}
    }
    assert_valid_validator_spec(spec)
}

fn assert_valid_validator_spec(spec: &Map<String, Value>) -> Result<()> {
    let op_value = spec.get("op").cloned().unwrap_or(Value::Null);
    match op_value.as_str().unwrap_or_default() {
        "email" | "url" | "integer" => Ok(()),
        "matches" => {
            let pattern = spec
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if crate::checkops::regex_error_detail(pattern).is_empty() && regex_compiles(pattern) {
                Ok(())
            } else {
                Err(schema_err(format!(
                    "[@umpire/json] Invalid regex pattern {:?}: {}",
                    pattern,
                    regex_error_detail(pattern)
                )))
            }
        }
        "minLength" | "maxLength" | "min" | "max" => {
            match spec.get("value").and_then(Value::as_f64) {
                Some(v) if v.is_finite() => Ok(()),
                _ => Err(schema_err(format!(
                    "[@umpire/json] Validator {:?} requires a numeric value",
                    op_value.as_str().unwrap_or_default()
                ))),
            }
        }
        "range" => {
            let min = spec.get("min").and_then(Value::as_f64);
            let max = spec.get("max").and_then(Value::as_f64);
            match (min, max) {
                (Some(min), Some(max)) if min.is_finite() && max.is_finite() => Ok(()),
                _ => Err(schema_err(
                    "[@umpire/json] Validator \"range\" requires numeric min and max values",
                )),
            }
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown validator op {:?}",
            op_value
        ))),
    }
}

fn regex_compiles(pattern: &str) -> bool {
    regex::Regex::new(pattern).is_ok()
}

fn validate_validator(field: &str, validator: &Value, field_names: &HashSet<String>) -> Result<()> {
    if !field_names.contains(field) {
        return Err(schema_err(format!(
            "[@umpire/json] Validator references unknown field {:?} (validator)",
            field
        )));
    }
    let Some(o) = validator.as_object() else {
        return Err(schema_err(format!(
            "[@umpire/json] Validator for field {:?} must be an object",
            field
        )));
    };
    validate_validator_spec_members(o, &["error"])?;
    if let Some(err_value) = o.get("error") {
        if !err_value.is_string() {
            return Err(schema_err(format!(
                "[@umpire/json] Validator for field {:?} must use a string error when provided",
                field
            )));
        }
    }
    Ok(())
}

fn validate_excluded_rule(rule: &Value) -> Result<()> {
    let Some(o) = rule.as_object() else {
        return Err(schema_err("[@umpire/json] Excluded rule must be an object"));
    };
    assert_known_members(
        o,
        &["type", "field", "description", "key", "signature"],
        "Excluded rule",
    )?;
    let type_ = o.get("type").and_then(Value::as_str).unwrap_or_default();
    if type_.is_empty() {
        return Err(schema_err(
            "[@umpire/json] Excluded rules must include a non-empty string type",
        ));
    }
    if let Some(field_value) = o.get("field") {
        if !field_value.is_string() {
            return Err(schema_err(
                "[@umpire/json] Excluded rule field must be a string when provided",
            ));
        }
    }
    let desc = o
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if desc.is_empty() {
        return Err(schema_err(
            "[@umpire/json] Excluded rules must include a non-empty string description",
        ));
    }
    if let Some(key_value) = o.get("key") {
        if !key_value.is_string() {
            return Err(schema_err(
                "[@umpire/json] Excluded rule key must be a string when provided",
            ));
        }
    }
    if let Some(sig_value) = o.get("signature") {
        if !sig_value.is_string() {
            return Err(schema_err(
                "[@umpire/json] Excluded rule signature must be a string when provided",
            ));
        }
    }
    Ok(())
}

fn validate_expression(raw: &Value) -> Result<()> {
    let Some(o) = raw.as_object() else {
        return Err(schema_err("[@umpire/json] Expression must be an object"));
    };
    let op = o.get("op").and_then(Value::as_str).unwrap_or_default();

    match op {
        "eq" | "neq" => {
            assert_known_members(o, &["op", "field", "value"], "Expression")?;
            assert_string(
                o.get("field").unwrap_or(&Value::Null),
                &format!("Expression {:?} field", op),
            )?;
            assert_primitive(
                o.get("value").unwrap_or(&Value::Null),
                &format!("Expression {:?} value", op),
            )
        }
        "gt" | "gte" | "lt" | "lte" => {
            assert_known_members(o, &["op", "field", "value"], "Expression")?;
            assert_string(
                o.get("field").unwrap_or(&Value::Null),
                &format!("Expression {:?} field", op),
            )?;
            assert_number(
                o.get("value").unwrap_or(&Value::Null),
                &format!("Expression {:?} value", op),
            )
        }
        "present" | "absent" | "truthy" | "falsy" => {
            assert_known_members(o, &["op", "field"], "Expression")?;
            assert_string(
                o.get("field").unwrap_or(&Value::Null),
                &format!("Expression {:?} field", op),
            )
        }
        "in" | "notIn" => {
            assert_known_members(o, &["op", "field", "values"], "Expression")?;
            assert_string(
                o.get("field").unwrap_or(&Value::Null),
                &format!("Expression {:?} field", op),
            )?;
            assert_primitive_array(
                o.get("values").unwrap_or(&Value::Null),
                &format!("Expression {:?} values", op),
            )
        }
        "cond" => {
            assert_known_members(o, &["op", "condition"], "Expression")?;
            assert_string(
                o.get("condition").unwrap_or(&Value::Null),
                "Expression \"cond\" condition",
            )
        }
        "condEq" => {
            assert_known_members(o, &["op", "condition", "value"], "Expression")?;
            assert_string(
                o.get("condition").unwrap_or(&Value::Null),
                "Expression \"condEq\" condition",
            )?;
            assert_primitive(
                o.get("value").unwrap_or(&Value::Null),
                "Expression \"condEq\" value",
            )
        }
        "condIn" => {
            assert_known_members(o, &["op", "condition", "values"], "Expression")?;
            assert_string(
                o.get("condition").unwrap_or(&Value::Null),
                "Expression \"condIn\" condition",
            )?;
            assert_primitive_array(
                o.get("values").unwrap_or(&Value::Null),
                "Expression \"condIn\" values",
            )
        }
        "fieldInCond" => {
            assert_known_members(o, &["op", "field", "condition"], "Expression")?;
            assert_string(
                o.get("field").unwrap_or(&Value::Null),
                "Expression \"fieldInCond\" field",
            )?;
            assert_string(
                o.get("condition").unwrap_or(&Value::Null),
                "Expression \"fieldInCond\" condition",
            )
        }
        "and" | "or" => {
            assert_known_members(o, &["op", "exprs"], "Expression")?;
            let exprs = o.get("exprs").and_then(Value::as_array).ok_or_else(|| {
                schema_err(format!(
                    "[@umpire/json] Expression {:?} exprs must be an array",
                    op
                ))
            })?;
            for entry in exprs {
                validate_expression(entry)?;
            }
            Ok(())
        }
        "not" => {
            assert_known_members(o, &["op", "expr"], "Expression")?;
            validate_expression(o.get("expr").unwrap_or(&Value::Null))
        }
        "check" => {
            assert_known_members(o, &["op", "field", "check"], "Expression")?;
            assert_string(
                o.get("field").unwrap_or(&Value::Null),
                "Expression \"check\" field",
            )?;
            let check_obj = o
                .get("check")
                .and_then(Value::as_object)
                .ok_or_else(|| schema_err("[@umpire/json] Validator spec must be an object"))?;
            validate_validator_spec_members(check_obj, &[])
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown expression op {:?}",
            o.get("op").unwrap_or(&Value::Null)
        ))),
    }
}

fn assert_rule_field(field: &str, field_names: &HashSet<String>, context: &str) -> Result<()> {
    if !field_names.contains(field) {
        return Err(schema_err(format!(
            "[@umpire/json] Rule {} references unknown field {:?}",
            context, field
        )));
    }
    Ok(())
}

// ruleConstraint reports the rule's constraint phase.
fn rule_constraint(rule: &Map<String, Value>) -> Result<&'static str> {
    let rule_type = rule.get("type").and_then(Value::as_str).unwrap_or_default();
    match rule_type {
        "fairWhen" | "check" => Ok("fair"),
        "anyOf" => {
            let rules = rule
                .get("rules")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let shape = resolve_composite_shape("anyOf()", &rules)?;
            Ok(shape.constraint)
        }
        "eitherOf" => {
            let shape = resolve_either_of_shape(rule)?;
            Ok(shape.constraint)
        }
        _ => Ok("enabled"),
    }
}

// ruleTargets lists a rule's target fields.
fn rule_targets(rule: &Map<String, Value>) -> Result<Vec<String>> {
    let rule_type = rule.get("type").and_then(Value::as_str).unwrap_or_default();
    match rule_type {
        "requires" | "enabledWhen" | "fairWhen" | "check" => {
            let field = rule
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(vec![field.to_string()])
        }
        "disables" => Ok(rule
            .get("targets")
            .and_then(Value::as_array)
            .map(|targets| {
                targets
                    .iter()
                    .map(|t| t.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default()),
        "oneOf" => {
            let branches = rule
                .get("branches")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    schema_err("[@umpire/json] Rule \"oneOf\" branches must be an object")
                })?;
            let mut seen: HashSet<&str> = HashSet::new();
            let mut out: Vec<String> = Vec::new();
            for (branch_name, fields) in branches {
                let _ = branch_name;
                for f in fields.as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                    let name = f.as_str().unwrap_or_default();
                    if seen.insert(name) {
                        out.push(name.to_string());
                    }
                }
            }
            Ok(out)
        }
        "anyOf" => {
            let rules = rule
                .get("rules")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let shape = resolve_composite_shape("anyOf()", &rules)?;
            Ok(shape.targets)
        }
        "eitherOf" => {
            let shape = resolve_either_of_shape(rule)?;
            Ok(shape.targets)
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown rule type {:?}",
            rule.get("type").unwrap_or(&Value::Null)
        ))),
    }
}

struct CompositeShape {
    targets: Vec<String>,
    constraint: &'static str,
}

fn resolve_composite_shape(label: &str, rules: &[Value]) -> Result<CompositeShape> {
    if rules.is_empty() {
        return Err(schema_err(format!(
            "[@umpire/json] {} requires at least one rule",
            label
        )));
    }

    let empty_map = Map::new();
    let first = rules[0].as_object().unwrap_or(&empty_map);
    let first_targets = rule_targets(first)?;
    let expected = unique_sorted(&first_targets);
    let constraint = rule_constraint(first)?;

    for raw in &rules[1..] {
        let rule = raw.as_object().unwrap_or(&empty_map);
        let current_targets = rule_targets(rule)?;
        let current = unique_sorted(&current_targets);
        if current.len() != expected.len()
            || current.iter().zip(expected.iter()).any(|(a, b)| a != b)
        {
            return Err(schema_err(format!(
                "[@umpire/json] {} rules must target the same fields",
                label
            )));
        }
        let inner_constraint = rule_constraint(rule)?;
        if inner_constraint != constraint {
            return Err(schema_err(format!(
                "[@umpire/json] {} cannot mix fairWhen rules with availability rules",
                label
            )));
        }
    }

    Ok(CompositeShape {
        targets: first_targets,
        constraint,
    })
}

fn unique_sorted(fields: &[String]) -> Vec<String> {
    let mut seen: HashSet<&String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for f in fields {
        if seen.insert(f) {
            out.push(f.clone());
        }
    }
    out.sort();
    out
}

fn resolve_either_of_shape(rule: &Map<String, Value>) -> Result<CompositeShape> {
    let group = rule
        .get("group")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let branches = rule
        .get("branches")
        .and_then(Value::as_object)
        .ok_or_else(|| schema_err("[@umpire/json] Rule \"eitherOf\" branches must be an object"))?;
    if branches.is_empty() {
        return Err(schema_err(format!(
            "[@umpire/json] eitherOf({:?}) must include at least one branch",
            group
        )));
    }
    let mut all: Vec<Value> = Vec::new();
    for (branch_name, branch_rules) in branches {
        let list = branch_rules.as_array().ok_or_else(|| {
            schema_err(format!(
                "[@umpire/json] Rule \"eitherOf\" branch {:?} must be an array",
                branch_name
            ))
        })?;
        if list.is_empty() {
            return Err(schema_err(format!(
                "[@umpire/json] eitherOf({:?}) branch {:?} must not be empty",
                group, branch_name
            )));
        }
        all.extend(list.iter().cloned());
    }
    resolve_composite_shape(&format!("eitherOf({:?})", group), &all)
}

fn validate_rule_members(rule: &Map<String, Value>) -> Result<()> {
    let rule_type = rule.get("type").and_then(Value::as_str).unwrap_or_default();
    match rule_type {
        "requires" => {
            let allowed: &[&str] = if rule.contains_key("dependency") {
                &["type", "field", "dependency", "reason"]
            } else if rule.contains_key("dependencies") {
                &["type", "field", "dependencies", "reason"]
            } else {
                &["type", "field", "when", "reason"]
            };
            assert_known_members(rule, allowed, RULE_MEMBERS_CONTEXT)
        }
        "enabledWhen" | "fairWhen" => assert_known_members(
            rule,
            &["type", "field", "when", "reason"],
            RULE_MEMBERS_CONTEXT,
        ),
        "disables" => {
            let allowed: &[&str] = if rule.contains_key("source") {
                &["type", "source", "targets", "reason"]
            } else {
                &["type", "when", "targets", "reason"]
            };
            assert_known_members(rule, allowed, RULE_MEMBERS_CONTEXT)
        }
        "oneOf" | "eitherOf" => {
            assert_known_members(rule, &["type", "group", "branches"], RULE_MEMBERS_CONTEXT)
        }
        "anyOf" => assert_known_members(rule, &["type", "rules"], RULE_MEMBERS_CONTEXT),
        "check" => {
            let mut allowed = vec!["type", "field", "reason"];
            allowed.extend(validator_spec_members(
                rule.get("op").unwrap_or(&Value::Null),
            ));
            assert_known_members(rule, &allowed, RULE_MEMBERS_CONTEXT)
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown rule type {:?}",
            rule.get("type").unwrap_or(&Value::Null)
        ))),
    }
}

fn validate_rule_shape(rule: &Map<String, Value>) -> Result<()> {
    validate_rule_members(rule)?;
    if let Some(reason) = rule.get("reason") {
        assert_string(reason, "Rule reason")?;
    }

    let rule_type = rule.get("type").and_then(Value::as_str).unwrap_or_default();
    match rule_type {
        "requires" => {
            assert_string(
                rule.get("field").unwrap_or(&Value::Null),
                "Rule \"requires\" field",
            )?;
            if rule.contains_key("dependency") {
                return assert_string(
                    rule.get("dependency").unwrap_or(&Value::Null),
                    "Rule \"requires\" dependency",
                );
            }
            if rule.contains_key("dependencies") {
                let deps = rule
                    .get("dependencies")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        schema_err("[@umpire/json] Rule \"requires\" dependencies must be an array")
                    })?;
                for d in deps {
                    if d.is_string() {
                        continue;
                    }
                    validate_expression(d)?;
                }
                return Ok(());
            }
            validate_expression(rule.get("when").unwrap_or(&Value::Null))
        }
        "enabledWhen" | "fairWhen" => {
            assert_string(
                rule.get("field").unwrap_or(&Value::Null),
                &format!("Rule {:?} field", rule_type),
            )?;
            validate_expression(rule.get("when").unwrap_or(&Value::Null))
        }
        "disables" => {
            assert_string_array(
                rule.get("targets").unwrap_or(&Value::Null),
                "Rule \"disables\" targets",
            )?;
            if rule.contains_key("source") {
                return assert_string(
                    rule.get("source").unwrap_or(&Value::Null),
                    "Rule \"disables\" source",
                );
            }
            validate_expression(rule.get("when").unwrap_or(&Value::Null))
        }
        "oneOf" => {
            assert_string(
                rule.get("group").unwrap_or(&Value::Null),
                "Rule \"oneOf\" group",
            )?;
            let branches = rule
                .get("branches")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    schema_err("[@umpire/json] Rule \"oneOf\" branches must be an object")
                })?;
            for (branch_name, fields) in branches {
                assert_string_array(fields, &format!("Rule \"oneOf\" branch {:?}", branch_name))?;
            }
            Ok(())
        }
        "eitherOf" => {
            assert_string(
                rule.get("group").unwrap_or(&Value::Null),
                "Rule \"eitherOf\" group",
            )?;
            let branches = rule
                .get("branches")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    schema_err("[@umpire/json] Rule \"eitherOf\" branches must be an object")
                })?;
            for (branch_name, branch_rules) in branches {
                if !branch_rules.is_array() {
                    return Err(schema_err(format!(
                        "[@umpire/json] Rule \"eitherOf\" branch {:?} must be an array",
                        branch_name
                    )));
                }
            }
            Ok(())
        }
        "anyOf" => {
            if !rule.get("rules").is_some_and(Value::is_array) {
                return Err(schema_err(
                    "[@umpire/json] Rule \"anyOf\" rules must be an array",
                ));
            }
            Ok(())
        }
        "check" => {
            assert_string(
                rule.get("field").unwrap_or(&Value::Null),
                "Rule \"check\" field",
            )?;
            let spec_members: Vec<&str> = {
                let mut members = vec!["type", "field", "reason"];
                members.extend(validator_spec_members(
                    rule.get("op").unwrap_or(&Value::Null),
                ));
                members
            };
            validate_validator_spec_members_named(rule, &spec_members)
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown rule type {:?}",
            rule.get("type").unwrap_or(&Value::Null)
        ))),
    }
}

// validateValidatorSpecMembers with the check rule's own context: the rule
// object itself carries the op parameters alongside type/field/reason.
fn validate_validator_spec_members_named(
    rule: &Map<String, Value>,
    allowed: &[&str],
) -> Result<()> {
    assert_known_members(rule, allowed, RULE_MEMBERS_CONTEXT)?;
    let op_value = rule.get("op").cloned().unwrap_or(Value::Null);
    match op_value.as_str().unwrap_or_default() {
        "email" | "url" | "integer" => Ok(()),
        "matches" => assert_string(
            rule.get("pattern").unwrap_or(&Value::Null),
            "Validator \"matches\" pattern",
        ),
        "minLength" | "maxLength" | "min" | "max" => assert_number(
            rule.get("value").unwrap_or(&Value::Null),
            &format!(
                "Validator {:?} value",
                op_value.as_str().unwrap_or_default()
            ),
        ),
        "range" => {
            assert_number(
                rule.get("min").unwrap_or(&Value::Null),
                "Validator \"range\" min",
            )?;
            assert_number(
                rule.get("max").unwrap_or(&Value::Null),
                "Validator \"range\" max",
            )
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown validator op {:?}",
            op_value
        ))),
    }
}

// validateRule performs shape plus semantic validation for one rule,
// recursing into composites. Mirrors the TS core's validateRule.
fn validate_rule(
    raw: &Value,
    field_names: &HashSet<String>,
    conditions: &HashMap<String, ConditionDef>,
) -> Result<()> {
    let Some(rule) = raw.as_object() else {
        return Err(schema_err("[@umpire/json] Rule must be an object"));
    };
    validate_rule_shape(rule)?;

    let rule_type = rule.get("type").and_then(Value::as_str).unwrap_or_default();
    let field_of = || {
        rule.get("field")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match rule_type {
        "requires" => {
            let field = field_of();
            assert_rule_field(&field, field_names, "\"requires\"")?;
            if rule.contains_key("dependency") {
                let dependency = rule
                    .get("dependency")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return assert_rule_field(dependency, field_names, "\"requires\"");
            }
            if rule.contains_key("dependencies") {
                let deps = rule
                    .get("dependencies")
                    .and_then(Value::as_array)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                if deps.is_empty() {
                    return Err(schema_err(
                        "[@umpire/json] \"requires\" rules with dependencies must include at least one entry",
                    ));
                }
                for d in deps {
                    match d.as_str() {
                        Some(dep_name) => {
                            assert_rule_field(dep_name, field_names, "\"requires\"")?;
                        }
                        None => {
                            compile_expr(d, field_names, conditions)?;
                        }
                    }
                }
                return Ok(());
            }
            compile_expr(
                rule.get("when").unwrap_or(&Value::Null),
                field_names,
                conditions,
            )
            .map(|_| ())
        }
        "enabledWhen" => {
            let field = field_of();
            assert_rule_field(&field, field_names, "\"enabledWhen\"")?;
            compile_expr(
                rule.get("when").unwrap_or(&Value::Null),
                field_names,
                conditions,
            )
            .map(|_| ())
        }
        "disables" => {
            let targets = rule
                .get("targets")
                .and_then(Value::as_array)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            for t in targets {
                assert_rule_field(t.as_str().unwrap_or_default(), field_names, "\"disables\"")?;
            }
            if rule.contains_key("source") {
                let source = rule
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return assert_rule_field(source, field_names, "\"disables\"");
            }
            compile_expr(
                rule.get("when").unwrap_or(&Value::Null),
                field_names,
                conditions,
            )
            .map(|_| ())
        }
        "oneOf" => {
            let empty_map = Map::new();
            let branches = rule
                .get("branches")
                .and_then(Value::as_object)
                .unwrap_or(&empty_map);
            for (branch_name, fields) in branches {
                let _ = branch_name;
                for f in fields.as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                    assert_rule_field(f.as_str().unwrap_or_default(), field_names, "\"oneOf\"")?;
                }
            }
            Ok(())
        }
        "fairWhen" => {
            let field = field_of();
            assert_rule_field(&field, field_names, "\"fairWhen\"")?;
            compile_expr(
                rule.get("when").unwrap_or(&Value::Null),
                field_names,
                conditions,
            )
            .map(|_| ())
        }
        "eitherOf" => {
            let empty_map = Map::new();
            let branches = rule
                .get("branches")
                .and_then(Value::as_object)
                .unwrap_or(&empty_map);
            for (branch_name, branch_rules) in branches {
                let _ = branch_name;
                for inner in branch_rules.as_array().map(|v| v.as_slice()).unwrap_or(&[]) {
                    validate_rule(inner, field_names, conditions)?;
                }
            }
            resolve_either_of_shape(rule).map(|_| ())
        }
        "anyOf" => {
            let rules = rule
                .get("rules")
                .and_then(Value::as_array)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            for inner in rules {
                validate_rule(inner, field_names, conditions)?;
            }
            resolve_composite_shape("anyOf()", rules).map(|_| ())
        }
        "check" => {
            let field = field_of();
            assert_rule_field(&field, field_names, "\"check\"")
        }
        _ => Err(schema_err(format!(
            "[@umpire/json] Unknown rule type {:?}",
            rule.get("type").unwrap_or(&Value::Null)
        ))),
    }
}

// validateSchemaObject mirrors @umpire/json's validateSchema.
pub(crate) fn validate_schema_object(root: &Map<String, Value>) -> Result<()> {
    assert_known_members(
        root,
        &["version", "conditions", "fields", "rules", "validators", "excluded"],
        "Schema has unknown members; expected version, conditions, fields, rules, validators, excluded",
    )?;

    let version_value = root
        .get("version")
        .ok_or_else(|| schema_err("[@umpire/json] Schema must include a \"version\" field"))?;
    match version_value.as_f64() {
        Some(1.0) => {}
        _ => {
            return Err(schema_err(format!(
                "[@umpire/json] Unsupported schema version {:?}",
                version_value
            )))
        }
    }

    let empty = Map::new();
    let fields_obj = root
        .get("fields")
        .and_then(Value::as_object)
        .ok_or_else(|| schema_err("[@umpire/json] Schema must include a \"fields\" object"))?;

    match root.get("rules") {
        Some(raw_rules) => {
            if !raw_rules.is_array() {
                return Err(schema_err(
                    "[@umpire/json] Schema must include a \"rules\" array",
                ));
            }
        }
        None => {
            return Err(schema_err(
                "[@umpire/json] Schema must include a \"rules\" array",
            ))
        }
    }

    if let Some(raw_validators) = root.get("validators") {
        if !raw_validators.is_object() {
            return Err(schema_err(
                "[@umpire/json] Schema \"validators\" must be an object when provided",
            ));
        }
    }
    if let Some(raw_conditions) = root.get("conditions") {
        if !raw_conditions.is_object() {
            return Err(schema_err(
                "[@umpire/json] Schema \"conditions\" must be an object when provided",
            ));
        }
    }
    if let Some(raw_excluded) = root.get("excluded") {
        if !raw_excluded.is_array() {
            return Err(schema_err(
                "[@umpire/json] Schema \"excluded\" must be an array when provided",
            ));
        }
    }

    let mut conditions: HashMap<String, ConditionDef> = HashMap::new();
    if let Some(conditions_obj) = root.get("conditions").and_then(Value::as_object) {
        for (name, def) in conditions_obj {
            validate_condition(name, def)?;
            let def_obj = def.as_object().unwrap_or(&empty);
            let mut condition = ConditionDef {
                type_: def_obj
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                description: String::new(),
            };
            if let Some(desc) = def_obj.get("description").and_then(Value::as_str) {
                condition.description = desc.to_string();
            }
            conditions.insert(name.clone(), condition);
        }
    }

    let field_names: HashSet<String> = fields_obj.keys().cloned().collect();

    for (name, def) in fields_obj {
        validate_field_def(name, def)?;
    }

    if let Some(rules) = root.get("rules").and_then(Value::as_array) {
        for rule in rules {
            validate_rule(rule, &field_names, &conditions)?;
        }
    }

    if let Some(validators_obj) = root.get("validators").and_then(Value::as_object) {
        for (field, validator) in validators_obj {
            validate_validator(field, validator, &field_names)?;
        }
    }

    if let Some(excluded) = root.get("excluded").and_then(Value::as_array) {
        for entry in excluded {
            validate_excluded_rule(entry)?;
        }
    }

    if fields_obj.is_empty() {
        return Err(schema_err(
            "[@umpire/json] Schema must include at least one field",
        ));
    }

    Ok(())
}
