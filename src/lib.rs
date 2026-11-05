//! Rust-native Umpire semantics: a runtime evaluator for Umpire availability
//! schemas. Rules stay data, so a binary can load a schema it has never seen
//! without a rebuild.
//!
//! This crate is a port of the TypeScript `@umpire/core` + `@umpire/json`
//! semantics and the Go `umpire-go` runtime, matching the
//! [`umpire-spec`](https://github.com/umpire-tools/umpire-spec) conformance
//! suite (the same fixtures `umpire-py`, `umpire-dart`, and `umpire-go` run).

mod checkops;
mod engine;
mod error;
mod evaluator;
mod expr;
mod field;
mod graph;
mod rule;
mod schema;
mod schema_validate;
mod values;

pub use checkops::CheckOp;
pub use engine::{FieldEntry, Foul, Umpire};
pub use error::UmpireError;
pub use evaluator::{AvailabilityMap, FieldAvailability, RuleEvaluation};
pub use expr::Pred;
pub use field::{FieldDef, IsEmptyStrategy};
pub use rule::Rule;
pub use schema::{parse_schema, schema_from_object, ConditionDef, Schema, ValidatorEntry};
