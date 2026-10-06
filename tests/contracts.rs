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

/// The widget's status number is hours since the last cycle that produced
/// something, so a cycle that ran and produced nothing must not move it, and
/// an association must name patterns by the same id the pattern rows carry.
#[test]
fn cycles_and_patterns_tell_produced_from_ran() {
    let home = sandbox();
    let dreams = home.path().join(".claude/subconscious/dreams");
    std::fs::create_dir_all(&dreams).unwrap();
    let now = chrono::Utc::now();
    let t1 = (now - chrono::Duration::hours(30)).to_rfc3339();
    let t2 = (now - chrono::Duration::hours(3)).to_rfc3339();
    let journal = format!(
        "{}\n{}\n",
        serde_json::json!({"id":"a","timestamp":t1,"phase":"all","sessions_analyzed":4,"patterns_extracted":2,"associations_found":1,"insights_promoted":0,"tokens_used":900}),
        serde_json::json!({"id":"b","timestamp":t2,"phase":"all","sessions_analyzed":0,"patterns_extracted":0,"associations_found":0,"insights_promoted":0,"tokens_used":0}),
    );
    std::fs::write(dreams.join("journal.jsonl"), journal).unwrap();
    let pat = |text: &str, s: f64| serde_json::json!({"id":"x","pattern":text,"valence":"negative","confidence":0.9,"category":"approach","source_sessions":[],"occurrences":2,"first_seen":t1,"last_seen":t1,"occurrence_history":[t1],"strength":s});
    std::fs::write(
        dreams.join("patterns.json"),
        serde_json::json!([pat("Run it before calling it done.", 0.7), pat("Read the siblings first.", 0.2)]).to_string(),
    )
    .unwrap();
    let v = run_json(home.path(), &["status", "--json"]);
    let ids: Vec<String> = v["patterns"]["top"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect();
    std::fs::write(
        dreams.join("associations.json"),
        serde_json::json!([{"id":"as1","patterns_linked":["x","y"],"hypothesis":"h","confidence":0.8,"actionable":false,"suggested_rule":null,"patterns_linked_stable":[ids[0], ids[1]]}]).to_string(),
    )
    .unwrap();
    let v = run_json(home.path(), &["status", "--json"]);
    let recent = v["cycles"]["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0]["produced"], true);
    assert_eq!(recent[1]["produced"], false);
    let lp = chrono::DateTime::parse_from_rfc3339(v["cycles"]["last_productive"].as_str().unwrap()).unwrap();
    assert!((lp.with_timezone(&chrono::Utc) - now).num_hours().abs() >= 29, "an idle cycle moved last_productive");
    assert_eq!(v["patterns"]["total"], 2);
    assert_eq!(v["patterns"]["top"][0]["last7"], 1, "a 30h-old occurrence counts in the last seven days");
    let a = &v["patterns"]["associations"][0];
    assert!(ids.contains(&a["a"].as_str().unwrap().to_string()));
    assert!(ids.contains(&a["b"].as_str().unwrap().to_string()));
}
