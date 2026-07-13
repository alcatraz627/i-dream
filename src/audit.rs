//! L3 weekly audit — coordinator + approval flow + apply-time renderer.
//!
//! Architecture per docs/16-consolidation-build.md §3.6 + §3.10:
//!   1. Coordinator gathers inputs (7 daily digests + per-domain derived/
//!      + rejection fingerprints + current GCC content)
//!   2. Single LLM call structured as multi-sub-agent voices (the literal
//!      Agent-tool multi-dispatch is a V2 refinement; v1 ships single-call
//!      with the prompt asking the model to organize output by lens).
//!   3. Proposals filtered via rejection fingerprint (4-week TTL).
//!   4. Interactive terminal loop: [a]pprove / [r]eject / [s]kip
//!   5. Approved proposals → second LLM call renders the concrete edit →
//!      user confirms with [y]es / [c]ancel → Edit applied.
//!   6. Per-audit log lands at ~/.claude/i-dream/audits/YYYY-MM-DD.md;
//!      rejections appended to _rejections.jsonl.
//!
//! Aggressive dials per user direction:
//!   confidence floor 0.5 · max 6 proposals per lens · max 30 total

use crate::api::ClaudeClient;
use crate::cli::AuditAction;
use crate::config::Config;
use crate::consolidation::views::rank_matches;
use crate::modules::dreaming::{Association, ExtractedPattern};
use crate::modules::parse_json_codeblock;
use crate::store::Store;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const AUDIT_BUDGET_TOKENS: u32 = 8000;
/// How many times to re-sample the proposal LLM call when the response fails
/// to parse as JSON. Malformed output is stochastic at temp 0.4, so a retry
/// usually clears it.
const AUDIT_PARSE_ATTEMPTS: u32 = 3;
const RENDER_BUDGET_TOKENS: u32 = 3000;
const REJECTION_TTL_DAYS: i64 = 28;
const PROPOSAL_CONFIDENCE_FLOOR: f64 = 0.5;
const MAX_PROPOSALS_PER_LENS: usize = 6;
const MAX_PROPOSALS_TOTAL: usize = 30;
/// Similarity floor for tracing an applied proposal back to the patterns
/// that motivated it. Deliberately conservative: a false link reactivates an
/// unrelated pattern, while a missed link just leaves the up-vote unrecorded
/// (and says so out loud). Calibrated against the 2026-07-12 applied set —
/// right matches scored 0.09–0.15 in pattern space, wrong ones 0.06–0.086
/// (see the `graduation_match_probe` ignored test; association space had no
/// usable separation at all).
const GRADUATION_SIM_MIN: f64 = 0.09;
/// An applied proposal up-votes at most this many associations, so one broad
/// graduation can't blanket-reactivate half the store.
const GRADUATION_MAX_LINKS: usize = 3;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Proposal {
    // Descriptive fields: tolerate an LLM-emitted null by coercing to "".
    // A blank lens name or rationale still leaves a usable proposal.
    #[serde(default, deserialize_with = "null_to_empty")]
    sub_agent: String,
    // Load-bearing fields: a null here makes the proposal unfingerprintable
    // and unappliable, so we leave them strict — a null drops just this one
    // proposal during the resilient parse, rather than producing a junk edit.
    target_file: String,
    intent: String,
    #[serde(default, deserialize_with = "null_to_empty")]
    rationale: String,
    #[serde(default)]
    draft_diff: Option<String>,
    #[serde(default)]
    challenger_note: Option<String>,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn default_confidence() -> f64 {
    0.7
}

/// Treat an explicit JSON `null` (or a missing key) as an empty string.
/// The audit model occasionally emits `null` for a descriptive field; we'd
/// rather keep the proposal with that field blank than reject it.
fn null_to_empty<'de, D>(de: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(de)?.unwrap_or_default())
}

#[derive(Debug, Serialize, Deserialize)]
struct Rejection {
    fp: String,
    target: String,
    intent: String,
    reason: String,
    rejected_ts: String,
}

pub async fn handle(action: AuditAction, config: &Config) -> Result<()> {
    match action {
        AuditAction::Run {
            dry_run,
            week_days,
            non_interactive,
        } => run(config, dry_run, week_days, non_interactive).await,
        AuditAction::Status => status(),
    }
}

