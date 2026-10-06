//! Turning every signal stream into one shape of evidence row.
//!
//! Each domain writes its own event layout. The join only needs a handful of
//! keys, so this module reads every stream and keeps, per event: when it
//! happened, who made it (provenance), and whichever of slug, session,
//! project and file paths that stream really carries. The key table and the
//! provenance rules are docs/32 §2.

use crate::config::expand_tilde;
use crate::store::Store;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// One event from one stream, reduced to what the join can key on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    /// `<domain>:<event id>`, unique across all streams.
    pub id: String,
    pub domain: String,
    pub ts: DateTime<Utc>,
    /// Who made it: human, agent, residue-review, pin, seat, telemetry, echo.
    pub provenance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The project's leaf name ("i-dream"), so every stream's spelling meets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// A one-line excerpt a person can read.
    pub text: String,
    /// Weekly fire count, for hook telemetry rows only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fires_week: Option<u64>,
}

/// Provenance classes that copy another stream's event instead of witnessing
/// something new. They never count toward a repeat's independence.
pub fn is_echo(provenance: &str) -> bool {
    matches!(provenance, "echo")
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

fn strs(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn clip(text: &str, n: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= n {
        one_line
    } else {
        let cut: String = one_line.chars().take(n).collect();
        format!("{cut}…")
    }
}

fn parse_ts(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    match v.get(key)? {
        Value::String(t) => DateTime::parse_from_rfc3339(t)
            .map(|d| d.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f")
                    .ok()
                    .map(|n| DateTime::from_naive_utc_and_offset(n, Utc))
            }),
        Value::Number(n) => {
            let secs = n.as_i64()?;
            // Some streams write epoch seconds, a few epoch milliseconds.
            let secs = if secs > 10_000_000_000 { secs / 1000 } else { secs };
            DateTime::from_timestamp(secs, 0)
        }
        _ => None,
    }
}

/// The project's leaf name from any spelling: an absolute path, an encoded
/// Claude Code id, or a short name already.
pub fn project_leaf(raw: &str, cache: &mut HashMap<String, Option<String>>) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "global" {
        return None;
    }
    if let Some(hit) = cache.get(raw) {
        return hit.clone();
    }
    let leaf = if raw.starts_with('/') {
        Path::new(raw).file_name().map(|n| n.to_string_lossy().to_lowercase())
    } else if raw.starts_with('-') {
        crate::modules::project_briefs::ProjectBriefsModule::decode_project_id(raw)
            .and_then(|p| Path::new(&p).file_name().map(|n| n.to_string_lossy().to_lowercase()))
    } else {
        Some(raw.to_lowercase())
    };
    cache.insert(raw.to_string(), leaf.clone());
    leaf
}

/// Absolute file paths mentioned in free text.
fn paths_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || "\"'`()[]<>,".contains(c)) {
        let w = word.trim_end_matches(['.', ':', ';']);
        if (w.starts_with("/Users/") || w.starts_with("~/")) && w.len() > 3 && w.contains('.') {
            out.push(normalise_path(w));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// One spelling per file: `~` expanded, no trailing slash, no `:line` suffix.
pub fn normalise_path(p: &str) -> String {
    let p = p.split(':').next().unwrap_or(p);
    let p = if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string())
    } else {
        p.to_string()
    };
    p.trim_end_matches('/').to_string()
}

/// Provenance of a gcc proposal, from its `src:` and marker tags.
pub fn proposal_provenance(tags: &[String]) -> &'static str {
    let has = |t: &str| tags.iter().any(|x| x == t);
    if has("src:owner-ask") || has("src:owner-request") || has("src:user-request") {
        "human"
    } else if has("src:atone-graduation")
        || has("src:auto-stub")
        || has("auto-filed")
        || has("dream-derived")
        || has("src:idream-reader")
    {
        "echo"
    } else if has("residue-review") || has("src:residue-review") {
        "residue-review"
    } else {
        "agent"
    }
}

