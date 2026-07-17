//! `i-dream domain <subcommand>` — first user-visible surface of the
//! dream-domain plugin system (docs/14-dreaming-plugins.md). Ships
//! `list` (enriched with per-domain pending/last-pass/insights),
//! `enable`, and `disable`; further subcommands (info, install, run,
//! …) land with Stage 2+.

use crate::cli::DomainAction;
use crate::config::Config;
use crate::idream_runtime::IDreamRuntime;
use crate::modules::registry::DomainRegistry;
use crate::store::Store;
use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Serialize)]
struct DomainListEntry {
    name: String,
    kind: &'static str,
    description: String,
    /// Cadence sourced from the manifest's `[consolidation].cadence` field.
    /// Native modules carry the synthetic `"manifest"` placeholder until
    /// per-domain runtime overrides land (Stage 2 of docs/16).
    cadence: String,
    /// Events sitting past this domain's cursor right now — what the next
    /// dream-pass would consume. Always 0 for natives (they dream in the
    /// nightly cycle, not the per-domain pass). None means the delta read
    /// FAILED (broken stream/extractor) — rendered as "?", never as a
    /// reassuring 0.
    pending: Option<usize>,
    /// Timestamp the domain's cursor last advanced through (None = never
    /// consumed).
    last_pass: Option<chrono::DateTime<chrono::Utc>>,
    /// Lines in the domain's dream/insights.jsonl (None = no insights
    /// store declared or not yet created).
    insights: Option<usize>,
}

pub fn handle(action: DomainAction, config: &Config) -> Result<()> {
    match action {
        DomainAction::List { json } => list(config, json),
        DomainAction::Enable { name } => set_enabled(&name, true, config),
        DomainAction::Disable { name } => set_enabled(&name, false, config),
    }
}

fn set_enabled(name: &str, enabled: bool, config: &Config) -> Result<()> {
    // Reject the action if `name` doesn't refer to a registered external
    // domain. Native modules have their own enable surface; refusing here
    // avoids silently writing meaningless state.
    let store = Store::new(config.data_dir())?;
    let registry = DomainRegistry::boot(config, &store);
    let Some(domain) = registry.get(name) else {
        // Could be an external that was previously disabled — check both
        // the runtime state and the manifest discovery dirs before bailing.
        let rt = IDreamRuntime::load();
        if !rt.enabled.contains_key(name) {
            bail!("Unknown domain '{name}'. Use `i-dream domain list` to see registered domains.");
        }
        let mut rt = rt;
        rt.set_enabled(name, enabled);
        rt.save()?;
        let verb = if enabled { "enabled" } else { "disabled" };
        println!(
            "Domain '{name}' {verb} (was previously disabled — manifest re-discovered on next boot)."
        );
        return Ok(());
    };
    if domain.manifest().consolidation.kind == "native" {
        bail!(
            "Native modules ('{name}') have enable wired to config.modules.<name>.enabled; \
             use that instead. Only external domains support enable/disable via runtime state."
        );
    }
    let mut rt = IDreamRuntime::load();
    rt.set_enabled(name, enabled);
    rt.save()?;
    let verb = if enabled { "enabled" } else { "disabled" };
    println!("Domain '{name}' {verb}.");
    Ok(())
}

fn list(config: &Config, as_json: bool) -> Result<()> {
    let store = Store::new(config.data_dir())?;
    let registry = DomainRegistry::boot(config, &store);

    let entries: Vec<DomainListEntry> = registry
        .iter()
        .map(|d| {
            let m = d.manifest();
            let cursor = d.current_cursor().unwrap_or_default();
            // Same delta read the dream-pass makes — file reads, no LLM. A
            // failed read must not masquerade as "0 pending / all caught
            // up": surface it (mirrors dream_pass's warn on the same call).
            let pending = match d.delta(&cursor) {
                Ok(v) => Some(v.len()),
                Err(e) => {
                    tracing::warn!("domain '{}' delta read failed: {e:#}", d.name());
                    None
                }
            };
            let insights = m
                .dream
                .insights_path
                .as_ref()
                .map(|p| crate::modules::external_domain::expand_path(p))
                .filter(|p| p.exists())
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count());
            DomainListEntry {
                name: d.name().to_string(),
                kind: if m.consolidation.kind == "native" {
                    "native"
                } else {
                    "external"
                },
                description: m.domain.description.clone(),
                cadence: m.consolidation.cadence.clone(),
                pending,
                last_pass: cursor.last_ts,
                insights,
            }
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string(&entries)?);
    } else {
        if entries.is_empty() {
            println!("(no domains registered)");
            return Ok(());
        }
        println!(
            "{:<18} {:<10} {:<13} {:>7}  {:<10} {:>8}  DESCRIPTION",
            "NAME", "KIND", "CADENCE", "PENDING", "LAST PASS", "INSIGHTS"
        );
        let now = chrono::Utc::now();
        for e in &entries {
            let last_pass = match e.last_pass {
                // A cursor stamped in the future means clock skew or a
                // hand-edited file — look suspicious, not reassuring.
                Some(ts) if ts > now => "future ts?".to_string(),
                Some(ts) => {
                    let age = (now - ts).to_std().unwrap_or_default();
                    format!("{} ago", crate::modules::registry::fmt_age(age))
                }
                None => "never".to_string(),
            };
            let pending = e
                .pending
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".to_string());
            let insights = e
                .insights
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".to_string());
            println!(
                "{:<18} {:<10} {:<13} {:>7}  {:<10} {:>8}  {}",
                e.name, e.kind, e.cadence, pending, last_pass, insights, e.description
            );
        }
        // The pass itself is delta-driven; the cron fire is the only real
        // per-domain "next chance to run".
        if let Some(next) = crate::cron::JOBS
            .iter()
            .find(|j| j.label.ends_with("dreampass"))
            .and_then(|j| j.schedule.next_fire_after(chrono::Local::now()))
        {
            println!("\nnext dream-pass: {} (cron)", next.format("%Y-%m-%d %H:%M"));
        }
    }
    Ok(())
}