async fn run(config: &Config, dry_run: bool, week_days: u32, non_interactive: bool) -> Result<()> {
    let client = ClaudeClient::for_config(config)?;
    let inputs = gather_inputs(week_days as i64)?;

    println!("─── L3 weekly audit ──────────────────────────────────────────────");
    println!("  Window:           last {week_days} days");
    println!("  Daily digests:    {} read", inputs.dailies.len());
    println!("  Domain summaries: {}", inputs.domain_summaries.len());
    println!(
        "  Rejections active (within {REJECTION_TTL_DAYS}d): {}",
        inputs.active_rejections.len()
    );
    println!("  Dry run:          {dry_run}");
    println!();

    println!("→ Generating proposals (LLM, ~{AUDIT_BUDGET_TOKENS} tokens)...");
    let proposals = if dry_run {
        println!("  [dry-run: skipping LLM call]");
        vec![]
    } else {
        generate_proposals(&client, &config.budget.model, &inputs).await?
    };

    println!("  {} proposals returned by LLM", proposals.len());

    // Filter by rejection fingerprint.
    let filtered: Vec<Proposal> = proposals
        .into_iter()
        .filter(|p| {
            let fp = fingerprint(&p.target_file, &p.intent);
            let rejected = inputs.active_rejections.contains(&fp);
            if rejected {
                println!(
                    "  ⊘ filtered (recently rejected): {} → {}",
                    p.target_file, p.intent
                );
            }
            !rejected
        })
        .filter(|p| p.confidence >= PROPOSAL_CONFIDENCE_FLOOR)
        .take(MAX_PROPOSALS_TOTAL)
        .collect();

    if filtered.is_empty() {
        println!("\n  (no proposals to review this week)");
        write_audit_log(&inputs.audit_date, &filtered, &[], &[])?;
        return Ok(());
    }

    // Non-interactive (the weekly cron): stage proposals to the audit log and
    // exit without prompting or applying. The user reviews the log, then runs
    // `i-dream audit run` interactively to approve/apply.
    if non_interactive {
        write_audit_log(&inputs.audit_date, &filtered, &[], &[])?;
        // Flag that proposals await review so `i-dream review --if-pending`
        // (the Monday LaunchAgent) surfaces them; cleared when review opens.
        crate::review::mark_pending(&inputs.audit_date.to_string())?;
        let log = audit_dir()?.join(format!("{}.md", inputs.audit_date));
        println!(
            "\n  {} proposal(s) staged (non-interactive); nothing applied.",
            filtered.len()
        );
        println!("  Review: {}", log.display());
        println!("  Opens automatically Monday 09:00, or now:  i-dream review");
        return Ok(());
    }

    // Interactive approval loop.
    println!(
        "\n─── {} proposals to review ───────────────────────────────────────",
        filtered.len()
    );
    let mut approved: Vec<Proposal> = vec![];
    let mut rejected_this_run: Vec<(Proposal, String)> = vec![];

    for (idx, p) in filtered.iter().enumerate() {
        println!();
        println!(
            "[{}/{}] {} — {}",
            idx + 1,
            filtered.len(),
            p.sub_agent,
            p.target_file
        );
        println!("  Intent:      {}", p.intent);
        println!("  Confidence:  {:.2}", p.confidence);
        println!("  Rationale:   {}", p.rationale);
        if let Some(d) = &p.draft_diff {
            println!("  Draft diff:");
            for line in d.lines().take(8) {
                println!("    {line}");
            }
        }
        if let Some(c) = &p.challenger_note {
            println!("  Challenger:  {c}");
        }

        loop {
            print!("\n  [a]pprove  [r]eject  [s]kip  > ");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line)? == 0 {
                println!("\n  (stdin closed — abandoning audit)");
                return Ok(());
            }
            match line.trim() {
                "a" | "approve" => {
                    approved.push(p.clone());
                    println!("  ✓ approved");
                    break;
                }
                "r" | "reject" => {
                    print!("    reason (one line): ");
                    std::io::stdout().flush().ok();
                    let mut reason = String::new();
                    std::io::stdin().read_line(&mut reason)?;
                    let reason = reason.trim().to_string();
                    rejected_this_run.push((p.clone(), reason));
                    println!("  ✗ rejected (fingerprint remembered for {REJECTION_TTL_DAYS}d)");
                    break;
                }
                "s" | "skip" | "" => {
                    println!("  → skipped (no record)");
                    break;
                }
                other => {
                    println!("  unrecognized: '{other}' — use a / r / s");
                }
            }
        }
    }

    // Apply phase.
    let mut applied: Vec<(Proposal, String)> = vec![];
    if !approved.is_empty() {
        println!(
            "\n─── apply phase: rendering edits for {} approved ─────────────────",
            approved.len()
        );
        for (idx, p) in approved.iter().enumerate() {
            println!(
                "\n[{}/{}] Rendering: {} → {}",
                idx + 1,
                approved.len(),
                p.target_file,
                p.intent
            );

            let target_path = expand_path(&p.target_file);
            let current = match fs::read_to_string(&target_path) {
                Ok(c) => c,
                Err(e) => {
                    println!("  ⚠ cannot read target file ({e}); skipping");
                    continue;
                }
            };

            if dry_run {
                println!("  [dry-run: would render edit]");
                continue;
            }

            let edit = match render_edit(&client, &config.budget.model, p, &current).await {
                Ok(e) => e,
                Err(e) => {
                    println!("  ⚠ render failed: {e:#}; skipping");
                    continue;
                }
            };

            println!("  Proposed change:");
            println!("  ─────────────────────────────────────────────");
            for line in edit.preview.lines().take(20) {
                println!("    {line}");
            }
            println!("  ─────────────────────────────────────────────");

            print!("  [y]es apply  [c]ancel  > ");
            std::io::stdout().flush().ok();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            match line.trim() {
                "y" | "yes" | "" => {
                    if apply_edit(&target_path, &edit, &current).is_ok() {
                        println!("  ✓ applied");
                        applied.push((p.clone(), edit.preview.clone()));
                    } else {
                        println!("  ⚠ apply failed (file likely changed since render)");
                    }
                }
                _ => println!("  → cancelled (no Edit applied)"),
            }
        }
    }

    // Applying a graduation is the strongest up-vote the feedback lane gets;
    // trace each applied proposal to its source insights and record them.
    let inferred_ups = match record_graduation_upvotes(config, &applied) {
        Ok(n) => n,
        Err(e) => {
            println!("  ⚠ graduation up-vote recording failed: {e:#}");
            0
        }
    };

    // Persist rejections.
    if !rejected_this_run.is_empty() {
        append_rejections(&rejected_this_run)?;
    }

    write_audit_log(&inputs.audit_date, &filtered, &approved, &applied)?;

    // An interactive run completing IS the review — clear the pending flag so
    // the Monday LaunchAgent stops re-surfacing these (set by --non-interactive).
    let _ = crate::review::clear_pending();

    println!("\n─── audit complete ──────────────────────────────────────────────");
    println!("  Surfaced:  {}", filtered.len());
    println!("  Approved:  {}", approved.len());
    println!("  Applied:   {}", applied.len());
    println!("  Inferred ups: {inferred_ups}");
    println!("  Rejected:  {}", rejected_this_run.len());
    println!(
        "  Skipped:   {}",
        filtered.len() - approved.len() - rejected_this_run.len()
    );
    Ok(())
}

