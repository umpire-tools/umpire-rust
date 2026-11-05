use serde_json::{json, Map, Value};
use umpire::{schema_from_object, Umpire};

fn resolve_conformance_dir() -> String {
    if let Ok(dir) = std::env::var("UMPIRE_SPEC_CONFORMANCE_DIR") {
        let path = std::path::Path::new(&dir).join("index.json");
        assert!(
            path.exists(),
            "UMPIRE_SPEC_CONFORMANCE_DIR is set to {:?} but index.json was not found there",
            dir
        );
        return dir;
    }
    let dir = std::path::Path::new("spec").join("conformance");
    assert!(
        dir.join("index.json").exists(),
        "conformance fixtures not found at {} — run `make spec-sync` first",
        dir.display()
    );
    dir.to_string_lossy().into_owned()
}

fn load_fixture_index(dir: &str) -> (Vec<Value>, Vec<Value>) {
    let raw = std::fs::read(std::path::Path::new(dir).join("index.json")).expect("read index.json");
    let index: Value = serde_json::from_slice(&raw).expect("decode index.json");
    let fixtures = index
        .get("fixtures")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let failures = index
        .get("failures")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !fixtures.is_empty() && !failures.is_empty(),
        "index.json must list fixtures and failures"
    );
    (fixtures, failures)
}

fn load_fixture_file(dir: &str, rel_path: &str) -> Value {
    let raw = std::fs::read(std::path::Path::new(dir).join(rel_path))
        .unwrap_or_else(|err| panic!("read {}: {}", rel_path, err));
    serde_json::from_slice(&raw).unwrap_or_else(|err| panic!("decode {}: {}", rel_path, err))
}

fn availability_to_value(fa: &umpire::FieldAvailability) -> Value {
    let mut m = Map::new();
    m.insert("enabled".into(), json!(fa.enabled));
    m.insert("satisfied".into(), json!(fa.satisfied));
    m.insert("fair".into(), json!(fa.fair));
    m.insert("required".into(), json!(fa.required));
    m.insert(
        "reason".into(),
        if fa.reason.is_empty() {
            Value::Null
        } else {
            json!(fa.reason)
        },
    );
    m.insert("reasons".into(), json!(fa.reasons));
    if let Some(valid) = fa.valid {
        m.insert("valid".into(), json!(valid));
    }
    if !fa.error.is_empty() {
        m.insert("error".into(), json!(fa.error));
    }
    Value::Object(m)
}

fn map_of(value: Option<&Value>) -> &Map<String, Value> {
    use std::sync::OnceLock;
    static EMPTY: OnceLock<Map<String, Value>> = OnceLock::new();
    value
        .and_then(Value::as_object)
        .unwrap_or(EMPTY.get_or_init(Map::new))
}

#[test]
fn conformance_fixtures() {
    let dir = resolve_conformance_dir();
    let (fixtures, _) = load_fixture_index(&dir);

    for entry in &fixtures {
        let fixture_id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let fixture = load_fixture_file(&dir, path);

        let schema = schema_from_object(&fixture["schema"])
            .unwrap_or_else(|err| panic!("{}: schema parse failed: {}", fixture_id, err));
        let engine = Umpire::from_schema(schema)
            .unwrap_or_else(|err| panic!("{}: engine build failed: {}", fixture_id, err));

        let cases = fixture
            .get("cases")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for raw_case in &cases {
            let case_id = raw_case
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let values = map_of(raw_case.get("values"));
            let conditions = map_of(raw_case.get("conditions"));
            let prev = raw_case.get("prev").and_then(Value::as_object);

            let availability = engine
                .check(values, conditions, prev)
                .unwrap_or_else(|err| panic!("{}: check failed: {}", case_id, err));

            let expected = map_of(raw_case.get("expectedAvailability"));
            for (field, expected_status) in expected {
                let actual = availability.get(field).unwrap_or_else(|| {
                    panic!("{}: field {:?} missing from availability", case_id, field)
                });
                let actual_value = availability_to_value(actual);
                if expected_status != &actual_value {
                    assert_eq!(
                        expected_status, &actual_value,
                        "{} field {:?}",
                        case_id, field
                    );
                }
            }
        }
    }
}

#[test]
fn conformance_failure_fixtures() {
    let dir = resolve_conformance_dir();
    let (_, failures) = load_fixture_index(&dir);

    for entry in &failures {
        let _fixture_id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let fixture = load_fixture_file(&dir, path);

        let failure_list = fixture
            .get("failures")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (i, raw_failure) in failure_list.iter().enumerate() {
            let label = raw_failure
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("failure[{}]", i));
            let phase = raw_failure
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let error_includes = raw_failure
                .get("errorIncludes")
                .and_then(Value::as_str)
                .unwrap_or_default();

            let schema_result = schema_from_object(&raw_failure["schema"]);
            if phase == "validate" {
                match schema_result {
                    Ok(_) => panic!("{}: expected schema validation to fail", label),
                    Err(err) => assert!(
                        error_includes.is_empty() || err.message.contains(error_includes),
                        "{}: error {:?} does not include {:?}",
                        label,
                        err.message,
                        error_includes
                    ),
                }
                continue;
            }

            let schema = schema_result
                .unwrap_or_else(|err| panic!("{}: unexpected schema error: {}", label, err));
            let engine = Umpire::from_schema(schema)
                .unwrap_or_else(|err| panic!("{}: unexpected engine error: {}", label, err));
            let values = map_of(raw_failure.get("values"));
            let conditions = map_of(raw_failure.get("conditions"));
            let result = engine.check(values, conditions, None);
            let err = result
                .err()
                .unwrap_or_else(|| panic!("{}: expected check to fail", label));
            assert!(
                error_includes.is_empty() || err.message.contains(error_includes),
                "{}: error {:?} does not include {:?}",
                label,
                err.message,
                error_includes
            );
        }
    }
}
