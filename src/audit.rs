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
use crate::modules::parse_json_codeblock;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const AUDIT_BUDGET_TOKENS: u32 = 8000;
const RENDER_BUDGET_TOKENS: u32 = 3000;
const REJECTION_TTL_DAYS: i64 = 28;
const PROPOSAL_CONFIDENCE_FLOOR: f64 = 0.5;
const MAX_PROPOSALS_PER_LENS: usize = 6;
const MAX_PROPOSALS_TOTAL: usize = 30;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Proposal {
    sub_agent: String,
    target_file: String,
    intent: String,
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
        AuditAction::Run { dry_run, week_days } => run(config, dry_run, week_days).await,
        AuditAction::Status => status(),
    }
}

async fn run(config: &Config, dry_run: bool, week_days: u32) -> Result<()> {
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

    // Persist rejections.
    if !rejected_this_run.is_empty() {
        append_rejections(&rejected_this_run)?;
    }

    write_audit_log(&inputs.audit_date, &filtered, &approved, &applied)?;

    println!("\n─── audit complete ──────────────────────────────────────────────");
    println!("  Surfaced:  {}", filtered.len());
    println!("  Approved:  {}", approved.len());
    println!("  Applied:   {}", applied.len());
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

    let active_rejections = load_active_rejections()?;

    Ok(AuditInputs {
        audit_date: today,
        dailies,
        domain_summaries,
        active_rejections,
    })
}

fn load_active_rejections() -> Result<HashSet<String>> {
    let path = audit_dir()?.join("_rejections.jsonl");
    let mut set = HashSet::new();
    if !path.exists() {
        return Ok(set);
    }
    let cutoff = Utc::now() - chrono::Duration::days(REJECTION_TTL_DAYS);
    let f = fs::File::open(&path)?;
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(r) = serde_json::from_str::<Rejection>(line) else {
            continue;
        };
        let Ok(ts) = DateTime::parse_from_rfc3339(&r.rejected_ts) else {
            continue;
        };
        if ts.with_timezone(&Utc) >= cutoff {
            set.insert(r.fp);
        }
    }
    Ok(set)
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
    );

    let response = client
        .analyze(system, &prompt, model, AUDIT_BUDGET_TOKENS, 0.4)
        .await
        .context("audit LLM call failed")?;

    let json_str =
        parse_json_codeblock(&response.content).context("audit response had no parseable JSON")?;
    let proposals: Vec<Proposal> = serde_json::from_str(&json_str)
        .context("audit response failed to parse as Vec<Proposal>")?;
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
