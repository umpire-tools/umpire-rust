use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use umpire::{CheckOp, FieldDef, FieldEntry, IsEmptyStrategy, Pred, Rule, Umpire, ValidatorEntry};

fn test_fields() -> Vec<FieldEntry> {
    vec![
        FieldEntry {
            name: "email".into(),
            def: FieldDef {
                is_empty: IsEmptyStrategy::String,
                ..Default::default()
            },
        },
        FieldEntry {
            name: "plan".into(),
            def: FieldDef {
                default: Some(json!("basic")),
                is_empty: IsEmptyStrategy::String,
                ..Default::default()
            },
        },
        FieldEntry {
            name: "discount".into(),
            def: FieldDef {
                is_empty: IsEmptyStrategy::Number,
                ..Default::default()
            },
        },
        FieldEntry {
            name: "company".into(),
            def: FieldDef {
                is_empty: IsEmptyStrategy::String,
                ..Default::default()
            },
        },
    ]
}

fn empty_map() -> Map<String, Value> {
    Map::new()
}

#[test]
fn check_programmatic_rules() {
    let engine = Umpire::new(
        test_fields(),
        vec![
            Rule::enabled_when(
                "discount",
                Pred::Eq {
                    field: "plan".into(),
                    value: json!("pro"),
                },
                "",
            ),
            Rule::requires_dependency("company", "email", ""),
        ],
        IndexMap::from([(
            "email".to_string(),
            ValidatorEntry {
                op: CheckOp::Email,
                error: "Bad email".into(),
                has_error: true,
            },
        )]),
    )
    .expect("engine builds");

    let values: Map<String, Value> = serde_json::from_value(json!({
        "email": "not-an-email", "plan": "pro", "discount": 10.0
    }))
    .unwrap();
    let availability = engine.check(&values, &empty_map(), None).unwrap();

    let discount = &availability["discount"];
    assert!(
        discount.enabled && discount.satisfied,
        "discount: {:?}",
        discount
    );
    let company = &availability["company"];
    assert!(
        company.enabled && !company.required,
        "company: {:?}",
        company
    );
    assert!(
        !company.satisfied,
        "company is absent, should not be satisfied"
    );
    let email = &availability["email"];
    assert_eq!(email.valid, Some(false), "email should report invalid");
    assert_eq!(email.error, "Bad email");
    assert!(email.enabled, "email has no gate rules; expected enabled");
}

#[test]
fn check_missing_condition_fails() {
    let engine = Umpire::new(
        test_fields(),
        vec![Rule::enabled_when(
            "discount",
            Pred::Cond {
                condition: "isPro".into(),
            },
            "",
        )],
        IndexMap::new(),
    )
    .expect("engine builds");

    let values: Map<String, Value> = serde_json::from_value(json!({"plan": "basic"})).unwrap();
    let result = engine.check(&values, &empty_map(), None);
    let err = result.expect_err("expected missing-condition error");
    assert!(
        err.message.contains("Missing runtime condition"),
        "error: {}",
        err.message
    );
}

#[test]
fn one_of_prev_resolution() {
    let engine = Umpire::new(
        vec![
            FieldEntry {
                name: "bullpenPhone".into(),
                def: FieldDef {
                    is_empty: IsEmptyStrategy::String,
                    ..Default::default()
                },
            },
            FieldEntry {
                name: "outfieldSignal".into(),
                def: FieldDef {
                    is_empty: IsEmptyStrategy::String,
                    ..Default::default()
                },
            },
        ],
        vec![Rule::one_of(
            "relay",
            vec![
                ("phone".into(), vec!["bullpenPhone".into()]),
                ("signal".into(), vec!["outfieldSignal".into()]),
            ],
            "",
        )],
        IndexMap::new(),
    )
    .expect("engine builds");

    // Both branches satisfied; prev had phone satisfied → signal is newly
    // satisfied and wins.
    let values: Map<String, Value> =
        serde_json::from_value(json!({"bullpenPhone": "ring", "outfieldSignal": "wave"})).unwrap();
    let prev: Map<String, Value> = serde_json::from_value(json!({"bullpenPhone": "ring"})).unwrap();
    let availability = engine.check(&values, &empty_map(), Some(&prev)).unwrap();
    assert!(
        availability["outfieldSignal"].enabled,
        "signal branch should win via prev: {:?}",
        availability["outfieldSignal"]
    );
    assert!(
        !availability["bullpenPhone"].enabled,
        "phone branch should be locked out"
    );
    assert_eq!(
        availability["bullpenPhone"].reason,
        "conflicts with signal strategy"
    );
}

