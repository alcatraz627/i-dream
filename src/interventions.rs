//! Compiled interventions — the felt-metabolism Phase 2 compiler (B1-B3).
//!
//! Phase 1 proved the efficacy gradient: lessons delivered as ambient prose
//! at session start do not change behavior; lessons delivered at the moment
//! of action (a hook nudge, a task-scoped note) do. This module is the
//! bridge: it lowers the highest-signal lessons — atone slugs with real
//! recurrence and a drafted precheck — into *intervention records* that the
//! repo's own hook scripts deliver at UserPromptSubmit or PreToolUse time.
//!
//! The compilation itself is an LLM judgment call (deriving a trigger
//! pattern from an English precheck mechanically would fabricate structure),
//! so an opus seat drafts each intervention and MECHANICAL validation gates
//! what it may say: allowlisted tools, shape caps, one intervention per
//! slug, deterministic ids. Everything born here starts in `shadow` state —
//! the interpreters log would-fires but inject nothing — and promotion to
//! `live` flows through the evidence bar and the owner's flip surface
//! (`i-dream promotions`), per the 2026-07-22 absence ladder: hints may
//! auto-promote on evidence, advisory nudges only after 2+ missed weekly
//! reviews, blocking tiers never (and no blocking form exists here at all).
//!
//! Store: `~/.claude/i-dream/interventions.json` (one small JSON array —
//! rewritten whole, client hooks read it with plain python). Would-fire
//! ledger: `~/.claude/i-dream/would-fire.jsonl` (appended by the hook
//! scripts, read here for the evidence bar).

use crate::api::ClaudeClient;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The model seat for compilation — same quality bar the owner set for the
/// smell panel ("we want quality, more of low quality outcome doesn't
/// help"). Production runs the subprocess CLI, which accepts the alias.
pub const COMPILE_MODEL: &str = "opus";

/// A slug qualifies for compilation once it has recurred this many times.
const QUALIFY_RECURRENCES: usize = 2;
/// A shadow hint may auto-promote once it would have fired this many times
/// across distinct sessions (the evidence bar; owner ladder 2026-07-22).
pub const EVIDENCE_BAR_FIRES: usize = 5;
/// Body length cap — an intervention is a nudge, not an essay.
const BODY_MAX: usize = 220;
/// Pattern length cap (consumer-side `re.compile` is the real validator;
/// this bounds obvious garbage).
const PATTERN_MAX: usize = 160;

/// Tools an intervention may scope to. Anything else the compiler emits is
/// dropped at validation.
const TOOL_ALLOWLIST: [&str; 7] = ["Bash", "Write", "Edit", "Read", "Glob", "Grep", "WebFetch"];

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Trigger {
    /// Project directory basename to scope to; None = all projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Case-insensitive regex matched against the user prompt
    /// (UserPromptSubmit surface).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_pattern: Option<String>,
    /// Tool name this fires on (PreToolUse surface).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Regex matched against the tool's command/file_path input
    /// (PreToolUse surface, requires `tool`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_pattern: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intervention {
    /// stable text-hash of slug+body — deterministic across compiles.
    pub id: String,
    pub slug: String,
    /// "atone-precheck" today; other sources later.
    pub source: String,
    /// "hint" (UserPromptSubmit, advisory) or "nudge" (PreToolUse, advisory).
    pub form: String,
    /// "shadow" → "candidate" → "live".
    pub state: String,
    pub trigger: Trigger,
    /// The injected text when live (≤BODY_MAX chars, imperative).
    pub body: String,
    pub created: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted: Option<DateTime<Utc>>,
    /// How it was promoted: "owner-flip" | "evidence-auto" (hints only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_by: Option<String>,
}

/// What one compile pass did — for the daemon log and the receipt.
#[derive(Debug, Default, Serialize)]
pub struct CompileReport {
    pub qualifying_slugs: usize,
    pub compiled_new: usize,
    pub rejected_by_validation: usize,
    pub already_covered: usize,
}