/// Normalise one raw event of a named domain. Returns None for an event with
/// no usable timestamp or nothing a person could read.
pub fn normalise(
    domain: &str,
    raw: &Value,
    ts_field: &str,
    id_field: &str,
    cache: &mut HashMap<String, Option<String>>,
) -> Option<Evidence> {
    let ts = parse_ts(raw, ts_field)?;
    let id = match raw.get(id_field)? {
        Value::String(x) => x.clone(),
        other => other.to_string(),
    };
    let mut e = Evidence {
        id: format!("{domain}:{id}"),
        domain: domain.to_string(),
        ts,
        provenance: "human".into(),
        slug: None,
        session: None,
        project: None,
        paths: vec![],
        text: String::new(),
        fires_week: None,
    };
    let project = |raw: &Value, key: &str, cache: &mut HashMap<String, Option<String>>| {
        s(raw, key).and_then(|p| project_leaf(&p, cache))
    };
    match domain {
        "atone" => {
            e.slug = s(raw, "slug");
            e.session = s(raw, "session_id");
            e.project = project(raw, "project", cache);
            e.paths = strs(raw, "files").iter().map(|p| normalise_path(p)).collect();
            let tags = strs(raw, "tags");
            if tags.iter().any(|t| t == "residue-review") {
                e.provenance = "residue-review".into();
            }
            e.text = clip(
                &format!("{}: {}", s(raw, "title").unwrap_or_default(), s(raw, "issue").unwrap_or_default()),
                200,
            );
        }
        "affirm" => {
            e.slug = s(raw, "slug");
            e.project = project(raw, "project", cache);
            e.paths = strs(raw, "files").iter().map(|p| normalise_path(p)).collect();
            e.text = clip(&s(raw, "title").unwrap_or_default(), 200);
        }
        "pinned" => {
            let from = raw.get("pinned_from").cloned().unwrap_or(Value::Null);
            e.session = s(&from, "session_id");
            e.project = s(&from, "cwd").and_then(|c| project_leaf(&c, cache));
            let text = s(raw, "text").unwrap_or_default();
            let context = s(raw, "context").unwrap_or_default();
            e.paths = paths_in(&format!("{text} {context}"));
            e.provenance = "pin".into();
            e.text = clip(&text, 200);
        }
        "memory-domain" => {
            e.project = project(raw, "project", cache);
            if let Some(p) = s(raw, "path").or_else(|| s(raw, "filepath")) {
                e.paths = vec![normalise_path(&p)];
            }
            e.text = clip(
                &s(raw, "description")
                    .or_else(|| s(raw, "title"))
                    .or_else(|| s(raw, "name"))
                    .unwrap_or_default(),
                200,
            );
        }
        "sessions-domain" => {
            e.session = s(raw, "session_id").or_else(|| Some(id.clone()));
            e.project = s(raw, "cwd")
                .and_then(|c| project_leaf(&c, cache))
                .or_else(|| project(raw, "project", cache));
            e.provenance = s(raw, "provenance").unwrap_or_else(|| "human".into());
            e.text = clip(&s(raw, "first_user_msg").unwrap_or_default(), 160);
        }
        "claude-audit" => {
            e.slug = s(raw, "slug");
            e.fires_week = raw.get("fires_week").and_then(|x| x.as_u64());
            e.provenance = match s(raw, "kind").as_deref() {
                Some("telemetry-pulse") => "telemetry".into(),
                _ => "agent".into(),
            };
            e.text = clip(&s(raw, "note").unwrap_or_default(), 200);
        }
        "codex-sessions" => {
            e.project = project(raw, "cwd", cache);
            e.provenance = "seat".into();
            e.text = clip(&s(raw, "first_prompt").unwrap_or_default(), 120);
        }
        "claude-ipc" => {
            e.provenance = "seat".into();
            e.text = clip(&s(raw, "body").unwrap_or_default(), 160);
        }
        "proposals" => {
            let tags = strs(raw, "tags");
            e.session = s(raw, "session_id");
            e.project = project(raw, "project", cache);
            e.slug = tags
                .iter()
                .find_map(|t| t.strip_prefix("link:atone:").map(str::to_string))
                .or_else(|| {
                    let title = s(raw, "title").unwrap_or_default();
                    title
                        .split("auto-stub: ")
                        .nth(1)
                        .map(|rest| rest.trim_end_matches(')').trim().to_string())
                        .filter(|x| !x.is_empty())
                });
            e.paths = tags
                .iter()
                .filter_map(|t| t.strip_prefix("target:"))
                .filter(|t| t.contains('/'))
                .map(normalise_path)
                .collect();
            e.provenance = proposal_provenance(&tags).into();
            e.text = clip(&s(raw, "title").unwrap_or_default(), 200);
        }
        "checkpoints" => {
            e.session = s(raw, "session_id");
            e.project = project(raw, "project", cache);
            e.provenance = s(raw, "provenance").unwrap_or_else(|| "human".into());
            let pending = s(raw, "pending").unwrap_or_default();
            e.text = clip(
                &if pending.is_empty() {
                    s(raw, "summary").unwrap_or_default()
                } else {
                    format!("pending: {pending}")
                },
                200,
            );
        }
        "skill-usage" => {
            e.session = s(raw, "session_id");
            e.provenance = s(raw, "provenance").unwrap_or_else(|| "agent".into());
            e.text = format!("/{}", s(raw, "skill").unwrap_or_default());
        }
        _ => {
            e.slug = s(raw, "slug");
            e.session = s(raw, "session_id");
            e.text = clip(
                &s(raw, "title")
                    .or_else(|| s(raw, "text"))
                    .or_else(|| s(raw, "note"))
                    .unwrap_or_default(),
                200,
            );
        }
    }
    if e.text.is_empty() && e.slug.is_none() {
        return None;
    }
    Some(e)
}

/// Per-domain read summary: how many events were in the window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamRead {
    pub domain: String,
    pub path: String,
    pub events_in_window: usize,
    pub newest: Option<DateTime<Utc>>,
}