fn status() -> Result<()> {
    let dir = audit_dir()?;
    println!("Audit dir: {}", dir.display());
    if !dir.exists() {
        println!("  (no audits run yet)");
        return Ok(());
    }
    let mut audits: Vec<_> = fs::read_dir(&dir)?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?.to_string();
            if name.ends_with(".md") && !name.starts_with('_') {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    audits.sort();
    println!("Past audits: {}", audits.len());
    for a in audits.iter().rev().take(10) {
        println!("  {a}");
    }
    let rej_path = dir.join("_rejections.jsonl");
    if rej_path.exists() {
        let count = BufReader::new(fs::File::open(&rej_path)?).lines().count();
        println!("Active rejections (incl. expired): {count}");
    }
    Ok(())
}

// ── inputs gathering ────────────────────────────────────────────────────────

struct AuditInputs {
    audit_date: chrono::NaiveDate,
    dailies: Vec<String>,
    domain_summaries: Vec<DomainSummary>,
    active_rejections: HashSet<String>,
    /// Full active rejection records, fed to the proposal prompt grouped by
    /// target — so a reworded intent against a rejected target meets the
    /// recorded reasons instead of dodging the fingerprint filter
    /// (prop-20260709-232250-a1; the cli-gating thread resurfaced 4 times).
    rejection_history: Vec<Rejection>,
}

struct DomainSummary {
    name: String,
    tldr: String,
    insights_count: usize,
}

fn gather_inputs(week_days: i64) -> Result<AuditInputs> {
    let today = Local::now().naive_local().date();
    let cutoff = today - chrono::Duration::days(week_days);

    let mut dailies = vec![];
    let daily_dir = home().join(".claude/i-dream/daily");
    if let Ok(entries) = fs::read_dir(&daily_dir) {
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|s| s.to_str()) == Some("md")
                    && p.file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
            })
            .collect();
        paths.sort();
        for p in paths {
            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Ok(d) = chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d")
                && d >= cutoff
                && let Ok(content) = fs::read_to_string(&p)
            {
                dailies.push(content);
            }
        }
    }

    let mut domain_summaries = vec![];
    for root_name in [
        "atone",
        "affirm",
        "memory-domain",
        "sessions-domain",
        "pinned",
    ] {
        let root = home().join(format!(".claude/{root_name}"));
        let tldr_path = root.join("derived/_tldr.txt");
        let insights_path = root.join("dream/insights.jsonl");
        let tldr = fs::read_to_string(&tldr_path).unwrap_or_default();
        let insights_count = fs::File::open(&insights_path)
            .map(|f| BufReader::new(f).lines().count())
            .unwrap_or(0);
        if !tldr.trim().is_empty() || insights_count > 0 {
            domain_summaries.push(DomainSummary {
                name: root_name.to_string(),
                tldr: tldr.lines().take(8).collect::<Vec<_>>().join("\n"),
                insights_count,
            });
        }
    }

    let (active_rejections, rejection_history) = load_active_rejections()?;

    Ok(AuditInputs {
        audit_date: today,
        dailies,
        domain_summaries,
        active_rejections,
        rejection_history,
    })
}