#[test]
fn play_fouls() {
    let engine = Umpire::new(
        test_fields(),
        vec![Rule::enabled_when(
            "discount",
            Pred::Eq {
                field: "plan".into(),
                value: json!("pro"),
            },
            "Pro plan only",
        )],
        IndexMap::new(),
    )
    .expect("engine builds");

    let before_values: Map<String, Value> =
        serde_json::from_value(json!({"plan": "pro", "discount": 10.0})).unwrap();
    let after_values: Map<String, Value> =
        serde_json::from_value(json!({"plan": "basic", "discount": 10.0})).unwrap();
    let before = engine.check(&before_values, &empty_map(), None).unwrap();
    let after = engine.check(&after_values, &empty_map(), None).unwrap();

    let fouls = engine.play(&before, &after, &after_values);
    assert_eq!(fouls.len(), 1, "expected 1 foul, got {:?}", fouls);
    let foul = &fouls[0];
    assert_eq!(foul.field, "discount");
    assert_eq!(foul.reason, "Pro plan only");
    assert_eq!(foul.suggested_value, None);
}

#[test]
fn init_defaults() {
    let engine = Umpire::new(test_fields(), Vec::new(), IndexMap::new()).expect("engine builds");
    let overrides: Map<String, Value> =
        serde_json::from_value(json!({"email": "a@b.com"})).unwrap();
    let values = engine.init(&overrides);
    assert_eq!(values.get("plan"), Some(&json!("basic")));
    assert_eq!(values.get("email"), Some(&json!("a@b.com")));
    assert!(
        values.contains_key("discount"),
        "discount should be initialized (null default)"
    );
}

#[test]
fn cycle_detection() {
    let result = Umpire::new(
        test_fields(),
        vec![
            Rule::requires_dependency("company", "email", ""),
            Rule::requires_dependency("email", "company", ""),
        ],
        IndexMap::new(),
    );
    let err = result.expect_err("expected cycle detection error");
    assert!(
        err.message.contains("Cycle detected"),
        "error: {}",
        err.message
    );
}

#[test]
fn schema_round_trip_warnings() {
    let schema_json = r#"{
        "version": 1,
        "fields": {"email": {"isEmpty": "string"}},
        "rules": [],
        "validators": {"email": {"op": "email", "error": "Enter a valid email address"}},
        "excluded": [
            {"type": "custom", "field": "email", "description": "handle natively"}
        ]
    }"#;
    let schema = umpire::parse_schema(schema_json.as_bytes()).expect("schema parses");
    assert_eq!(schema.warnings.len(), 1, "expected 1 warning");
    let engine = Umpire::from_schema(schema).expect("engine builds");
    let values: Map<String, Value> = serde_json::from_value(json!({"email": "nope"})).unwrap();
    let availability = engine.check(&values, &empty_map(), None).unwrap();
    let email = &availability["email"];
    assert_eq!(email.valid, Some(false), "email should be invalid");
    assert_eq!(email.error, "Enter a valid email address");
}

// Conditions flow through Pred without being declared in a schema when the
// engine is built programmatically; assert the cond predicates evaluate.
#[test]
fn cond_predicates_evaluate() {
    let engine = Umpire::new(
        vec![FieldEntry {
            name: "discount".into(),
            def: FieldDef {
                is_empty: IsEmptyStrategy::Number,
                ..Default::default()
            },
        }],
        vec![
            Rule::enabled_when(
                "discount",
                Pred::Cond {
                    condition: "allowDiscount".into(),
                },
                "",
            ),
            Rule::enabled_when(
                "discount",
                Pred::Or(vec![
                    Pred::CondEq {
                        condition: "tier".into(),
                        value: json!("gold"),
                    },
                    Pred::CondIn {
                        condition: "regions".into(),
                        values: vec![json!("us"), json!("eu")],
                    },
                ]),
                "tier or region gate",
            ),
        ],
        IndexMap::new(),
    )
    .expect("engine builds");

    let values: Map<String, Value> = serde_json::from_value(json!({"discount": 10.0})).unwrap();
    let conditions: Map<String, Value> = serde_json::from_value(json!({
        "allowDiscount": true, "tier": "gold", "regions": ["eu"]
    }))
    .unwrap();
    let availability = engine.check(&values, &conditions, None).unwrap();
    assert!(
        availability["discount"].enabled,
        "{:?}",
        availability["discount"]
    );

    let conditions_off: Map<String, Value> = serde_json::from_value(json!({
        "allowDiscount": false, "tier": "bronze", "regions": []
    }))
    .unwrap();
    let availability = engine.check(&values, &conditions_off, None).unwrap();
    assert!(!availability["discount"].enabled);
    assert_eq!(availability["discount"].reason, "condition not met");
}
