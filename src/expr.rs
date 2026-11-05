use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::checkops::CheckOp;
use crate::error::{Result, UmpireError};
use crate::values::{is_truthy, json_contains, json_equal};

/// A compiled DSL expression reduced to a predicate. `check_field` is set
/// when the expression references exactly one field; the graph uses it for
/// informational (non-ordering) edges.
#[derive(Clone, Debug)]
pub(crate) struct CompiledExpr {
    pub pred: Pred,
    pub check_field: String,
}

impl CompiledExpr {
    pub(crate) fn eval(
        &self,
        values: &Map<String, Value>,
        conditions: &Map<String, Value>,
    ) -> Result<bool> {
        self.pred.eval(values, conditions)
    }
}

/// A DSL predicate over runtime values and conditions.
#[derive(Clone, Debug)]
pub enum Pred {
    Eq {
        field: String,
        value: Value,
    },
    Neq {
        field: String,
        value: Value,
    },
    Gt {
        field: String,
        bound: f64,
    },
    Gte {
        field: String,
        bound: f64,
    },
    Lt {
        field: String,
        bound: f64,
    },
    Lte {
        field: String,
        bound: f64,
    },
    Present {
        field: String,
    },
    Absent {
        field: String,
    },
    Truthy {
        field: String,
    },
    Falsy {
        field: String,
    },
    In {
        field: String,
        values: Vec<Value>,
    },
    NotIn {
        field: String,
        values: Vec<Value>,
    },
    Cond {
        condition: String,
    },
    CondEq {
        condition: String,
        value: Value,
    },
    CondIn {
        condition: String,
        values: Vec<Value>,
    },
    FieldInCond {
        field: String,
        condition: String,
    },
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
    Check {
        field: String,
        op: CheckOp,
    },
}

fn field_of(obj: &Map<String, Value>) -> String {
    obj.get("field")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn value_of(obj: &Map<String, Value>) -> Value {
    obj.get("value").cloned().unwrap_or(Value::Null)
}

fn values_of(obj: &Map<String, Value>) -> Vec<Value> {
    obj.get("values")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn assert_expr_field(field: &str, op: &str, field_names: &HashSet<String>) -> Result<()> {
    if !field_names.contains(field) {
        return Err(UmpireError::new(format!(
            "[@umpire/dsl] Unknown field {:?} in {:?} expression",
            field, op
        )));
    }
    Ok(())
}

fn get_condition_value<'a>(
    condition: &str,
    conditions: &'a Map<String, Value>,
) -> Result<&'a Value> {
    conditions.get(condition).ok_or_else(|| {
        UmpireError::new(format!(
            "[@umpire/dsl] Missing runtime condition {:?}",
            condition
        ))
    })
}

fn assert_condition_declared(
    condition: &str,
    op: &str,
    conditions: &HashMap<String, crate::schema::ConditionDef>,
) -> Result<()> {
    if !conditions.contains_key(condition) {
        return Err(UmpireError::new(format!(
            "[@umpire/dsl] Unknown condition {:?} in {:?} expression",
            condition, op
        )));
    }
    Ok(())
}

fn value_at<'a>(values: &'a Map<String, Value>, field: &str) -> &'a Value {
    values.get(field).unwrap_or(&Value::Null)
}

