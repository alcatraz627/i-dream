//! Exercises scripts/revert-autonomous.sh end-to-end against fixture ledgers.
//!
//! The 2026-07-13 adversarial gate found the script had zero automated
//! coverage: deleting its reinsert idempotence guard stayed green, the
//! documented default usage crashed on its second call, and a conflicting
//! live file was silently clobbered. Each test here pins one of the exact
//! input classes the validator exploited.

use std::path::Path;
use std::process::Output;

fn run(ledger: &Path, store_root: &Path, sel: Option<&str>) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/revert-autonomous.sh");
    let mut c = std::process::Command::new("bash");
    c.arg(script)
        .env("LEDGER_OVERRIDE", ledger)
        .env("STORE_ROOT_OVERRIDE", store_root);
    if let Some(s) = sel {
        c.arg(s);
    }
    c.output().expect("revert script should execute")
}

fn ledger_line(action: &str, target: &str, diff: &str, token: &str) -> String {
    serde_json::json!({
        "ts": "2026-07-13T00:00:00Z",
        "action": action,
        "target": target,
        "diff": diff,
        "revert_token": token,
        "source": "test-fixture",
    })
    .to_string()
        + "\n"
}

#[test]
fn reinsert_round_trips_then_noops_and_default_survives_meta_records() {
    let t = tempfile::tempdir().unwrap();
    let store = t.path().join("store");
    std::fs::create_dir_all(store.join("dreams")).unwrap();
    std::fs::write(store.join("dreams/patterns.json"), "[]").unwrap();
    let ledger = t.path().join("ledger.jsonl");
    std::fs::write(
        &ledger,
        ledger_line(
            "evict-pattern",
            "pat-x",
            "{\"id\":\"pat-x\"}",
            "reinsert:dreams/patterns.json",
        ),
    )
    .unwrap();

    let out = run(&ledger, &store, None);
    assert!(
        out.status.success(),
        "reinsert failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(store.join("dreams/patterns.json")).unwrap())
            .unwrap();
    assert_eq!(arr.len(), 1, "object reinserted exactly once");

    // Second default run: the ledger now ends with the first revert's own
    // meta-record. The selector must skip it (a bare `tail -1` crashed here),
    // and the reinsert must no-op instead of duplicating.
    let out2 = run(&ledger, &store, None);
    assert!(
        out2.status.success(),
        "2nd default run must not crash on the meta-record: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let arr2: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(store.join("dreams/patterns.json")).unwrap())
            .unwrap();
    assert_eq!(arr2.len(), 1, "idempotent: still exactly one copy");
}

#[test]
fn restore_round_trips_then_noops() {
    let t = tempfile::tempdir().unwrap();
    let queue = t.path().join("queue");
    let arch = queue.join("_processed/2026-07-13");
    std::fs::create_dir_all(&arch).unwrap();
    let archived = arch.join("cp.json");
    std::fs::write(&archived, "{\"x\":1}").unwrap();
    let target = queue.join("cp.json");
    let ledger = t.path().join("ledger.jsonl");
    std::fs::write(
        &ledger,
        ledger_line(
            "drain-checkpoint",
            &target.to_string_lossy(),
            "duplicate",
            &format!("restore:{}", archived.display()),
        ),
    )
    .unwrap();

    let out = run(&ledger, t.path(), Some("1"));
    assert!(
        out.status.success(),
        "restore failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(target.exists() && !archived.exists(), "moved back");

    // Same explicit line again: archived copy gone, target present. The gate
    // proved this exited 1 while the parent claimed double-revert idempotence.
    let out2 = run(&ledger, t.path(), Some("1"));
    assert!(
        out2.status.success(),
        "double restore must no-op: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    assert!(target.exists());
}

#[test]
fn restore_refuses_to_clobber_a_differing_live_file() {
    let t = tempfile::tempdir().unwrap();
    let queue = t.path().join("queue");
    let arch = queue.join("_processed/2026-07-13");
    std::fs::create_dir_all(&arch).unwrap();
    let archived = arch.join("cp.json");
    std::fs::write(&archived, "{\"x\":1}").unwrap();
    // Gate repro: new, unrelated content landed at the original path after the
    // archive; the old `mv -f` silently destroyed it.
    let target = queue.join("cp.json");
    std::fs::write(&target, "{\"unrelated\":true}").unwrap();
    let ledger = t.path().join("ledger.jsonl");
    std::fs::write(
        &ledger,
        ledger_line(
            "drain-checkpoint",
            &target.to_string_lossy(),
            "duplicate",
            &format!("restore:{}", archived.display()),
        ),
    )
    .unwrap();

    let out = run(&ledger, t.path(), Some("1"));
    assert_eq!(out.status.code(), Some(4), "refusal is its own exit code");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "{\"unrelated\":true}",
        "live file untouched"
    );
    assert!(archived.exists(), "archived copy untouched");
}

#[test]
fn restore_dir_moves_bucket_items_back_and_skips_occupied_paths() {
    let t = tempfile::tempdir().unwrap();
    let store = t.path().join("store");
    let bucket = store.join("_archived/2026-07-13");
    std::fs::create_dir_all(&bucket).unwrap();
    std::fs::write(bucket.join("a.json"), "A").unwrap();
    std::fs::write(bucket.join("b.json"), "B-archived").unwrap();
    // b.json's live path is occupied (the JSONL-overflow shape): never clobber.
    std::fs::write(store.join("b.json"), "B-live").unwrap();
    let ledger = t.path().join("ledger.jsonl");
    std::fs::write(
        &ledger,
        ledger_line(
            "retention-archive",
            "store",
            "2 item(s) archived",
            &format!("restore-dir:{}", bucket.display()),
        ),
    )
    .unwrap();

    let out = run(&ledger, t.path(), Some("1"));
    assert!(
        out.status.success(),
        "restore-dir failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(store.join("a.json")).unwrap(), "A");
    assert_eq!(
        std::fs::read_to_string(store.join("b.json")).unwrap(),
        "B-live",
        "occupied live path never clobbered"
    );
    assert!(bucket.join("b.json").exists(), "skipped item stays archived");
}

#[test]
fn meta_record_selected_explicitly_errors_cleanly() {
    let t = tempfile::tempdir().unwrap();
    let ledger = t.path().join("ledger.jsonl");
    std::fs::write(&ledger, ledger_line("revert", "x", "", "")).unwrap();

    let out = run(&ledger, t.path(), Some("1"));
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no revert token"),
        "names the problem instead of 'unknown revert token:'"
    );
}
