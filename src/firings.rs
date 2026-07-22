//! Firing detection (A1) — did an injected lesson visibly surface in the
//! session it was injected into?
//!
//! The injector logs which lesson ids entered which session
//! (`~/.claude/i-dream/injections.jsonl`) and, since the felt-metabolism
//! arc, renders each lesson with a `[L:xxxxxxxx]` tag (first 8 hex of its
//! stable_id). This scan joins those receipts against the session's own
//! transcript: an assistant message that carries the tag is a FIRING — the
//! agent surfaced the lesson while working — and becomes an honored
//! feedback event (`rating: up, source: fired`) that reinforce potentiates
//! through the stable-id path. Injected-but-never-echoed ids become
//! `source: present-unused` rows with NO rating: visible to the assay and
//! the receipt, deliberately invisible to every voting reader (reinforce,
//! confidence-apply, Brier all skip rating-less lines).
//!
//! Matching is the tag literal only. Fuzzy text-echo detection is deferred
//! on purpose: a zero-false-positive baseline that starts at zero when the
//! tag render ships is honest; a similarity heuristic that manufactures
//! firings is the SURFACED_LOG mistake with extra steps.

use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

/// A session is scanned once, after it has had time to finish.
const SETTLE_HOURS: i64 = 6;
/// A session whose transcript never appears is written off after this long.
const EXPIRY_DAYS: i64 = 7;
/// Scanned-session records older than this are dropped from the state file.
const STATE_RETAIN_DAYS: i64 = 90;

#[derive(Debug, Deserialize)]
struct InjectionRow {
    sid: String,
    #[serde(default)]
    ids: Vec<String>,
    #[serde(default)]
    ts: Option<DateTime<Utc>>,
}

/// Which sessions have been scanned (or written off), and when — plus which
/// ids each session already fired, so a re-scan after a resume can credit
/// late echoes exactly once (validation finding 2).
#[derive(Debug, Default, Serialize, Deserialize)]
struct FiringsState {
    #[serde(default)]
    scanned: HashMap<String, DateTime<Utc>>,
    #[serde(default)]
    fired: HashMap<String, Vec<String>>,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub sessions_scanned: u64,
    pub fired: u64,
    pub present_unused: u64,
    /// Sessions too fresh to scan yet, or transcript not found within expiry.
    pub pending: u64,
    /// Sessions written off — no transcript appeared within the expiry window.
    pub expired: u64,
}

/// The tag the injector renders and this scan matches.
fn tag_for(id: &str) -> String {
    format!("[L:{}]", &id[..id.len().min(8)])
}

/// Pull every assistant-authored text span out of a transcript. Injected
/// content lives in user/system rows, so matching only assistant text is
/// what makes a tag hit evidence of UPTAKE rather than an echo of the
/// injection itself.
fn assistant_text(transcript: &Path) -> Result<String> {
    let f = std::fs::File::open(transcript)
        .with_context(|| format!("opening transcript {}", transcript.display()))?;
    let mut out = String::new();
    for line in std::io::BufReader::new(f).lines() {
        let Ok(line) = line else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        if let Some(content) = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            for part in content {
                if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                    out.push_str(t);
                    out.push('\n');
                }
            }
        }
    }
    Ok(out)
}

