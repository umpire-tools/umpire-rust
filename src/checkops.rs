use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::error::{Result, UmpireError};
use crate::values::format_json_number;

/// A named validation check with metadata for introspection.
#[derive(Clone, Debug)]
pub enum CheckOp {
    Email,
    Url,
    Matches(String),
    MinLength(f64),
    MaxLength(f64),
    Min(f64),
    Max(f64),
    Range { min: f64, max: f64 },
    Integer,
}

impl CheckOp {
    pub fn name(&self) -> &'static str {
        match self {
            CheckOp::Email => "email",
            CheckOp::Url => "url",
            CheckOp::Matches(_) => "matches",
            CheckOp::MinLength(_) => "minLength",
            CheckOp::MaxLength(_) => "maxLength",
            CheckOp::Min(_) => "min",
            CheckOp::Max(_) => "max",
            CheckOp::Range { .. } => "range",
            CheckOp::Integer => "integer",
        }
    }

    /// The reason reported when the check fails.
    pub fn reason(&self) -> String {
        match self {
            CheckOp::Email => "Must be a valid email address".to_string(),
            CheckOp::Url => "Must be a valid URL".to_string(),
            CheckOp::Matches(_) => "Must match the required format".to_string(),
            CheckOp::MinLength(min) => {
                format!("Must be at least {} characters", format_json_number(*min))
            }
            CheckOp::MaxLength(max) => {
                format!("Must be {} characters or fewer", format_json_number(*max))
            }
            CheckOp::Min(min) => format!("Must be at least {}", format_json_number(*min)),
            CheckOp::Max(max) => format!("Must be {} or less", format_json_number(*max)),
            CheckOp::Range { min, max } => format!(
                "Must be between {} and {}",
                format_json_number(*min),
                format_json_number(*max)
            ),
            CheckOp::Integer => "Must be a whole number".to_string(),
        }
    }

    pub fn validate(&self, value: &Value) -> bool {
        match self {
            CheckOp::Email => valid_email(value),
            CheckOp::Url => valid_url(value),
            CheckOp::Matches(pattern) => match value {
                Value::String(s) => regex_for(pattern).is_some_and(|re| re.is_match(s)),
                _ => false,
            },
            CheckOp::MinLength(min) => length_of(value) >= *min,
            CheckOp::MaxLength(max) => is_length_like(value) && length_of(value) <= *max,
            CheckOp::Min(min) => value.is_number() && value.as_f64().is_some_and(|v| v >= *min),
            CheckOp::Max(max) => value.is_number() && value.as_f64().is_some_and(|v| v <= *max),
            CheckOp::Range { min, max } => {
                value.is_number() && value.as_f64().is_some_and(|v| v >= *min && v <= *max)
            }
            CheckOp::Integer => value.as_f64().is_some_and(|v| v == v.trunc()),
        }
    }
}

fn email_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The RE2-safe remainder of the @umpire/json email pattern. The
        // original lookahead guards (no leading dot, no "..") are checked in
        // code because RE2 does not support lookaheads.
        Regex::new(
            r"^([A-Za-z0-9_'+\-.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$",
        )
        .expect("built-in email regex must compile")
    })
}

fn valid_email(value: &Value) -> bool {
    let s = match value {
        Value::String(s) => s,
        _ => return false,
    };
    if s.is_empty() || s.starts_with('.') || s.contains("..") {
        return false;
    }
    email_regex().is_match(s)
}

fn url_scheme_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").expect("built-in URL scheme regex must compile")
    })
}

fn valid_url(value: &Value) -> bool {
    let s = match value {
        Value::String(s) => s,
        _ => return false,
    };
    // Mirror WHATWG new URL(): a scheme is mandatory, and special schemes
    // require a host (the url crate rejects those at parse time).
    if !url_scheme_regex().is_match(s) {
        return false;
    }
    url::Url::parse(s).is_ok()
}

fn regex_for(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

fn is_length_like(v: &Value) -> bool {
    matches!(v, Value::String(_) | Value::Array(_))
}

fn length_of(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.len() as f64,
        Value::Array(a) => a.len() as f64,
        _ => -1.0,
    }
}

/// Builds a CheckOp from a flat spec object (op plus params).
pub(crate) fn parse_check_op(spec: &Value) -> Result<CheckOp> {
    let obj = spec
        .as_object()
        .ok_or_else(|| UmpireError::new("expected object"))?;
    let op_name = obj.get("op").and_then(Value::as_str).unwrap_or_default();
    match op_name {
        "email" => Ok(CheckOp::Email),
        "url" => Ok(CheckOp::Url),
        "matches" => {
            let pattern = require_string(spec, "pattern")?;
            if Regex::new(&pattern).is_err() {
                return Err(UmpireError::new(format!(
                    "[@umpire/json] Invalid regex pattern {:?}: {}",
                    pattern,
                    regex_error_detail(&pattern)
                )));
            }
            Ok(CheckOp::Matches(pattern))
        }
        "minLength" => Ok(CheckOp::MinLength(require_number(spec, "value")?)),
        "maxLength" => Ok(CheckOp::MaxLength(require_number(spec, "value")?)),
        "min" => Ok(CheckOp::Min(require_number(spec, "value")?)),
        "max" => Ok(CheckOp::Max(require_number(spec, "value")?)),
        "range" => {
            let min = require_number(spec, "min")?;
            let max = require_number(spec, "max")?;
            Ok(CheckOp::Range { min, max })
        }
        "integer" => Ok(CheckOp::Integer),
        _ => Err(UmpireError::new(format!("unknown check op: {:?}", op_name))),
    }
}

fn require_string(spec: &Value, key: &str) -> Result<String> {
    spec.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| UmpireError::new(format!("{:?} must be a string", key)))
}

fn require_number(spec: &Value, key: &str) -> Result<f64> {
    spec.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| UmpireError::new(format!("{:?} must be a number", key)))
}

pub(crate) fn regex_error_detail(pattern: &str) -> String {
    match Regex::new(pattern) {
        Err(err) => err.to_string(),
        Ok(_) => String::new(),
    }
}