/// Load the rejection ledger: fingerprints for the hard filter plus the full
/// active records for the prompt's per-target history. Expired lines (past
/// REJECTION_TTL_DAYS) are pruned from the file on the way through — the TTL
/// previously existed only in this read path, so the ledger grew forever
/// (census 2026-07-12, unpaired-writes table).
fn load_active_rejections() -> Result<(HashSet<String>, Vec<Rejection>)> {
    let path = audit_dir()?.join("_rejections.jsonl");
    let mut set = HashSet::new();
    let mut records: Vec<Rejection> = vec![];
    if !path.exists() {
        return Ok((set, records));
    }
    let cutoff = Utc::now() - chrono::Duration::days(REJECTION_TTL_DAYS);
    let mut kept_lines: Vec<String> = vec![];
    let mut expired_lines: Vec<String> = vec![];
    let f = fs::File::open(&path)?;
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(r) = serde_json::from_str::<Rejection>(line) else {
            // Pruning must never eat lines it doesn't understand — keep them
            // on disk, just don't feed them to the filter.
            kept_lines.push(line.to_string());
            continue;
        };
        let Ok(ts) = DateTime::parse_from_rfc3339(&r.rejected_ts) else {
            kept_lines.push(line.to_string());
            continue;
        };
        if ts.with_timezone(&Utc) >= cutoff {
            set.insert(r.fp.clone());
            kept_lines.push(line.to_string());
            records.push(r);
        } else {
            expired_lines.push(line.to_string());
        }
    }
    if !expired_lines.is_empty() {
        // Archive-never-delete, matching the retention reaper's philosophy:
        // expired rejections carry the zombie-proposal lineage, so they move
        // to the audits _archived bucket instead of vanishing.
        let archive_dir = audit_dir()?.join("_archived");
        fs::create_dir_all(&archive_dir)?;
        let archive = archive_dir.join("rejections-expired.jsonl");
        let mut af = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&archive)?;
        for l in &expired_lines {
            writeln!(af, "{l}")?;
        }
        let tmp = path.with_extension("jsonl.tmp");
        fs::write(&tmp, kept_lines.join("\n") + "\n")?;
        fs::rename(&tmp, &path)?;
        println!(
            "  ({} expired rejection line(s) archived to _archived/rejections-expired.jsonl)",
            expired_lines.len()
        );
    }
    Ok((set, records))
}

// ── LLM calls ───────────────────────────────────────────────────────────────

