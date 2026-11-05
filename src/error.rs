use std::fmt;

/// An Umpire failure: schema validation, compilation, or evaluation.
/// Messages carry the same `[@umpire/...]` prefixes as the TypeScript and Go
/// runtimes; conformance failure fixtures match against their substrings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UmpireError {
    pub message: String,
}

impl UmpireError {
    pub(crate) fn new(message: impl Into<String>) -> UmpireError {
        UmpireError {
            message: message.into(),
        }
    }
}

impl fmt::Display for UmpireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UmpireError {}

pub(crate) type Result<T> = std::result::Result<T, UmpireError>;

pub(crate) fn schema_err(message: impl Into<String>) -> UmpireError {
    UmpireError::new(message)
}
