# umpire-rust

Rust-native Umpire semantics: a runtime evaluator for Umpire availability
schemas. Where [`umpire-go-gen`](https://github.com/umpire-tools/umpire-go-gen)
compiles a static `.umpire.json` into typed code ahead of time, this crate
evaluates schemas **live at runtime** — rules stay data, so a binary can load
a schema it has never seen without a rebuild.

It is a port of the TypeScript `@umpire/core` + `@umpire/json` semantics,
matching the [`umpire-spec`](https://github.com/umpire-tools/umpire-spec)
conformance suite (same fixtures `umpire-py`, `umpire-dart`, and `umpire-go`
run).

## Status

Conforming: the full pinned `umpire-spec` conformance suite passes — all
availability fixtures plus the validate/evaluate failure fixtures.

## Installation

```bash
cargo add umpire-core
```

## Quick Start

### From a JSON schema (the live path)

```rust
use serde_json::json;
use umpire::{parse_schema, Umpire};

const CHECKOUT_SCHEMA: &str = r#"{
    "version": 1,
    "fields": {
        "email": {"isEmpty": "string"},
        "plan": {"default": "basic", "isEmpty": "string"},
        "discount": {"isEmpty": "number"}
    },
    "rules": [
        {"type": "enabledWhen", "field": "discount",
         "when": {"op": "eq", "field": "plan", "value": "pro"},
         "reason": "Discounts are pro-only"}
    ],
    "validators": {"email": {"op": "email", "error": "Enter a valid email address"}}
}"#;

fn main() -> Result<(), umpire::UmpireError> {
    let schema = parse_schema(CHECKOUT_SCHEMA.as_bytes())?;
    let engine = Umpire::from_schema(schema)?;

    let values = serde_json::json!({"email": "nope", "plan": "basic", "discount": 10.0});
    let availability = engine.check(values.as_object().unwrap(), &Default::default(), None)?;

    let discount = &availability["discount"];
    assert!(!discount.enabled);
    assert_eq!(discount.reason, "Discounts are pro-only");

    let email = &availability["email"];
    assert_eq!(email.valid, Some(false));
    assert_eq!(email.error, "Enter a valid email address");
    Ok(())
}
```

### Programmatically (rules as code)

```rust
use umpire::{Pred, Rule, Umpire, FieldDef, FieldEntry, IsEmptyStrategy};

let engine = Umpire::new(
    vec![
        FieldEntry { name: "plan".into(), def: FieldDef { is_empty: IsEmptyStrategy::String, ..Default::default() } },
        FieldEntry { name: "discount".into(), def: FieldDef { is_empty: IsEmptyStrategy::Number, ..Default::default() } },
    ],
    vec![Rule::enabled_when(
        "discount",
        Pred::Eq { field: "plan".into(), value: serde_json::json!("pro") },
        "Discounts are pro-only",
    )],
    Default::default(),
)?;
```

## Semantics

Evaluation mirrors the TypeScript core: rules evaluate over fields in
topological order; gate rules run first and fairness rules only for
still-enabled fields; the first failure reason is reported alongside the full
reason list; `oneOf` resolves the active branch from satisfied values with
`prev`-assisted disambiguation; `requires` dependencies read the dependency's
already-evaluated availability.

## Conformance

```bash
make test   # syncs the pinned umpire-spec fixtures, then cargo test
```

`make spec-sync` fetches the `umpire-spec` tarball pinned by `UMPIRE_SPEC_SHA256`
in the Makefile, so a silent edit to the fixtures fails the build.
