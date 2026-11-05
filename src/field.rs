use serde_json::Value;

/// Names how a field decides its value is "empty".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IsEmptyStrategy {
    /// Treats only null/absent values as empty (the default).
    #[default]
    Present,
    /// Treats non-strings and empty strings as empty.
    String,
    /// Treats non-numbers and NaN as empty.
    Number,
    /// Treats non-booleans as empty.
    Boolean,
    /// Treats non-arrays and empty arrays as empty.
    Array,
    /// Treats non-objects and empty objects as empty.
    Object,
}

impl IsEmptyStrategy {
    pub(crate) fn from_name(name: &str) -> Option<IsEmptyStrategy> {
        match name {
            "present" => Some(IsEmptyStrategy::Present),
            "string" => Some(IsEmptyStrategy::String),
            "number" => Some(IsEmptyStrategy::Number),
            "boolean" => Some(IsEmptyStrategy::Boolean),
            "array" => Some(IsEmptyStrategy::Array),
            "object" => Some(IsEmptyStrategy::Object),
            _ => None,
        }
    }
}

/// Defines a field's structure and emptiness semantics.
#[derive(Clone, Debug, Default)]
pub struct FieldDef {
    pub required: bool,
    pub default: Option<Value>,
    pub is_empty: IsEmptyStrategy,
}

pub(crate) fn is_empty(strategy: IsEmptyStrategy, value: &Value) -> bool {
    match strategy {
        IsEmptyStrategy::String => match value {
            Value::String(s) => s.is_empty(),
            _ => true,
        },
        IsEmptyStrategy::Number => match value {
            Value::Number(n) => n.as_f64().is_none_or(|f| f.is_nan()),
            _ => true,
        },
        IsEmptyStrategy::Boolean => !value.is_boolean(),
        IsEmptyStrategy::Array => match value {
            Value::Array(a) => a.is_empty(),
            _ => true,
        },
        IsEmptyStrategy::Object => match value {
            Value::Object(o) => o.is_empty(),
            _ => true,
        },
        IsEmptyStrategy::Present => value.is_null(),
    }
}

/// Reports whether a value counts as present for a field.
pub(crate) fn is_satisfied(value: &Value, def: &FieldDef) -> bool {
    !is_empty(def.is_empty, value)
}