fn interventions_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    Ok(home.join(".claude/i-dream/interventions.json"))
}

fn would_fire_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    Ok(home.join(".claude/i-dream/would-fire.jsonl"))
}

pub fn load_interventions(path: &Path) -> Vec<Intervention> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_interventions(path: &Path, items: &[Intervention]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(items)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// One qualifying lesson, assembled from the atone ledger.
#[derive(Debug, Clone)]
pub struct QualifiedSlug {
    pub slug: String,
    pub recurrences: usize,
    pub severity: String,
    pub precheck: String,
    pub what_not: String,
}

/// Read the atone ledger (read-only) and collect slugs that have earned a
/// compiled intervention: recurrence ≥ bar AND a non-empty precheck. The
/// newest event's precheck wins (latest drafting is the most refined).
pub fn qualified_slugs(atone_events_path: &Path) -> Vec<QualifiedSlug> {
    let Ok(body) = std::fs::read_to_string(atone_events_path) else {
        return vec![];
    };
    let mut count: HashMap<String, usize> = HashMap::new();
    let mut latest: HashMap<String, (String, String, String)> = HashMap::new();
    for line in body.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(slug) = v.get("slug").and_then(|s| s.as_str()) else {
            continue;
        };
        if slug.is_empty() {
            continue;
        }
        *count.entry(slug.to_string()).or_default() += 1;
        let precheck = v
            .get("precheck")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !precheck.is_empty() {
            latest.insert(
                slug.to_string(),
                (
                    precheck,
                    v.get("severity").and_then(|s| s.as_str()).unwrap_or("").into(),
                    v.get("what_not_to_do")
                        .or_else(|| v.get("what_not"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .into(),
                ),
            );
        }
    }
    let mut out: Vec<QualifiedSlug> = count
        .into_iter()
        .filter(|(_, n)| *n >= QUALIFY_RECURRENCES)
        .filter_map(|(slug, n)| {
            latest.get(&slug).map(|(pre, sev, wn)| QualifiedSlug {
                slug: slug.clone(),
                recurrences: n,
                severity: sev.clone(),
                precheck: pre.clone(),
                what_not: wn.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.recurrences.cmp(&a.recurrences).then(a.slug.cmp(&b.slug)));
    out
}

/// Validate one LLM-drafted intervention. Mechanical only — the consumer
/// hooks re-validate patterns with `re.compile` at fire time (the check
/// that must hold lives at the point of use).
fn validate(v: &serde_json::Value) -> Option<(String, Trigger, String)> {
    let form = v.get("form")?.as_str()?;
    if form != "hint" && form != "nudge" {
        return None;
    }
    let body = v.get("body")?.as_str()?.trim();
    if body.is_empty() || body.len() > BODY_MAX || body.contains('\n') {
        return None;
    }
    let t = v.get("trigger")?;
    let get = |k: &str| {
        t.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= PATTERN_MAX && !s.contains('\n'))
            .map(str::to_string)
    };
    let trigger = Trigger {
        project: get("project"),
        prompt_pattern: get("prompt_pattern"),
        tool: get("tool"),
        input_pattern: get("input_pattern"),
    };
    // A nudge is tool-scoped by definition; its tool must be allowlisted.
    // A hint must carry a prompt_pattern (a hint with no trigger would fire
    // on every prompt — that is the ambient briefing again, not a nudge).
    match form {
        "nudge" => {
            let tool = trigger.tool.as_deref()?;
            if !TOOL_ALLOWLIST.contains(&tool) {
                return None;
            }
        }
        _ => {
            trigger.prompt_pattern.as_deref()?;
        }
    }
    Some((form.to_string(), trigger, body.to_string()))
}

/// Build the compiler prompt for a batch of qualifying slugs.
fn compile_prompt(batch: &[QualifiedSlug]) -> String {
    let mut p = String::from(
        "Compile each lesson below into AT MOST ONE intervention record, or \
         skip it if no conservative trigger exists. Output ONLY a JSON array \
         (no fences, no prose). Each element:\n\
         {\"slug\": \"...\", \"form\": \"hint\"|\"nudge\", \
         \"trigger\": {\"project\": null|\"dirname\", \
         \"prompt_pattern\": null|\"python-regex\", \
         \"tool\": null|\"Bash|Write|Edit|Read|Glob|Grep|WebFetch\", \
         \"input_pattern\": null|\"python-regex\"}, \
         \"body\": \"imperative, <=200 chars\"}\n\
         Rules: a nudge fires at tool time — set tool (+ input_pattern \
         matching the command or file path); a hint fires on the user's \
         prompt — set prompt_pattern. Patterns are case-insensitive python \
         regex; be CONSERVATIVE (a false fire costs trust; when in doubt, \
         narrower). body distills the precheck into one actionable line. \
         Skip lessons that are too abstract to trigger mechanically.\n\n",
    );
    for q in batch {
        p.push_str(&format!(
            "- slug: {}\n  recurrences: {} · severity: {}\n  precheck: {}\n  what-not: {}\n",
            q.slug, q.recurrences, q.severity, q.precheck, q.what_not
        ));
    }
    p
}

/// Parse + validate the compiler's response into interventions (shadow).
pub fn parse_compiled(
    response: &str,
    batch: &[QualifiedSlug],
    now: DateTime<Utc>,
) -> (Vec<Intervention>, usize) {
    let clean = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(clean) else {
        return (vec![], 0);
    };
    let known: HashSet<&str> = batch.iter().map(|q| q.slug.as_str()).collect();
    let mut out = Vec::new();
    let mut rejected = 0;
    let mut seen_slugs: HashSet<String> = HashSet::new();
    for v in arr {
        let Some(slug) = v.get("slug").and_then(|s| s.as_str()) else {
            rejected += 1;
            continue;
        };
        // The compiler may only speak about slugs it was asked about, once.
        if !known.contains(slug) || !seen_slugs.insert(slug.to_string()) {
            rejected += 1;
            continue;
        }
        match validate(&v) {
            Some((form, trigger, body)) => out.push(Intervention {
                id: crate::consolidation::views::stable_id(&format!("{slug}|{body}")),
                slug: slug.to_string(),
                source: "atone-precheck".into(),
                form,
                state: "shadow".into(),
                trigger,
                body,
                created: now,
                promoted: None,
                promoted_by: None,
            }),
            None => rejected += 1,
        }
    }
    (out, rejected)
}

/// Run one compile pass: qualify → skip already-covered slugs → one opus
/// call for the remainder → validate → merge into the store (never touching
/// existing records). Delta-driven: with nothing new to compile, no LLM
/// call happens at all.
pub async fn run_compile(client: &ClaudeClient) -> Result<CompileReport> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    let atone = home.join(".claude/atone/events.jsonl");
    let path = interventions_path()?;
    let mut existing = load_interventions(&path);
    let covered: HashSet<String> = existing.iter().map(|i| i.slug.clone()).collect();

    let qualified = qualified_slugs(&atone);
    let mut report = CompileReport {
        qualifying_slugs: qualified.len(),
        ..Default::default()
    };
    let batch: Vec<QualifiedSlug> = qualified
        .into_iter()
        .filter(|q| !covered.contains(&q.slug))
        .take(10) // bound one pass; the next cycle picks up the rest
        .collect();
    report.already_covered = report.qualifying_slugs - batch.len();
    if batch.is_empty() {
        return Ok(report);
    }

    let resp = client
        .analyze(
            "You compile behavioral lessons into precise, conservative \
             intervention triggers for a coding agent's hook system.",
            &compile_prompt(&batch),
            COMPILE_MODEL,
            2000,
            0.2,
        )
        .await?;
    let (new_items, rejected) = parse_compiled(&resp.content, &batch, Utc::now());
    report.compiled_new = new_items.len();
    report.rejected_by_validation = rejected;
    existing.extend(new_items);
    save_interventions(&path, &existing)?;
    Ok(report)
}

/// Would-fire counts per intervention id, from the hook-appended ledger,
/// counting DISTINCT sessions (five fires in one session is one datum).
pub fn would_fire_sessions(path: &Path) -> HashMap<String, HashSet<String>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    let Ok(body) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in body.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let (Some(id), Some(sid)) = (
            v.get("id").and_then(|x| x.as_str()),
            v.get("sid").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        out.entry(id.to_string()).or_default().insert(sid.to_string());
    }
    out
}

/// The promotion pass (owner ladder, 2026-07-22): shadow hints that met the
/// evidence bar become `candidate`; with `auto_hints` (absence path or the
/// owner's standing approval) qualifying HINTS go straight to live. Nudges
/// never auto-promote here — `auto_nudges` exists for the 2+-missed-reviews
/// path and is wired by the caller, not defaulted.
pub fn promote_on_evidence(
    items: &mut [Intervention],
    fires: &HashMap<String, HashSet<String>>,
    now: DateTime<Utc>,
    auto_hints: bool,
    auto_nudges: bool,
) -> usize {
    let mut changed = 0;
    for it in items.iter_mut() {
        if it.state == "live" {
            continue;
        }
        let n = fires.get(&it.id).map(|s| s.len()).unwrap_or(0);
        if n < EVIDENCE_BAR_FIRES {
            continue;
        }
        // An owner demotion is a standing veto: evidence may re-nominate the
        // item as candidate, but never auto-revive it — only an explicit
        // flip can (the flip surface outranks the ladder).
        let vetoed = it.promoted_by.as_deref() == Some("owner-demoted");
        let may_auto =
            !vetoed && ((it.form == "hint" && auto_hints) || (it.form == "nudge" && auto_nudges));
        if may_auto {
            it.state = "live".into();
            it.promoted = Some(now);
            it.promoted_by = Some("evidence-auto".into());
            changed += 1;
        } else if it.state == "shadow" {
            it.state = "candidate".into();
            changed += 1;
        }
    }
    changed
}

/// Owner flip: promote or demote one intervention by id (the
/// non-interactive surface `i-dream promotions --promote/--demote` uses).
pub fn flip(items: &mut [Intervention], id: &str, to_live: bool, now: DateTime<Utc>) -> bool {
    for it in items.iter_mut() {
        if it.id == id || it.id.starts_with(id) {
            if to_live {
                it.state = "live".into();
                it.promoted = Some(now);
                it.promoted_by = Some("owner-flip".into());
            } else {
                it.state = "shadow".into();
                it.promoted = None;
                // The latch promote_on_evidence honors: a demoted item never
                // auto-revives, whatever its evidence.
                it.promoted_by = Some("owner-demoted".into());
            }
            return true;
        }
    }
    false
}

/// Render the promotions table (plain text, non-interactive — minutes, not
/// a session).
pub fn render_promotions(items: &[Intervention], fires: &HashMap<String, HashSet<String>>) -> String {
    let mut out = String::from(
        "state      fires  form   id        slug / body\n\
         ─────────  ─────  ─────  ────────  ───────────\n",
    );
    let mut sorted: Vec<&Intervention> = items.iter().collect();
    sorted.sort_by_key(|i| {
        (
            match i.state.as_str() {
                "candidate" => 0,
                "live" => 1,
                _ => 2,
            },
            std::cmp::Reverse(fires.get(&i.id).map(|s| s.len()).unwrap_or(0)),
        )
    });
    for it in sorted {
        let n = fires.get(&it.id).map(|s| s.len()).unwrap_or(0);
        out.push_str(&format!(
            "{:<9}  {:>5}  {:<5}  {}  {} — {}\n",
            it.state,
            n,
            it.form,
            &it.id[..8.min(it.id.len())],
            it.slug,
            it.body
        ));
    }
    out.push_str(&format!(
        "\npromote: i-dream promotions --promote <id8> · demote: --demote <id8>\n\
         evidence bar: {EVIDENCE_BAR_FIRES} distinct sessions\n"
    ));
    out
}

/// Paths bundle for the CLI verb (production).
pub fn live_paths() -> Result<(PathBuf, PathBuf)> {
    Ok((interventions_path()?, would_fire_path()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 24, 12, 0, 0).unwrap()
    }

    fn write_atone(dir: &Path, lines: &[&str]) -> PathBuf {
        let p = dir.join("events.jsonl");
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    #[test]
    fn qualification_needs_recurrence_and_precheck() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_atone(
            dir.path(),
            &[
                r#"{"slug":"twice-with-precheck","severity":"S2","precheck":"Before X, check Y."}"#,
                r#"{"slug":"twice-with-precheck","severity":"S3","precheck":"Before X, check Y (refined)."}"#,
                r#"{"slug":"once-only","precheck":"Before Z."}"#,
                r#"{"slug":"twice-no-precheck"}"#,
                r#"{"slug":"twice-no-precheck"}"#,
            ],
        );
        let q = qualified_slugs(&p);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].slug, "twice-with-precheck");
        assert_eq!(q[0].recurrences, 2);
        assert!(q[0].precheck.contains("refined"), "newest precheck wins");
    }

    fn batch1() -> Vec<QualifiedSlug> {
        vec![QualifiedSlug {
            slug: "rg-replace-flag-mangles-evidence".into(),
            recurrences: 2,
            severity: "S2".into(),
            precheck: "Before rg with -r: replacing?".into(),
            what_not: "Never pass -r unless replacing.".into(),
        }]
    }

    #[test]
    fn parse_accepts_valid_nudge_and_rejects_garbage() {
        let resp = r#"```json
        [
          {"slug":"rg-replace-flag-mangles-evidence","form":"nudge",
           "trigger":{"tool":"Bash","input_pattern":"\\brg\\s+-[a-z]*r"},
           "body":"rg -r is --replace, not recursive; use rg -n alone."},
          {"slug":"rg-replace-flag-mangles-evidence","form":"nudge",
           "trigger":{"tool":"Bash"},"body":"duplicate slug must drop"},
          {"slug":"unknown-slug","form":"hint",
           "trigger":{"prompt_pattern":"x"},"body":"not asked about"},
          {"slug":"rg-replace-flag-mangles-evidence","form":"nudge",
           "trigger":{"tool":"EvilTool"},"body":"bad tool"}
        ]
        ```"#;
        let (items, rejected) = parse_compiled(resp, &batch1(), now());
        assert_eq!(items.len(), 1);
        assert_eq!(rejected, 3);
        assert_eq!(items[0].state, "shadow", "everything is born in shadow");
        assert_eq!(items[0].trigger.tool.as_deref(), Some("Bash"));
        // Deterministic id: same slug+body → same id on recompile.
        let (again, _) = parse_compiled(resp, &batch1(), now());
        assert_eq!(items[0].id, again[0].id);
    }

    #[test]
    fn hints_require_a_prompt_pattern() {
        let resp = r#"[{"slug":"rg-replace-flag-mangles-evidence","form":"hint",
            "trigger":{},"body":"untriggered hint = ambient briefing again"}]"#;
        let (items, rejected) = parse_compiled(resp, &batch1(), now());
        assert!(items.is_empty());
        assert_eq!(rejected, 1);
    }

    #[test]
    fn evidence_promotion_ladder_hints_auto_nudges_candidate() {
        let mk = |id: &str, form: &str| Intervention {
            id: id.into(),
            slug: format!("slug-{id}"),
            source: "atone-precheck".into(),
            form: form.into(),
            state: "shadow".into(),
            trigger: Trigger::default(),
            body: "b".into(),
            created: now(),
            promoted: None,
            promoted_by: None,
        };
        let mut items = vec![mk("hint-hot", "hint"), mk("nudge-hot", "nudge"), mk("cold", "hint")];
        let mut fires: HashMap<String, HashSet<String>> = HashMap::new();
        for id in ["hint-hot", "nudge-hot"] {
            fires.insert(
                id.into(),
                (0..EVIDENCE_BAR_FIRES).map(|i| format!("s{i}")).collect(),
            );
        }
        let changed = promote_on_evidence(&mut items, &fires, now(), true, false);
        assert_eq!(changed, 2);
        assert_eq!(items[0].state, "live", "hot hint auto-promotes");
        assert_eq!(items[0].promoted_by.as_deref(), Some("evidence-auto"));
        assert_eq!(items[1].state, "candidate", "hot nudge waits for the flip");
        assert_eq!(items[2].state, "shadow", "cold stays shadow");

        // The owner demotes the hot hint: the veto must hold against the
        // evidence pass forever, not for one cycle.
        assert!(flip(&mut items, "hint-hot", false, now()));
        let changed = promote_on_evidence(&mut items, &fires, now(), true, false);
        assert_eq!(
            items[0].state, "candidate",
            "vetoed item may be re-nominated but never auto-revived"
        );
        assert!(changed >= 1);
        let changed2 = promote_on_evidence(&mut items, &fires, now(), true, false);
        assert_eq!(changed2, 0, "steady state after veto");
        assert_eq!(items[0].state, "candidate");
        // Only an explicit flip revives it.
        assert!(flip(&mut items, "hint-hot", true, now()));
        assert_eq!(items[0].state, "live");
        assert_eq!(items[0].promoted_by.as_deref(), Some("owner-flip"));
    }

    #[test]
    fn would_fire_counts_distinct_sessions_and_flip_works() {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join("would-fire.jsonl");
        std::fs::write(
            &wf,
            r#"{"id":"abc","sid":"s1"}
{"id":"abc","sid":"s1"}
{"id":"abc","sid":"s2"}
garbage
{"sid":"s3"}
"#,
        )
        .unwrap();
        let fires = would_fire_sessions(&wf);
        assert_eq!(fires["abc"].len(), 2, "same-session repeats collapse");

        let mut items = vec![Intervention {
            id: "abcdef1234567890".into(),
            slug: "s".into(),
            source: "atone-precheck".into(),
            form: "nudge".into(),
            state: "candidate".into(),
            trigger: Trigger::default(),
            body: "b".into(),
            created: now(),
            promoted: None,
            promoted_by: None,
        }];
        assert!(flip(&mut items, "abcdef12", true, now()), "prefix flip");
        assert_eq!(items[0].state, "live");
        assert_eq!(items[0].promoted_by.as_deref(), Some("owner-flip"));
        assert!(flip(&mut items, "abcdef12", false, now()));
        assert_eq!(items[0].state, "shadow");
    }

    #[test]
    fn store_roundtrip_is_atomic_and_tolerant() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested/interventions.json");
        assert!(load_interventions(&p).is_empty(), "missing file = empty");
        let items = vec![Intervention {
            id: "x".into(),
            slug: "s".into(),
            source: "atone-precheck".into(),
            form: "hint".into(),
            state: "shadow".into(),
            trigger: Trigger {
                prompt_pattern: Some("deploy".into()),
                ..Default::default()
            },
            body: "b".into(),
            created: now(),
            promoted: None,
            promoted_by: None,
        }];
        save_interventions(&p, &items).unwrap();
        let back = load_interventions(&p);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].trigger.prompt_pattern.as_deref(), Some("deploy"));
        assert!(!p.with_extension("json.tmp").exists());
    }
}
