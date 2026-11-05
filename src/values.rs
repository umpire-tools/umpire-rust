use serde_json::Value;

// jsonEqual mirrors JavaScript strict equality (===) over JSON-decoded
// values: numbers compare numerically, strings/booleans by value, and
// containers never equal across types.
pub(crate) fn json_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Null, _) | (_, Value::Null) => false,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        _ => a == b,
    }
}

// jsonContains mirrors Array.prototype.includes: strict equality against
// each element.
pub(crate) fn json_contains(items: &[Value], needle: &Value) -> bool {
    items.iter().any(|item| json_equal(item, needle))
}

// isTruthy mirrors JavaScript Boolean() coercion over JSON values.
// Empty arrays and objects are truthy in JavaScript, so they stay truthy here.
pub(crate) fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

// formatJSONNumber renders a number the way JavaScript would interpolate
// it into a string: 8.0 becomes "8", 2.5 stays "2.5".
pub(crate) fn format_json_number(v: f64) -> String {
    format!("{}", v)
}