/// Locate a session's transcript: `<projects_dir>/<project>/<sid>.jsonl`.
/// Direct path probes per project dir — no tree walk.
fn find_transcript(projects_dir: &Path, sid: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(projects_dir).ok()?;
    for e in rd.flatten() {
        let cand = e.path().join(format!("{sid}.jsonl"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Run one scan pass. All paths injectable; `store` is where feedback rows
/// land (`dreams/insight-feedback.jsonl`).
pub fn scan_at(
    injections_path: &Path,
    projects_dir: &Path,
    state_path: &Path,
    store: &Store,
    now: DateTime<Utc>,
) -> Result<ScanReport> {
    let mut state: FiringsState = std::fs::read_to_string(state_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut report = ScanReport::default();

    // Group injections by session: last injection time + the union of ids.
    let mut by_sid: HashMap<String, (Option<DateTime<Utc>>, Vec<String>)> = HashMap::new();
    let body = std::fs::read_to_string(injections_path).unwrap_or_default();
    for line in body.lines() {
        let Ok(row) = serde_json::from_str::<InjectionRow>(line) else {
            continue;
        };
        if row.sid.is_empty() || row.ids.is_empty() {
            continue;
        }
        let entry = by_sid.entry(row.sid).or_default();
        entry.0 = entry.0.max(row.ts);
        for id in row.ids {
            if !id.is_empty() && !entry.1.contains(&id) {
                entry.1.push(id);
            }
        }
    }

    for (sid, (last_ts, ids)) in by_sid {
        // An undated row can't be aged; treat it as fresh forever rather
        // than scanning a session that may still be running.
        let Some(last_ts) = last_ts else {
            if !state.scanned.contains_key(&sid) {
                report.pending += 1;
            }
            continue;
        };
        // A resumed session re-injects under the same sid. Scanning once and
        // marking forever would record its late echoes as present-unused
        // (validation finding 2) — so a scan repeats when injections newer
        // than the last scan exist, crediting only ids not already fired.
        let prior_scan = state.scanned.get(&sid).copied();
        if let Some(at) = prior_scan {
            if last_ts <= at {
                continue;
            }
        }
        if now - last_ts < Duration::hours(SETTLE_HOURS) {
            report.pending += 1;
            continue;
        }
        let Some(transcript) = find_transcript(projects_dir, &sid) else {
            if now - last_ts > Duration::days(EXPIRY_DAYS) {
                state.scanned.insert(sid, now);
                report.expired += 1;
            } else {
                report.pending += 1;
            }
            continue;
        };
        let text = assistant_text(&transcript).unwrap_or_default();
        let fired_before: Vec<String> = state.fired.get(&sid).cloned().unwrap_or_default();
        for id in &ids {
            if fired_before.contains(id) {
                continue; // credited on an earlier scan of this session
            }
            if text.contains(&tag_for(id)) {
                store.append_jsonl(
                    "dreams/insight-feedback.jsonl",
                    &serde_json::json!({
                        "insight_id": id, "rating": "up", "source": "fired",
                        "sid": sid, "ts": now.to_rfc3339(),
                    }),
                )?;
                state.fired.entry(sid.clone()).or_default().push(id.clone());
                report.fired += 1;
            } else if prior_scan.is_none() {
                // First scan only — a re-scan repeating these would duplicate
                // the row per resume. No rating on purpose: assay-visible,
                // vote-invisible.
                store.append_jsonl(
                    "dreams/insight-feedback.jsonl",
                    &serde_json::json!({
                        "insight_id": id, "source": "present-unused",
                        "sid": sid, "ts": now.to_rfc3339(),
                    }),
                )?;
                report.present_unused += 1;
            }
        }
        state.scanned.insert(sid, now);
        report.sessions_scanned += 1;
    }

    state
        .scanned
        .retain(|_, ts| now - *ts < Duration::days(STATE_RETAIN_DAYS));
    let FiringsState { scanned, fired } = &mut state;
    fired.retain(|sid, _| scanned.contains_key(sid));
    if let Some(dir) = state_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = state_path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&state)?)?;
    std::fs::rename(&tmp, state_path)?;
    Ok(report)
}

/// Production entrypoint: live injections + transcripts + store.
pub fn scan(store: &Store) -> Result<ScanReport> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    scan_at(
        &home.join(".claude/i-dream/injections.jsonl"),
        &home.join(".claude/projects"),
        &home.join(".claude/i-dream/derived/firings-state.json"),
        store,
        Utc::now(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 22, 12, 0, 0).unwrap()
    }

    struct Rig {
        _dir: tempfile::TempDir,
        injections: PathBuf,
        projects: PathBuf,
        state: PathBuf,
        store: Store,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(projects.join("-Users-x-proj")).unwrap();
        let store = Store::new(dir.path().join("subconscious")).unwrap();
        store.init_dirs().unwrap();
        Rig {
            injections: dir.path().join("injections.jsonl"),
            projects,
            state: dir.path().join("derived/firings-state.json"),
            store,
            _dir: dir,
        }
    }

    fn write_injection(rig: &Rig, sid: &str, ids: &[&str], ts: DateTime<Utc>) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&rig.injections)
            .unwrap();
        writeln!(
            f,
            "{}",
            serde_json::json!({"sid": sid, "ids": ids, "kind": "dream-ranked",
                "ts": ts.to_rfc3339(), "cwd_leaf": "proj"})
        )
        .unwrap();
    }

    fn write_transcript(rig: &Rig, sid: &str, assistant_texts: &[&str], user_texts: &[&str]) {
        let mut lines = Vec::new();
        for t in user_texts {
            lines.push(
                serde_json::json!({"type":"user","message":{"content":[{"type":"text","text":t}]}})
                    .to_string(),
            );
        }
        for t in assistant_texts {
            lines.push(
                serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":t}]}})
                    .to_string(),
            );
        }
        std::fs::write(
            rig.projects.join("-Users-x-proj").join(format!("{sid}.jsonl")),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }

    fn ledger(rig: &Rig) -> Vec<serde_json::Value> {
        std::fs::read_to_string(rig.store.path("dreams/insight-feedback.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn detects_fired_and_present_unused_and_is_idempotent() {
        let r = rig();
        let old = now() - Duration::hours(12);
        write_injection(&r, "sid-1", &["aabbccddeeff0011", "1122334455667788"], old);
        write_transcript(
            &r,
            "sid-1",
            &["Per the injected lesson [L:aabbccdd], auditing siblings first."],
            &["the injection itself carries [L:11223344] but user rows never count"],
        );

        let rep = scan_at(&r.injections, &r.projects, &r.state, &r.store, now()).unwrap();
        assert_eq!(rep.sessions_scanned, 1);
        assert_eq!(rep.fired, 1);
        assert_eq!(rep.present_unused, 1, "user-row echo is not uptake");

        let rows = ledger(&r);
        assert_eq!(rows.len(), 2);
        let fired = rows.iter().find(|v| v["source"] == "fired").unwrap();
        assert_eq!(fired["insight_id"], "aabbccddeeff0011");
        assert_eq!(fired["rating"], "up");
        let unused = rows.iter().find(|v| v["source"] == "present-unused").unwrap();
        assert!(unused.get("rating").is_none(), "vote-invisible by shape");

        // Second pass: session already scanned, nothing new lands.
        let rep2 = scan_at(&r.injections, &r.projects, &r.state, &r.store, now()).unwrap();
        assert_eq!(rep2.sessions_scanned, 0);
        assert_eq!(ledger(&r).len(), 2);
    }

    #[test]
    fn rescan_on_new_injection_credits_late_fires_exactly_once() {
        let r = rig();
        let t0 = now() - Duration::hours(12);
        write_injection(&r, "sid-r", &["aabbccddeeff0011"], t0);
        write_transcript(&r, "sid-r", &["no tag yet"], &[]);
        let rep1 = scan_at(&r.injections, &r.projects, &r.state, &r.store, now()).unwrap();
        assert_eq!((rep1.fired, rep1.present_unused), (0, 1));

        // The session resumes: a newer injection lands, and the transcript
        // now carries the echo.
        write_injection(&r, "sid-r", &["aabbccddeeff0011"], now() + Duration::hours(1));
        write_transcript(&r, "sid-r", &["acting on [L:aabbccdd] now"], &[]);
        let later = now() + Duration::hours(8);
        let rep2 = scan_at(&r.injections, &r.projects, &r.state, &r.store, later).unwrap();
        assert_eq!(rep2.fired, 1, "late echo credited on re-scan");
        assert_eq!(rep2.present_unused, 0, "unused rows never duplicate");

        // A third pass with nothing newer is a no-op; the fire stays single.
        let rep3 = scan_at(&r.injections, &r.projects, &r.state, &r.store, later).unwrap();
        assert_eq!((rep3.sessions_scanned, rep3.fired), (0, 0));
        let rows = ledger(&r);
        assert_eq!(rows.iter().filter(|v| v["source"] == "fired").count(), 1);
        assert_eq!(rows.len(), 2, "one unused + one fired, ever");
    }

    #[test]
    fn fresh_sessions_wait_for_settle() {
        let r = rig();
        write_injection(&r, "sid-2", &["aabbccddeeff0011"], now() - Duration::hours(1));
        write_transcript(&r, "sid-2", &["[L:aabbccdd]"], &[]);
        let rep = scan_at(&r.injections, &r.projects, &r.state, &r.store, now()).unwrap();
        assert_eq!(rep.pending, 1);
        assert_eq!(rep.sessions_scanned, 0);
        assert!(ledger(&r).is_empty());
    }

    #[test]
    fn missing_transcript_pends_then_expires() {
        let r = rig();
        write_injection(&r, "sid-3", &["aabbccddeeff0011"], now() - Duration::days(2));
        let rep = scan_at(&r.injections, &r.projects, &r.state, &r.store, now()).unwrap();
        assert_eq!(rep.pending, 1, "within expiry: keep waiting");

        write_injection(&r, "sid-4", &["1122334455667788"], now() - Duration::days(10));
        let rep = scan_at(&r.injections, &r.projects, &r.state, &r.store, now()).unwrap();
        assert_eq!(rep.expired, 1, "past expiry: written off");
        // The expired session never emits feedback rows.
        assert!(ledger(&r).is_empty());
        // And is not revisited.
        let rep2 = scan_at(&r.injections, &r.projects, &r.state, &r.store, now()).unwrap();
        assert_eq!(rep2.expired, 0);
    }
}