async fn generate_proposals(
    client: &ClaudeClient,
    model: &str,
    inputs: &AuditInputs,
) -> Result<Vec<Proposal>> {
    let system = "You are i-dream's L3 weekly audit. Multiple analytical lenses \
                  vote on what should change in the user's global Claude config \
                  (~/.claude/CLAUDE.md and ~/.claude/rules/*.md). \
                  Output a JSON array of proposals, each tagged with sub_agent.";

    let prompt = format!(
        r#"You are an audit ensemble of several lenses. Generate proposals for changes
to the user's ~/.claude/ Global Claude Config (GCC) based on this week's signals.

# Lenses (treat each as a distinct sub-agent voice)

1. **atone-analyst** — read atone TLDR. Surface patterns ripe for graduation
   from mistake-patterns.md into rules/*.md or a hook.
2. **affirm-analyst** — read affirm TLDR. Surface affirmed behaviors worth
   promoting to standing rules.
3. **dreams-analyst** — read the daily digests. Surface cross-domain
   patterns the LLM found that warrant GCC encoding.
4. **gcc-fitness-scorer** — propose structural GCC improvements (sections
   that have grown too big, duplicate content, etc.).
5. **graduation-curator** — propose specific slug→rule promotions when 3+
   atone or affirm events agree.
6. **abandoned-threads** — flag pinned insights or daily-digest topics that
   have appeared multiple weeks without action.
7. **challenger** — for any proposal another lens makes, write a one-line
   counter-argument (in `challenger_note`).

# Aggressive dial (per user direction)

- Confidence floor: 0.5 (surface, don't pre-filter; user filters via reject)
- Max {MAX_PROPOSALS_PER_LENS} proposals per lens
- Max {MAX_PROPOSALS_TOTAL} total

# Inputs

## Daily digests (last {n_days} days)

{dailies}

## Per-domain TLDRs

{domain_tldrs}

# Rejection memory (last {REJECTION_TTL_DAYS}d) — same-target proposals must overcome these

Proposals whose intent duplicates a listed rejection IN ANY WORDING are wasted
output: the exact-fingerprint filter catches verbatim repeats, and the reviewer
rejects rewordings on sight (the cli-gating thread was re-proposed 4 times this
way). Only propose against a listed target if you bring NEW evidence that
overcomes the recorded reason — and name that evidence explicitly in `rationale`.

{rejection_block}

# Output (strict JSON array)

```json
[
  {{
    "sub_agent": "atone-analyst",
    "target_file": "~/.claude/rules/testing.md",
    "intent": "Add render-before-judge as a numbered rule",
    "rationale": "3 atone events in 14 days; precheck is unambiguous; pattern is mature enough for a rule entry.",
    "draft_diff": "+ [render-before-judge] Don't call a value \"wrong\" based on number alone. Render it first.",
    "challenger_note": "Already informally covered by rules/testing.md line 47 (verify-each-change); risk of duplication.",
    "confidence": 0.7
  }}
]
```

Parseable JSON array only. No markdown fences. No preamble."#,
        n_days = inputs.dailies.len(),
        dailies = if inputs.dailies.is_empty() {
            "_(no daily digests in window)_".to_string()
        } else {
            inputs
                .dailies
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    format!(
                        "### Day {}\n{}\n",
                        i + 1,
                        d.chars().take(2000).collect::<String>()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n---\n")
        },
        domain_tldrs = if inputs.domain_summaries.is_empty() {
            "_(no domain summaries yet — run `i-dream dream-pass` first)_".to_string()
        } else {
            inputs
                .domain_summaries
                .iter()
                .map(|s| {
                    format!(
                        "### {}\n  insights: {}\n  tldr:\n{}\n",
                        s.name, s.insights_count, s.tldr
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
        rejection_block = if inputs.rejection_history.is_empty() {
            "_(no active rejections)_".to_string()
        } else {
            let mut by_target: std::collections::BTreeMap<&str, Vec<&Rejection>> =
                Default::default();
            for r in &inputs.rejection_history {
                by_target.entry(r.target.as_str()).or_default().push(r);
            }
            by_target
                .iter()
                .map(|(target, rs)| {
                    let mut s = format!("- `{}` — {} rejection(s):\n", target, rs.len());
                    for r in rs {
                        s.push_str(&format!(
                            "    - \"{}\" → {}\n",
                            r.intent.chars().take(120).collect::<String>(),
                            r.reason.chars().take(220).collect::<String>()
                        ));
                    }
                    s
                })
                .collect::<Vec<_>>()
                .join("")
        },
    );

    // The model occasionally emits syntactically broken JSON (an unescaped
    // quote inside a draft, a truncated tail). That's stochastic at temp 0.4,
    // so a fresh sample usually parses. Retry a few times; on every failure
    // dump the raw response so a persistent break is diagnosable rather than
    // silent.
    let mut last_err = None;
    for attempt in 1..=AUDIT_PARSE_ATTEMPTS {
        let response = client
            .analyze(system, &prompt, model, AUDIT_BUDGET_TOKENS, 0.4)
            .await
            .context("audit LLM call failed")?;

        let parsed = parse_json_codeblock(&response.content)
            .context("audit response had no parseable JSON")
            .and_then(|json_str| parse_proposals(&json_str));

        match parsed {
            Ok(proposals) => return Ok(proposals),
            Err(e) => {
                dump_raw_response(&response.content, attempt);
                eprintln!(
                    "  ⚠ proposal parse failed (attempt {attempt}/{AUDIT_PARSE_ATTEMPTS}): {e}"
                );
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("audit produced no parseable proposals")))
}

/// Write a failed audit response to disk so a persistent parse break can be
/// inspected. Best-effort — a dump failure must not mask the real parse error.
fn dump_raw_response(content: &str, attempt: u32) {
    if let Ok(dir) = audit_dir() {
        let path = dir.join(format!("_failed-response-attempt{attempt}.txt"));
        let _ = fs::write(&path, content);
        eprintln!("    raw response written to {}", path.display());
    }
}

/// Deserialize the LLM's proposal array resiliently.
///
/// A single malformed proposal — most often a `null` the model put in a
/// load-bearing field — must not discard the whole week's audit. We parse the
/// array element by element and skip the ones that don't deserialize, so a bad
/// proposal costs one proposal rather than the entire batch.
fn parse_proposals(json_str: &str) -> Result<Vec<Proposal>> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(json_str)
        .context("audit response was not a JSON array of proposals")?;
    let total = raw.len();
    let proposals: Vec<Proposal> = raw
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<Proposal>(v) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("  ⚠ skipping malformed proposal: {e}");
                None
            }
        })
        .collect();
    if proposals.len() < total {
        eprintln!(
            "  ⚠ {} of {total} proposals were malformed and skipped",
            total - proposals.len()
        );
    }
    Ok(proposals)
}

struct RenderedEdit {
    old_text: String,
    new_text: String,
    preview: String,
}

async fn render_edit(
    client: &ClaudeClient,
    model: &str,
    p: &Proposal,
    current_file: &str,
) -> Result<RenderedEdit> {
    let system = "You render concrete file edits from approved audit proposals. \
                  Output a single JSON object: {\"old_text\": str, \"new_text\": str}. \
                  old_text must be an EXACT substring of the current file (or empty for \
                  append-to-end). new_text is the replacement. Match the file's voice. \
                  Parseable JSON only.";
    let prompt = format!(
        r#"## Approved proposal
sub_agent: {sa}
target_file: {tf}
intent: {intent}
rationale: {rat}
draft_diff: {dd}

## Current file content ({tf})

{cur}

## Output JSON

Return ONE JSON object: {{"old_text": str, "new_text": str}}.
- old_text MUST be a verbatim substring of the current file content above,
  OR the empty string (to append at end).
- new_text is what old_text becomes.
- Keep the file's existing voice + indentation.
- If old_text is empty, new_text gets appended with a leading "\n\n".
"#,
        sa = p.sub_agent,
        tf = p.target_file,
        intent = p.intent,
        rat = p.rationale,
        dd = p.draft_diff.as_deref().unwrap_or("(none)"),
        cur = current_file.chars().take(8000).collect::<String>(),
    );

    let response = client
        .analyze(system, &prompt, model, RENDER_BUDGET_TOKENS, 0.3)
        .await?;
    let json_str =
        parse_json_codeblock(&response.content).context("render response had no JSON")?;
    let v: serde_json::Value = serde_json::from_str(&json_str)?;
    let old_text = v
        .get("old_text")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let new_text = v
        .get("new_text")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let preview = if old_text.is_empty() {
        format!(
            "(append to end)\n+ {}",
            new_text.lines().collect::<Vec<_>>().join("\n+ ")
        )
    } else {
        format!(
            "- {}\n+ {}",
            old_text.lines().collect::<Vec<_>>().join("\n- "),
            new_text.lines().collect::<Vec<_>>().join("\n+ ")
        )
    };

    Ok(RenderedEdit {
        old_text,
        new_text,
        preview,
    })
}

fn apply_edit(target: &Path, edit: &RenderedEdit, current: &str) -> Result<()> {
    let new_content = if edit.old_text.is_empty() {
        format!("{current}\n\n{}", edit.new_text)
    } else {
        if !current.contains(&edit.old_text) {
            bail!("old_text not found in current file (file may have changed)");
        }
        current.replacen(&edit.old_text, &edit.new_text, 1)
    };
    let tmp = target.with_extension("tmp");
    fs::write(&tmp, &new_content)?;
    fs::rename(&tmp, target)?;
    Ok(())
}

// ── graduation feedback ─────────────────────────────────────────────────────

/// Record each applied proposal as positive feedback on the dream insights
/// behind it. Shipping a rule from an insight is the strongest up-vote the
/// system ever gets, and the positive channel is otherwise starved (a handful
/// of ups against ~1200 downs), leaving reinforcement nothing to strengthen
/// (docs/25 item 16).
///
/// The link is recovered by deterministic text similarity against the
/// episodic pattern store — pattern texts are short behavioral lessons, the
/// same register as a proposal's intent, which is what makes the match
/// separable (association hypotheses are narrative prose and do not separate;
/// see `graduation_match_probe`). Asking the LLM to name ids instead would be
/// unverifiable. When no pattern clears the floor, no event is written and
/// that is said out loud rather than silently skipped.
fn record_graduation_upvotes(config: &Config, applied: &[(Proposal, String)]) -> Result<usize> {
    if applied.is_empty() {
        return Ok(0);
    }
    let store = Store::new(config.data_dir())?;
    let patterns: Vec<ExtractedPattern> = store
        .read_json("dreams/patterns.json")
        .unwrap_or_default();
    if patterns.is_empty() {
        println!("  (pattern store empty — no graduation up-votes recorded)");
        return Ok(0);
    }
    let corpus: Vec<&str> = patterns.iter().map(|p| p.pattern.as_str()).collect();

    let mut written = 0usize;
    for (p, _) in applied {
        let query = format!("{} {}", p.intent, p.rationale);
        let matches = rank_matches(&query, &corpus, GRADUATION_SIM_MIN);
        if matches.is_empty() {
            println!(
                "  ○ no pattern matched \"{}\" — no up-vote recorded",
                p.intent
            );
            continue;
        }
        for &(i, score) in matches.iter().take(GRADUATION_MAX_LINKS) {
            let pat = &patterns[i];
            store.append_jsonl(
                "dreams/insight-feedback.jsonl",
                &serde_json::json!({
                    "ts": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "pattern_id": pat.id,
                    "rating": "up",
                    "source": "graduation",
                    "match_score": (score * 1000.0).round() / 1000.0,
                    "proposal_intent": p.intent,
                }),
            )?;
            written += 1;
            let head: String = pat.pattern.chars().take(60).collect();
            println!(
                "  ▲ up-vote → {} ({score:.2}) {head}",
                &pat.id[..8.min(pat.id.len())]
            );
        }
    }
    Ok(written)
}

// ── persistence ─────────────────────────────────────────────────────────────

fn append_rejections(rejected: &[(Proposal, String)]) -> Result<()> {
    let dir = audit_dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("_rejections.jsonl");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let now = Utc::now().to_rfc3339();
    for (p, reason) in rejected {
        let r = Rejection {
            fp: fingerprint(&p.target_file, &p.intent),
            target: p.target_file.clone(),
            intent: p.intent.clone(),
            reason: reason.clone(),
            rejected_ts: now.clone(),
        };
        writeln!(f, "{}", serde_json::to_string(&r)?)?;
    }
    Ok(())
}

fn write_audit_log(
    date: &chrono::NaiveDate,
    surfaced: &[Proposal],
    approved: &[Proposal],
    applied: &[(Proposal, String)],
) -> Result<()> {
    let dir = audit_dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{date}.md"));
    let mut out = String::new();
    out.push_str(&format!("# {} — L3 weekly audit\n\n", date));
    out.push_str(&format!(
        "Surfaced: {} · Approved: {} · Applied: {}\n\n",
        surfaced.len(),
        approved.len(),
        applied.len()
    ));
    for (idx, p) in surfaced.iter().enumerate() {
        let status = if applied.iter().any(|(a, _)| a.intent == p.intent) {
            "✅ applied"
        } else if approved.iter().any(|a| a.intent == p.intent) {
            "✓ approved (not applied)"
        } else {
            "→ skipped or rejected"
        };
        out.push_str(&format!(
            "## Proposal {}/{} — {} _{}_\n\n",
            idx + 1,
            surfaced.len(),
            p.sub_agent,
            status
        ));
        out.push_str(&format!("- Target:     `{}`\n", p.target_file));
        out.push_str(&format!("- Intent:     {}\n", p.intent));
        out.push_str(&format!("- Confidence: {:.2}\n", p.confidence));
        out.push_str(&format!("- Rationale:  {}\n", p.rationale));
        if let Some(d) = &p.draft_diff {
            out.push_str(&format!("- Draft:\n```\n{d}\n```\n"));
        }
        if let Some(c) = &p.challenger_note {
            out.push_str(&format!("- Challenger: {c}\n"));
        }
        out.push('\n');
    }
    fs::write(&path, &out)?;
    Ok(())
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn fingerprint(target: &str, intent: &str) -> String {
    let normalized_intent = intent
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut hasher = Sha256::new();
    hasher.update(expand_path(target).to_string_lossy().as_bytes());
    hasher.update(b"\n");
    hasher.update(normalized_intent.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Expand `~/` in an LLM-proposed target path via the shared
/// `config::expand_tilde` (single source of truth for tilde expansion).
fn expand_path(p: &str) -> PathBuf {
    crate::config::expand_tilde(Path::new(p))
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn audit_dir() -> Result<PathBuf> {
    Ok(home().join(".claude/i-dream/audits"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proposals_coerces_null_descriptive_fields() {
        // sub_agent + rationale are null — descriptive, so the proposal is kept
        // with those fields blank rather than dropped.
        let json = r#"[
            {"sub_agent": null, "target_file": "~/.claude/rules/x.md",
             "intent": "do a thing", "rationale": null, "confidence": 0.8}
        ]"#;
        let got = parse_proposals(json).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].sub_agent, "");
        assert_eq!(got[0].rationale, "");
        assert_eq!(got[0].target_file, "~/.claude/rules/x.md");
    }

    #[test]
    fn parse_proposals_drops_only_the_malformed_one() {
        // The middle proposal has a null in a load-bearing field (target_file);
        // it must be dropped while the two valid proposals survive — this is the
        // exact failure that crashed the 2026-05-31 audit cron.
        let json = r#"[
            {"sub_agent": "a", "target_file": "f1", "intent": "i1", "rationale": "r1", "confidence": 0.7},
            {"sub_agent": "b", "target_file": null, "intent": "i2", "rationale": "r2", "confidence": 0.7},
            {"sub_agent": "c", "target_file": "f3", "intent": "i3", "rationale": "r3", "confidence": 0.7}
        ]"#;
        let got = parse_proposals(json).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].target_file, "f1");
        assert_eq!(got[1].target_file, "f3");
    }

    #[test]
    fn fingerprint_normalizes_whitespace_and_case() {
        let a = fingerprint("~/.claude/rules/testing.md", "Add Render-Before-Judge Rule");
        let b = fingerprint("~/.claude/rules/testing.md", "add render-before-judge rule");
        let c = fingerprint(
            "~/.claude/rules/testing.md",
            "ADD   render-before-judge   RULE",
        );
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn fingerprint_target_path_normalization() {
        // ~/.claude/X should equal $HOME/.claude/X
        let a = fingerprint("~/.claude/rules/testing.md", "x");
        let home = std::env::var("HOME").unwrap();
        let b = fingerprint(&format!("{home}/.claude/rules/testing.md"), "x");
        assert_eq!(a, b);
    }

    /// Calibration probe (read-only): score the proposals the 2026-07-12
    /// weekly review applied against the live association store, so
    /// GRADUATION_SIM_MIN is set from real score distributions instead of
    /// guessed. Prints the top 5 candidates per query with scores.
    /// Run: cargo test graduation_match_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn graduation_match_probe() {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let store = Store::new(home.join(".claude/subconscious")).unwrap();
        let associations: Vec<Association> =
            store.read_json("dreams/associations.json").unwrap();
        let corpus: Vec<&str> = associations.iter().map(|a| a.hypothesis.as_str()).collect();
        // The four proposals applied in the 2026-07-12 review
        // (audits/2026-07-12.md), paraphrased as intent+rationale queries.
        let applied_intents = [
            "Create rules/unprompted-infra-scope-creep.md — never add CI workflows, \
             git hooks, cron jobs, or automation infrastructure the user did not \
             explicitly request; a feasibility question is not a build order",
            "Add pushback face-3 affirm lineage to rules/pushback-and-self-criticism.md — \
             contradict a false premise with evidence before complying; \
             intelligent-disobedience affirmed across distinct contexts",
            "Add process-completion claims sub-section to \
             rules/structural-claim-without-reading-code.md — before writing that a \
             migration ran or a deploy succeeded, name the artifact that proves it",
            "declared-ready-stop.sh gap analysis — theme-state misses are Swift \
             surfaces exiting silent via the RAN branch before any multi-state \
             reminder fires",
        ];
        let patterns: Vec<crate::modules::dreaming::ExtractedPattern> =
            store.read_json("dreams/patterns.json").unwrap();
        let pattern_corpus: Vec<&str> = patterns.iter().map(|p| p.pattern.as_str()).collect();
        let schemas = crate::consolidation::schemas::load_schemas(&store);
        let schema_corpus: Vec<&str> = schemas.iter().map(|s| s.text.as_str()).collect();

        for q in applied_intents {
            let head: String = q.chars().take(60).collect();
            println!("\nQUERY: {head}…");
            println!(" vs associations ({}):", corpus.len());
            for (i, s) in rank_matches(q, &corpus, 0.0).iter().take(3) {
                let a = &associations[*i];
                let hyp: String = a.hypothesis.chars().take(70).collect();
                println!("  {s:.3}  {}  {hyp}", &a.id[..8]);
            }
            println!(" vs patterns ({}):", pattern_corpus.len());
            for (i, s) in rank_matches(q, &pattern_corpus, 0.0).iter().take(3) {
                let p = &patterns[*i];
                let txt: String = p.pattern.chars().take(70).collect();
                println!("  {s:.3}  {}  {txt}", &p.id[..8.min(p.id.len())]);
            }
            println!(" vs schemas ({}):", schema_corpus.len());
            for (i, s) in rank_matches(q, &schema_corpus, 0.0).iter().take(3) {
                let sc = &schemas[*i];
                let txt: String = sc.text.chars().take(70).collect();
                println!("  {s:.3}  {}  {txt}", &sc.id[..8.min(sc.id.len())]);
            }
        }
    }

    #[test]
    fn apply_edit_replaces_old_text_once() {
        let tmp = std::env::temp_dir().join("idream-audit-test.md");
        let original = "line A\nline B\nline A\nline C";
        fs::write(&tmp, original).unwrap();
        let edit = RenderedEdit {
            old_text: "line B".to_string(),
            new_text: "REPLACED".to_string(),
            preview: String::new(),
        };
        apply_edit(&tmp, &edit, original).unwrap();
        let got = fs::read_to_string(&tmp).unwrap();
        assert_eq!(got, "line A\nREPLACED\nline A\nline C");
        // Verify only ONE replacement (replacen with 1).
        let edit2 = RenderedEdit {
            old_text: "line A".to_string(),
            new_text: "X".to_string(),
            preview: String::new(),
        };
        apply_edit(&tmp, &edit2, &got).unwrap();
        let got2 = fs::read_to_string(&tmp).unwrap();
        assert_eq!(got2, "X\nREPLACED\nline A\nline C");
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn apply_edit_appends_when_old_text_empty() {
        let tmp = std::env::temp_dir().join("idream-audit-append.md");
        let original = "existing content";
        fs::write(&tmp, original).unwrap();
        let edit = RenderedEdit {
            old_text: String::new(),
            new_text: "appended".to_string(),
            preview: String::new(),
        };
        apply_edit(&tmp, &edit, original).unwrap();
        let got = fs::read_to_string(&tmp).unwrap();
        assert_eq!(got, "existing content\n\nappended");
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn apply_edit_fails_when_old_text_missing() {
        let tmp = std::env::temp_dir().join("idream-audit-missing.md");
        let original = "content";
        fs::write(&tmp, original).unwrap();
        let edit = RenderedEdit {
            old_text: "not in file".to_string(),
            new_text: "x".to_string(),
            preview: String::new(),
        };
        assert!(apply_edit(&tmp, &edit, original).is_err());
        let _ = fs::remove_file(&tmp);
    }
}