impl Pred {
    pub(crate) fn eval(
        &self,
        values: &Map<String, Value>,
        conditions: &Map<String, Value>,
    ) -> Result<bool> {
        match self {
            Pred::Eq { field, value } => Ok(json_equal(value_at(values, field), value)),
            Pred::Neq { field, value } => Ok(!json_equal(value_at(values, field), value)),
            Pred::Gt { field, bound } => {
                Ok(value_at(values, field).as_f64().is_some_and(|v| v > *bound))
            }
            Pred::Gte { field, bound } => Ok(value_at(values, field)
                .as_f64()
                .is_some_and(|v| v >= *bound)),
            Pred::Lt { field, bound } => {
                Ok(value_at(values, field).as_f64().is_some_and(|v| v < *bound))
            }
            Pred::Lte { field, bound } => Ok(value_at(values, field)
                .as_f64()
                .is_some_and(|v| v <= *bound)),
            Pred::Present { field } => Ok(!value_at(values, field).is_null()),
            Pred::Absent { field } => Ok(value_at(values, field).is_null()),
            Pred::Truthy { field } => Ok(is_truthy(value_at(values, field))),
            Pred::Falsy { field } => Ok(!is_truthy(value_at(values, field))),
            Pred::In {
                field,
                values: items,
            } => Ok(json_contains(items, value_at(values, field))),
            Pred::NotIn {
                field,
                values: items,
            } => Ok(!json_contains(items, value_at(values, field))),
            Pred::Cond { condition } => Ok(is_truthy(get_condition_value(condition, conditions)?)),
            Pred::CondEq { condition, value } => Ok(json_equal(
                get_condition_value(condition, conditions)?,
                value,
            )),
            Pred::CondIn {
                condition,
                values: items,
            } => Ok(json_contains(
                items,
                get_condition_value(condition, conditions)?,
            )),
            Pred::FieldInCond { field, condition } => {
                let cv = get_condition_value(condition, conditions)?;
                let items = cv.as_array().ok_or_else(|| {
                    UmpireError::new(format!(
                        "[@umpire/dsl] Runtime condition {:?} must be an array for \"fieldInCond\"",
                        condition
                    ))
                })?;
                Ok(json_contains(items, value_at(values, field)))
            }
            Pred::And(exprs) => {
                for e in exprs {
                    if !e.eval(values, conditions)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Pred::Or(exprs) => {
                for e in exprs {
                    if e.eval(values, conditions)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Pred::Not(inner) => Ok(!inner.eval(values, conditions)?),
            Pred::Check { field, op } => {
                let value = value_at(values, field);
                if value.is_null() {
                    return Ok(false);
                }
                Ok(op.validate(value))
            }
        }
    }
}

// compileExpr compiles a JSON expression into a predicate, mirroring
// @umpire/dsl's compileExpr plus @umpire/json's check expressions.
pub(crate) fn compile_expr(
    raw: &Value,
    field_names: &HashSet<String>,
    conditions: &HashMap<String, crate::schema::ConditionDef>,
) -> Result<CompiledExpr> {
    let obj = raw
        .as_object()
        .ok_or_else(|| UmpireError::new("[@umpire/dsl] Expression must be an object"))?;
    let op = obj.get("op").and_then(Value::as_str).unwrap_or_default();

    if let Some(pred) = field_predicate(obj, op, field_names)? {
        return Ok(pred);
    }
    if let Some(pred) = condition_predicate(obj, op, conditions)? {
        return Ok(pred);
    }

    match op {
        "fieldInCond" => {
            let field = field_of(obj);
            assert_expr_field(&field, op, field_names)?;
            let condition = obj
                .get("condition")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let def = conditions.get(condition).ok_or_else(|| {
                UmpireError::new(format!(
                    "[@umpire/dsl] Unknown condition {:?} in {:?} expression",
                    condition, op
                ))
            })?;
            if def.type_ != "string[]" && def.type_ != "number[]" {
                return Err(UmpireError::new(format!(
                    "[@umpire/dsl] \"fieldInCond\" requires an array condition, but {:?} is {:?}",
                    condition, def.type_
                )));
            }
            Ok(with_check_field(
                Pred::FieldInCond {
                    field: field.clone(),
                    condition: condition.to_string(),
                },
                field,
            ))
        }
        "and" | "or" => {
            let exprs = obj.get("exprs").and_then(Value::as_array).ok_or_else(|| {
                UmpireError::new(format!(
                    "[@umpire/dsl] {:?} expression requires an exprs array",
                    op
                ))
            })?;
            if exprs.is_empty() {
                return Err(UmpireError::new(format!(
                    "[@umpire/dsl] {:?} expression requires an exprs array",
                    op
                )));
            }
            let mut compiled = Vec::with_capacity(exprs.len());
            let mut refs: Vec<String> = Vec::new();
            for entry in exprs {
                let c = compile_expr(entry, field_names, conditions)?;
                if !c.check_field.is_empty() && !refs.contains(&c.check_field) {
                    refs.push(c.check_field);
                }
                compiled.push(c.pred);
            }
            let pred = if op == "and" {
                Pred::And(compiled)
            } else {
                Pred::Or(compiled)
            };
            Ok(with_check_field(pred, single_ref(refs)))
        }
        "not" => {
            let inner_raw = obj.get("expr").ok_or_else(|| {
                UmpireError::new("[@umpire/dsl] \"not\" expression requires an expr field")
            })?;
            let inner = compile_expr(inner_raw, field_names, conditions)?;
            let check_field = inner.check_field.clone();
            Ok(with_check_field(
                Pred::Not(Box::new(inner.pred)),
                check_field,
            ))
        }
        "check" => {
            let field = field_of(obj);
            assert_expr_field(&field, op, field_names)?;
            let check_obj = obj
                .get("check")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            let op_check = parse_check(check_obj)?;
            Ok(with_check_field(
                Pred::Check {
                    field: field.clone(),
                    op: op_check,
                },
                field,
            ))
        }
        _ => Err(UmpireError::new(format!(
            "[@umpire/dsl] Unknown expression op {:?}",
            op
        ))),
    }
}

fn parse_check(check: Value) -> Result<CheckOp> {
    crate::checkops::parse_check_op(&check)
}

fn with_check_field(pred: Pred, check_field: String) -> CompiledExpr {
    CompiledExpr { pred, check_field }
}

fn single_ref(refs: Vec<String>) -> String {
    if refs.len() == 1 {
        refs.into_iter().next().unwrap_or_default()
    } else {
        String::new()
    }
}

// field_predicate handles the field-bound ops; returns None when the op is
// not field-bound.
fn field_predicate(
    obj: &Map<String, Value>,
    op: &str,
    field_names: &HashSet<String>,
) -> Result<Option<CompiledExpr>> {
    let field = || field_of(obj);
    let checked = |field: String, pred: Pred| -> Result<Option<CompiledExpr>> {
        assert_expr_field(&field, op, field_names)?;
        Ok(Some(with_check_field(pred, field)))
    };
    match op {
        "eq" => checked(
            field(),
            Pred::Eq {
                field: field(),
                value: value_of(obj),
            },
        ),
        "neq" => checked(
            field(),
            Pred::Neq {
                field: field(),
                value: value_of(obj),
            },
        ),
        "gt" => checked(
            field(),
            Pred::Gt {
                field: field(),
                bound: value_of(obj).as_f64().unwrap_or(0.0),
            },
        ),
        "gte" => checked(
            field(),
            Pred::Gte {
                field: field(),
                bound: value_of(obj).as_f64().unwrap_or(0.0),
            },
        ),
        "lt" => checked(
            field(),
            Pred::Lt {
                field: field(),
                bound: value_of(obj).as_f64().unwrap_or(0.0),
            },
        ),
        "lte" => checked(
            field(),
            Pred::Lte {
                field: field(),
                bound: value_of(obj).as_f64().unwrap_or(0.0),
            },
        ),
        "present" => checked(field(), Pred::Present { field: field() }),
        "absent" => checked(field(), Pred::Absent { field: field() }),
        "truthy" => checked(field(), Pred::Truthy { field: field() }),
        "falsy" => checked(field(), Pred::Falsy { field: field() }),
        "in" => checked(
            field(),
            Pred::In {
                field: field(),
                values: values_of(obj),
            },
        ),
        "notIn" => checked(
            field(),
            Pred::NotIn {
                field: field(),
                values: values_of(obj),
            },
        ),
        _ => Ok(None),
    }
}

// condition_predicate handles cond/condEq/condIn; returns None when the op is
// not condition-bound.
fn condition_predicate(
    obj: &Map<String, Value>,
    op: &str,
    conditions: &HashMap<String, crate::schema::ConditionDef>,
) -> Result<Option<CompiledExpr>> {
    let condition = || {
        obj.get("condition")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let declared = |condition: String, pred: Pred| -> Result<Option<CompiledExpr>> {
        assert_condition_declared(&condition, op, conditions)?;
        Ok(Some(with_check_field(pred, String::new())))
    };
    match op {
        "cond" => declared(
            condition(),
            Pred::Cond {
                condition: condition(),
            },
        ),
        "condEq" => declared(
            condition(),
            Pred::CondEq {
                condition: condition(),
                value: value_of(obj),
            },
        ),
        "condIn" => declared(
            condition(),
            Pred::CondIn {
                condition: condition(),
                values: values_of(obj),
            },
        ),
        _ => Ok(None),
    }
}
