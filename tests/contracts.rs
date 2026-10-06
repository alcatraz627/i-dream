//! The two JSON contracts the widget reads, checked against the schemas in
//! docs/contracts. The widget never opens a store file, so a field these
//! commands drop is a blank on the owner's screen; this test fails first.

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

fn sandbox() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude/subconscious")).unwrap();
    dir
}

fn run_json(home: &Path, args: &[&str]) -> Value {
    let out = Command::cargo_bin("i-dream")
        .unwrap()
        .env("HOME", home)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("valid JSON")
}

fn schema(name: &str) -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/contracts").join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// The subset of JSON Schema the contracts use: object `required` keys,
/// nested `properties`, and `items` of non-empty arrays. Returns every
/// missing path.
fn missing(value: &Value, schema: &Value, at: &str, out: &mut Vec<String>) {
    if value.is_null() {
        return;
    }
    if let Some(req) = schema.get("required").and_then(|r| r.as_array()) {
        for k in req.iter().filter_map(|k| k.as_str()) {
            if value.get(k).is_none() {
                out.push(format!("{at}.{k}"));
            }
        }
    }
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (k, sub) in props {
            if let Some(v) = value.get(k) {
                missing(v, sub, &format!("{at}.{k}"), out);
            }
        }
    }
    if let (Some(items), Some(arr)) = (schema.get("items"), value.as_array()) {
        for (i, v) in arr.iter().enumerate() {
            missing(v, items, &format!("{at}[{i}]"), out);
        }
    }
}

#[test]
fn status_json_meets_its_contract_and_names_every_source() {
    let home = sandbox();
    let v = run_json(home.path(), &["status", "--json"]);
    let mut gaps = vec![];
    missing(&v, &schema("status.schema.json"), "status", &mut gaps);
    assert!(gaps.is_empty(), "status --json is missing {gaps:?}");
    let sources = v["sources"].as_object().unwrap();
    for key in v.as_object().unwrap().keys().filter(|k| *k != "sources" && *k != "state_error") {
        let named = sources.get(key).and_then(|s| s.as_str()).unwrap_or("");
        assert!(!named.is_empty(), "section {key} names no source");
    }
}

#[test]
fn reader_json_meets_its_contract() {
    let home = sandbox();
    let v = run_json(home.path(), &["reader", "--json"]);
    let mut gaps = vec![];
    missing(&v, &schema("reader.schema.json"), "reader", &mut gaps);
    assert!(gaps.is_empty(), "reader --json is missing {gaps:?}");
}

#[test]
fn the_checker_catches_a_dropped_field() {
    let s = schema("status.schema.json");
    let mut gaps = vec![];
    missing(&serde_json::json!({"daemon": {}}), &s, "status", &mut gaps);
    assert!(gaps.contains(&"status.daemon.status".to_string()));
    assert!(gaps.contains(&"status.reader".to_string()));
}