/// Read every external domain's stream, plus i-dream's own correction
/// signals, keeping events newer than `since`.
pub fn load_all(store: &Store, since: DateTime<Utc>) -> (Vec<Evidence>, Vec<StreamRead>) {
    let mut cache = HashMap::new();
    let mut out = Vec::new();
    let mut reads = Vec::new();
    for m in crate::modules::registry::discover_external_manifests() {
        let name = m.domain.name.clone();
        let path = expand_tilde(Path::new(&m.event_stream.path));
        let mut read = StreamRead {
            domain: name.clone(),
            path: path.display().to_string(),
            ..Default::default()
        };
        if let Ok(body) = std::fs::read_to_string(&path) {
            for line in body.lines() {
                let Ok(raw) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let Some(e) = normalise(
                    &name,
                    &raw,
                    &m.event_stream.ts_field,
                    &m.event_stream.id_field,
                    &mut cache,
                ) else {
                    continue;
                };
                if e.ts < since {
                    continue;
                }
                read.events_in_window += 1;
                read.newest = read.newest.max(Some(e.ts));
                out.push(e);
            }
        }
        reads.push(read);
    }
    let signals = load_signals(store, since);
    reads.push(StreamRead {
        domain: "signals".into(),
        path: store.path("logs/signals.jsonl").display().to_string(),
        events_in_window: signals.len(),
        newest: signals.iter().map(|e| e.ts).max(),
    });
    out.extend(signals);
    out.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.id.cmp(&b.id)));
    out.dedup_by(|a, b| a.id == b.id);
    (out, reads)
}

/// Correction prompts the owner typed, from the daemon's own signals log.
/// Only rows with a session id can join anything.
fn load_signals(store: &Store, since: DateTime<Utc>) -> Vec<Evidence> {
    let rows: Vec<Value> = store.read_jsonl("logs/signals.jsonl").unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            let ev = r.get("event")?;
            if ev.get("correction").and_then(|c| c.as_bool()) != Some(true) {
                return None;
            }
            if ev
                .get("entrypoint")
                .and_then(|x| x.as_str())
                .is_some_and(|x| !crate::events::is_interactive_entrypoint(x))
            {
                return None;
            }
            let session = s(ev, "session_id")?;
            let ts = parse_ts(&r, "received_at")?;
            if ts < since {
                return None;
            }
            Some(Evidence {
                id: format!("signals:{}-{}", session, ts.timestamp()),
                domain: "signals".into(),
                ts,
                provenance: "human".into(),
                slug: None,
                session: Some(session),
                project: None,
                paths: vec![],
                text: "the owner corrected the agent".into(),
                fires_week: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(domain: &str, raw: Value) -> Evidence {
        normalise(domain, &raw, "ts", "id", &mut HashMap::new()).unwrap()
    }

    #[test]
    fn atone_keeps_slug_session_files_and_marks_residue_review() {
        let e = n("atone", json!({"id":"m1","ts":"2026-10-01T00:00:00Z","slug":"s","session_id":"abc",
            "project":"/Users/x/Code/i-dream","files":["~/a.rs"],"title":"t","issue":"i","tags":["residue-review"]}));
        assert_eq!(e.id, "atone:m1");
        assert_eq!(e.slug.as_deref(), Some("s"));
        assert_eq!(e.session.as_deref(), Some("abc"));
        assert_eq!(e.project.as_deref(), Some("i-dream"));
        assert_eq!(e.provenance, "residue-review");
        assert!(e.paths[0].ends_with("/a.rs") && !e.paths[0].starts_with('~'));
    }

    #[test]
    fn proposal_stubs_are_echoes_and_carry_their_slug() {
        let e = n("proposals", json!({"id":"p1","ts":"2026-10-01T00:00:00Z",
            "title":"Flesh out gcc friction (auto-stub: dense-briefing)","tags":["src:auto-stub"]}));
        assert_eq!(e.slug.as_deref(), Some("dense-briefing"));
        assert!(is_echo(&e.provenance));
        let human = n("proposals", json!({"id":"p2","ts":"2026-10-01T00:00:00Z","title":"x",
            "tags":["src:owner-ask","link:atone:a-slug"]}));
        assert_eq!(human.provenance, "human");
        assert_eq!(human.slug.as_deref(), Some("a-slug"));
    }

    #[test]
    fn pins_take_session_and_paths_from_nested_fields() {
        let e = n("pinned", json!({"id":"pin1","ts":"2026-10-01T00:00:00Z","text":"see /Users/x/a.rs:12 now",
            "pinned_from":{"session_id":"s9","cwd":"/Users/x/Code/proj"}}));
        assert_eq!(e.session.as_deref(), Some("s9"));
        assert_eq!(e.project.as_deref(), Some("proj"));
        assert_eq!(e.paths, vec!["/Users/x/a.rs".to_string()]);
        assert_eq!(e.provenance, "pin");
    }

    #[test]
    fn epoch_timestamps_parse_in_seconds_and_millis() {
        let a = n("claude-ipc", json!({"id":"m","ts":1791133835,"body":"b"}));
        let b = n("claude-ipc", json!({"id":"m","ts":1791133835000_i64,"body":"b"}));
        assert_eq!(a.ts, b.ts);
    }

    #[test]
    fn an_event_without_a_timestamp_is_dropped() {
        assert!(normalise("atone", &json!({"id":"x","slug":"s"}), "ts", "id", &mut HashMap::new()).is_none());
    }
}
