mod api;
mod cli;
mod config;
mod daemon;
mod dashboard;
mod dream_trace;
mod events;
mod graph_metrics;
mod hooks;
mod logging;
mod modules;
mod service;
mod store;
mod transcript;
mod widget;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file from two places, in order:
    //   1. CWD (developer convenience during `cargo run`)
    //   2. ~/.claude/subconscious/.env (the canonical location under
    //      launchd, where CWD is the daemon data dir anyway)
    // `dotenvy::dotenv()` is silent on missing files — nothing to check.
    let _ = dotenvy::dotenv();
    if let Some(home) = dirs::home_dir() {
        let _ = dotenvy::from_path(home.join(".claude/subconscious/.env"));
    }

    let cli = Cli::parse();

    // Initialize logging: stderr + daily-rotated file at
    // `~/.claude/subconscious/logs/i-dream.log`.
    //
    // The returned guard must stay alive for the duration of the
    // program — dropping it shuts down the non-blocking writer thread
    // and any buffered lines get discarded. We bind it here so it
    // lives until `main` returns.
    let log_level = cli.log_level.as_deref().unwrap_or("info");
    let _log_guard = logging::init(log_level)?;

    match cli.command {
        Command::Start { daemonize } => {
            info!("Starting i-dream daemon");
            let config = config::Config::load(&cli.config)?;
            let daemon = daemon::Daemon::new(config).await?;

            if daemonize {
                daemon.daemonize().await?;
            } else {
                daemon.run_foreground().await?;
            }
        }

        Command::Stop => {
            info!("Stopping i-dream daemon");
            daemon::Daemon::stop().await?;
        }

        Command::Status => {
            let status = daemon::Daemon::status().await?;
            println!("{status}");
        }

        Command::Dream { phase, backlog, modules: module_list } => {
            let config = config::Config::load(&cli.config)?;

            if backlog {
                let store = store::Store::new(config.data_dir())?;
                let targets = match &module_list {
                    Some(mods) if !mods.iter().any(|m| m == "all") => mods.clone(),
                    _ => vec![
                        "dreaming".to_string(),
                        "introspection".to_string(),
                        "metacog".to_string(),
                        "valence".to_string(),
                    ],
                };
                info!("Backlog mode: resetting processed state for {:?}", targets);
                for module in &targets {
                    let path = match module.as_str() {
                        "dreaming" | "dreams" => "dreams/processed.json",
                        "introspection" => "introspection/processed.json",
                        "metacog" => "metacog/processed.json",
                        "valence" | "intuition" => "valence/processed.json",
                        other => {
                            warn!("Unknown module for backlog: {other}, skipping");
                            continue;
                        }
                    };
                    let full_path = store.path(path);
                    if full_path.exists() {
                        // Back up the processed state before resetting
                        let backup = store.path(&format!("{path}.bak"));
                        std::fs::copy(&full_path, &backup)?;
                        // Write empty sessions map
                        store.write_json(path, &serde_json::json!({"sessions": {}}))?;
                        info!("Reset {path} (backup at {path}.bak)");
                    }
                }
                println!("Backlog: reset processed state for {} module(s). Running cycle...", targets.len());
            }

            info!("Running manual dream cycle (phase: {phase:?})");
            let daemon = daemon::Daemon::new(config).await?;
            daemon.run_dream(phase).await?;
        }

        Command::Inspect { module } => {
            let config = config::Config::load(&cli.config)?;
            let report = modules::inspect(&config, &module)?;
            println!("{report}");
        }

        Command::Hooks { action } => {
            let config = config::Config::load(&cli.config)?;
            hooks::manage(&config, action)?;
        }

        Command::Service { action } => {
            // Service management is a thin wrapper over `launchctl`; it
            // does not need the daemon config and should work even if
            // `config.toml` is missing (e.g. first-run `service install`).
            service::manage(action)?;
        }

        Command::Widget { action } => {
            widget::manage(action)?;
        }

        Command::Dashboard { no_open, run_tests } => {
            // If the menubar widget is running, signal it to open the native panel.
            if !no_open {
                let pgrep = std::process::Command::new("pgrep")
                    .args(["-x", "i-dream-bar"])
                    .output();
                if let Ok(out) = pgrep
                    && let Ok(pid_str) = std::str::from_utf8(&out.stdout)
                        && let Ok(pid) = pid_str.trim().parse::<u32>() {
                            let _ = std::process::Command::new("kill")
                                .args(["-USR1", &pid.to_string()])
                                .status();
                            println!("Opening native dashboard (sent SIGUSR1 to widget PID {pid})");
                            return Ok(());
                        }
            }
            // Fallback: generate and open HTML dashboard.
            let config = config::Config::load(&cli.config)?;
            let path = dashboard::generate(&config, run_tests)?;
            println!("Dashboard written to {}", path.display());
            if !no_open {
                dashboard::open_in_browser(&path)?;
            }
        }

        Command::GraphMetrics { snapshot } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;
            let metrics = graph_metrics::compute_and_persist(&store)?;
            println!(
                "✓ Wrote graph-metrics.json\n  patterns: {}\n  associations: {}\n  edges: {}\n  isolated patterns: {}\n  top hubs: {} (degree-sorted)",
                metrics.n_patterns,
                metrics.n_associations,
                metrics.n_edges,
                metrics.isolated_patterns,
                metrics.hubs.len(),
            );
            if snapshot {
                let path = graph_metrics::snapshot_for_diff(&store)?;
                println!("✓ Wrote snapshot {}", path.display());
            }
        }

        Command::BriefProjects { cwd } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;
            let client = api::ClaudeClient::for_config(&config)?;
            let pbm = modules::project_briefs::ProjectBriefsModule::new(&config, &store);
            if let Some(c) = cwd {
                let project_id = modules::project_briefs::ProjectBriefsModule::encode_cwd(&c);
                let (tokens, path) = pbm.generate_for_project(&client, &project_id).await?;
                println!("✓ Brief written to {}\n  Tokens used: {tokens}", path.display());
            } else {
                let (count, total_tokens) = pbm.generate_all(&client).await?;
                println!("✓ Generated briefs for {count} projects\n  Total tokens: {total_tokens}");
            }
        }

        Command::Briefing { force } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;
            let client = api::ClaudeClient::for_config(&config)?;
            let bm = modules::weekly_briefing::WeeklyBriefingModule::new(&config, &store);
            let result = if force {
                Some(bm.run_force(&client).await?)
            } else {
                bm.run(&client).await?
            };
            match result {
                Some((tokens, path)) => {
                    println!(
                        "✓ Weekly briefing written to {}\n  Tokens used: {tokens}",
                        path.display()
                    );
                }
                None => {
                    println!(
                        "Skipping — already ran this ISO week. Use --force to regenerate."
                    );
                }
            }
        }

        Command::Config => {
            let config = config::Config::load(&cli.config)?;
            println!("{}", toml::to_string_pretty(&config)?);
        }

        Command::AutoIntentions { dry_run, min_confidence } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;
            let mut associations: Vec<modules::dreaming::Association> =
                store.read_json("dreams/associations.json").unwrap_or_default();
            let patterns: Vec<modules::dreaming::ExtractedPattern> =
                store.read_json("dreams/patterns.json").unwrap_or_default();
            let pm = modules::prospective::ProspectiveModule::new(&config, &store);
            let (created, skipped) = pm.auto_promote_associations(
                &mut associations, &patterns, min_confidence, dry_run,
            )?;
            // Persist mutated associations only if we actually wrote intentions.
            if !dry_run && created > 0 {
                store.write_json("dreams/associations.json", &associations)?;
            }
            println!(
                "{}D8 auto-intentions: {} created, {} skipped (threshold: confidence ≥ {:.2}, actionable ∧ promoted ∧ ¬dismissed ∧ ¬already-promoted).",
                if dry_run { "[dry-run] " } else { "" },
                created, skipped, min_confidence,
            );
        }

        Command::PrunePatterns { dry_run, max_confidence, days, restore } => {
            use chrono::{DateTime, Duration, Utc};
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;

            // --restore: merge a backup file's patterns back into patterns.json.
            // No-op if any restored id already exists (idempotent rescue).
            if let Some(backup_id) = restore {
                let backup_path = if std::path::Path::new(&backup_id).is_file() {
                    std::path::PathBuf::from(&backup_id)
                } else {
                    store.path(&format!("dreams/pruned/{}.json", backup_id))
                };
                if !backup_path.exists() {
                    anyhow::bail!("Backup not found: {}", backup_path.display());
                }
                let backup_bytes = std::fs::read(&backup_path)?;
                let pruned: Vec<modules::dreaming::ExtractedPattern> =
                    serde_json::from_slice(&backup_bytes)?;
                let mut current: Vec<modules::dreaming::ExtractedPattern> =
                    store.read_json("dreams/patterns.json").unwrap_or_default();
                let existing_ids: std::collections::HashSet<String> =
                    current.iter().map(|p| p.id.clone()).collect();
                let mut restored = 0usize;
                for p in pruned {
                    if !existing_ids.contains(&p.id) {
                        current.push(p);
                        restored += 1;
                    }
                }
                if !dry_run {
                    store.write_json("dreams/patterns.json", &current)?;
                }
                println!(
                    "{}Restored {restored} pattern(s) from {}\n  Total now: {}",
                    if dry_run { "[dry-run] " } else { "" },
                    backup_path.display(),
                    current.len(),
                );
                return Ok(());
            }

            // Normal pruning path.
            let all: Vec<modules::dreaming::ExtractedPattern> =
                store.read_json("dreams/patterns.json").unwrap_or_default();
            let cutoff = Utc::now() - Duration::days(days);
            let (to_prune, to_keep): (Vec<_>, Vec<_>) = all.into_iter().partition(|p| {
                if p.confidence >= max_confidence { return false; }
                // Unparseable last_seen → treat as dormant (very old).
                let last_seen = DateTime::parse_from_rfc3339(&p.last_seen)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(cutoff - Duration::days(1));
                last_seen < cutoff
            });

            if to_prune.is_empty() {
                println!(
                    "Nothing to prune. {} patterns kept (threshold: confidence < {:.2} AND last_seen > {} days old).",
                    to_keep.len(), max_confidence, days,
                );
                return Ok(());
            }

            // Always write a backup before mutating patterns.json. Backup
            // path is timestamped so successive prunes don't overwrite.
            let stamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
            let backup_rel = format!("dreams/pruned/{}.json", stamp);
            let backup_path = store.path(&backup_rel);
            if !dry_run {
                if let Some(parent) = backup_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                store.write_json(&backup_rel, &to_prune)?;
                store.write_json("dreams/patterns.json", &to_keep)?;
            }

            println!(
                "{}Pruned {} pattern(s) (kept {}).\n  Backup: {}\n  Restore: i-dream prune-patterns --restore {}",
                if dry_run { "[dry-run] " } else { "" },
                to_prune.len(),
                to_keep.len(),
                backup_path.display(),
                stamp,
            );
            // Show the lowest-confidence prunees so the user can sanity-check.
            let mut preview: Vec<_> = to_prune.iter().collect();
            preview.sort_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap());
            for p in preview.iter().take(5) {
                let snippet = p.pattern.chars().take(72).collect::<String>();
                println!("    [{:.2}] {} — {}", p.confidence, p.category, snippet);
            }
            if preview.len() > 5 {
                println!("    … +{} more", preview.len() - 5);
            }
        }

        Command::Prune {
            dry_run,
            keep_events,
            keep_activity,
            keep_signals,
            keep_journal,
        } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;

            let targets = [
                ("logs/events.jsonl",       keep_events,   "hook events"),
                ("metacog/activity.jsonl",  keep_activity, "metacog activity"),
                ("logs/signals.jsonl",      keep_signals,  "signals"),
                ("dreams/journal.jsonl",    keep_journal,  "dream journal"),
            ];

            let mut total_removed = 0usize;
            for (path, keep, label) in &targets {
                let current = store.count_jsonl(path)?;
                let would_remove = current.saturating_sub(*keep);
                if dry_run {
                    println!("[dry-run] {label}: {current} entries → would remove {would_remove}");
                } else {
                    let removed = store.prune_jsonl(path, *keep)?;
                    println!("{label}: removed {removed} of {current} entries ({} remain)", current - removed);
                    total_removed += removed;
                }
            }

            if !dry_run {
                println!("\nTotal entries removed: {total_removed}");
            }
        }
    }

    Ok(())
}
