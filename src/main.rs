mod api;
mod audit;
mod board;
mod cli;
mod config;
mod consolidation;
mod cron;
mod daemon;
mod dashboard;
mod domain;
mod dream_trace;
mod events;
mod graph_metrics;
mod hooks;
mod idream_runtime;
mod logging;
mod modules;
mod pin;
mod reflect;
mod review;
mod service;
mod store;
mod thread;
mod transcript;
mod widget;

use anyhow::{Context, Result};
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

        Command::Dream {
            phase,
            backlog,
            modules: module_list,
        } => {
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
                println!(
                    "Backlog: reset processed state for {} module(s). Running cycle...",
                    targets.len()
                );
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

        Command::Domain { action } => {
            let config = config::Config::load(&cli.config)?;
            domain::handle(action, &config)?;
        }

        Command::Digest { day } => {
            use chrono::{Local, NaiveDate};
            let date = match day {
                Some(s) => NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                    .with_context(|| format!("--day '{s}' is not YYYY-MM-DD"))?,
                None => Local::now().naive_local().date(),
            };
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir())?;
            let path = consolidation::l2_digest::write_daily(date, &config, &store)?;
            let content = std::fs::read_to_string(&path)?;
            print!("{content}");
            eprintln!("\n[digest written: {}]", path.display());
        }

        Command::Cron { action } => {
            cron::handle(action)?;
        }

        Command::Pin { action } => {
            pin::handle(action)?;
        }

        Command::Thread { action } => {
            thread::handle(action)?;
        }

        Command::Board => {
            board::render()?;
        }

        Command::Reflect { json } => {
            if json {
                reflect::render_json()?;
            } else {
                reflect::render()?;
            }
        }

        Command::Review {
            if_pending,
            add_calendar,
        } => {
            review::handle(if_pending, add_calendar)?;
        }

        Command::Audit { action } => {
            let config = config::Config::load(&cli.config)?;
            audit::handle(action, &config).await?;
        }

        Command::DreamPass { budget } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir())?;
            let client = api::ClaudeClient::for_config(&config)?;
            let registry = modules::registry::DomainRegistry::boot(&config, &store);
            let report = consolidation::dream_pass::run_dream_pass(
                &registry,
                &client,
                &config.budget.model,
                budget,
            )
            .await?;
            // Views feed the digest + widget from the stores the pass just
            // updated — rebuild them in the same nightly slot.
            match consolidation::views::rebuild_views(&store) {
                Ok(paths) => {
                    for p in &paths {
                        eprintln!("[view rebuilt: {}]", p.display());
                    }
                }
                Err(e) => eprintln!("[view rebuild failed: {e:#}]"),
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
        }

        Command::Views => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir())?;
            let paths = consolidation::views::rebuild_views(&store)?;
            for p in &paths {
                println!("{}", p.display());
            }
        }

        Command::Contract { install } => {
            // Single source: the contract doc is embedded at compile time, so
            // `i-dream contract` and the installed CONTRACT.md can never drift.
            const CONTRACT: &str = include_str!("../docs/20-ingestion-contract.md");
            if install {
                let home = dirs::home_dir().context("could not resolve home dir")?;
                let dir = home.join(".claude/i-dream");
                std::fs::create_dir_all(&dir)?;
                let path = dir.join("CONTRACT.md");
                std::fs::write(&path, CONTRACT)?;
                eprintln!("[contract installed: {}]", path.display());
            } else {
                print!("{CONTRACT}");
            }
        }

        Command::Dashboard { no_open, run_tests } => {
            // If the menubar widget is running, signal it to open the native panel.
            if !no_open {
                let pgrep = std::process::Command::new("pgrep")
                    .args(["-x", "i-dream-bar"])
                    .output();
                if let Ok(out) = pgrep
                    && let Ok(pid_str) = std::str::from_utf8(&out.stdout)
                    && let Ok(pid) = pid_str.trim().parse::<u32>()
                {
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
                println!(
                    "✓ Brief written to {}\n  Tokens used: {tokens}",
                    path.display()
                );
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
                    println!("Skipping — already ran this ISO week. Use --force to regenerate.");
                }
            }
        }

        Command::Config => {
            let config = config::Config::load(&cli.config)?;
            println!("{}", toml::to_string_pretty(&config)?);
        }

        Command::SnapshotDiff {
            from,
            to,
            shift_threshold,
        } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;

            #[derive(serde::Deserialize)]
            struct Snap {
                ts: String,
                patterns: Vec<modules::dreaming::ExtractedPattern>,
                associations: Vec<modules::dreaming::Association>,
            }

            // Resolve a snapshot reference to a path. Bare timestamp →
            // dreams/snapshots/<ts>.json; full path → use as-is.
            let resolve = |s: &str| -> std::path::PathBuf {
                if std::path::Path::new(s).is_file() {
                    std::path::PathBuf::from(s)
                } else {
                    store.path(&format!("dreams/snapshots/{}.json", s))
                }
            };

            // If --from / --to omitted, list snapshots and pick the two
            // most recent (by filename, which is rfc3339-ish).
            let snaps_dir = store.path("dreams/snapshots");
            let mut all_snaps: Vec<std::path::PathBuf> = if snaps_dir.exists() {
                std::fs::read_dir(&snaps_dir)?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e == "json"))
                    .collect()
            } else {
                Vec::new()
            };
            all_snaps.sort();

            let from_path = match from {
                Some(s) => resolve(&s),
                None => {
                    if all_snaps.len() < 2 {
                        anyhow::bail!(
                            "Need ≥2 snapshots in dreams/snapshots/ (found {}). Run `i-dream graph-metrics --snapshot` to create one.",
                            all_snaps.len()
                        );
                    }
                    all_snaps[all_snaps.len() - 2].clone()
                }
            };
            let to_path = match to {
                Some(s) => resolve(&s),
                None => {
                    if all_snaps.is_empty() {
                        anyhow::bail!("No snapshots in dreams/snapshots/.");
                    }
                    all_snaps[all_snaps.len() - 1].clone()
                }
            };
            if !from_path.exists() {
                anyhow::bail!("Snapshot not found: {}", from_path.display());
            }
            if !to_path.exists() {
                anyhow::bail!("Snapshot not found: {}", to_path.display());
            }

            let from_snap: Snap = serde_json::from_slice(&std::fs::read(&from_path)?)?;
            let to_snap: Snap = serde_json::from_slice(&std::fs::read(&to_path)?)?;

            // Build id→pattern lookups for both sides.
            let from_p: std::collections::HashMap<&str, &modules::dreaming::ExtractedPattern> =
                from_snap
                    .patterns
                    .iter()
                    .map(|p| (p.id.as_str(), p))
                    .collect();
            let to_p: std::collections::HashMap<&str, &modules::dreaming::ExtractedPattern> =
                to_snap
                    .patterns
                    .iter()
                    .map(|p| (p.id.as_str(), p))
                    .collect();
            let from_a: std::collections::HashSet<&str> = from_snap
                .associations
                .iter()
                .map(|a| a.id.as_str())
                .collect();
            let to_a: std::collections::HashSet<&str> =
                to_snap.associations.iter().map(|a| a.id.as_str()).collect();

            let added: Vec<_> = to_p
                .iter()
                .filter(|(id, _)| !from_p.contains_key(*id))
                .collect();
            let removed: Vec<_> = from_p
                .iter()
                .filter(|(id, _)| !to_p.contains_key(*id))
                .collect();
            let mut shifted: Vec<(&str, f64, f64)> = Vec::new();
            for (id, p_to) in &to_p {
                if let Some(p_from) = from_p.get(id) {
                    let delta = p_to.confidence - p_from.confidence;
                    if delta.abs() >= shift_threshold {
                        shifted.push((id, p_from.confidence, p_to.confidence));
                    }
                }
            }
            shifted.sort_by(|a, b| (b.2 - b.1).abs().partial_cmp(&(a.2 - a.1).abs()).unwrap());

            let added_a = to_a.difference(&from_a).count();
            let removed_a = from_a.difference(&to_a).count();

            println!("Snapshot diff");
            println!(
                "  from: {} ({})",
                from_path.file_name().unwrap_or_default().to_string_lossy(),
                from_snap.ts
            );
            println!(
                "  to:   {} ({})",
                to_path.file_name().unwrap_or_default().to_string_lossy(),
                to_snap.ts
            );
            println!();
            println!(
                "Patterns: +{} added · -{} removed · {} shifted (≥{:.2} confidence delta)",
                added.len(),
                removed.len(),
                shifted.len(),
                shift_threshold
            );
            println!("Associations: +{} added · -{} removed", added_a, removed_a);
            if !added.is_empty() {
                println!("\n+ Added patterns:");
                for (id, p) in added.iter().take(10) {
                    let snip = p.pattern.chars().take(72).collect::<String>();
                    println!(
                        "    [{:.2}] {} — {} ({})",
                        p.confidence,
                        p.category,
                        snip,
                        &id[..8.min(id.len())]
                    );
                }
                if added.len() > 10 {
                    println!("    … +{} more", added.len() - 10);
                }
            }
            if !removed.is_empty() {
                println!("\n- Removed patterns:");
                for (id, p) in removed.iter().take(10) {
                    let snip = p.pattern.chars().take(72).collect::<String>();
                    println!(
                        "    [{:.2}] {} — {} ({})",
                        p.confidence,
                        p.category,
                        snip,
                        &id[..8.min(id.len())]
                    );
                }
                if removed.len() > 10 {
                    println!("    … +{} more", removed.len() - 10);
                }
            }
            if !shifted.is_empty() {
                println!("\n~ Shifted patterns (top 10 by |Δconfidence|):");
                for (id, c_from, c_to) in shifted.iter().take(10) {
                    let p = to_p.get(id).copied();
                    let cat = p.map(|p| p.category.as_str()).unwrap_or("?");
                    let snip = p
                        .map(|p| p.pattern.chars().take(60).collect::<String>())
                        .unwrap_or_default();
                    println!(
                        "    {:.2} → {:.2}  ({:+.2})  {} — {}",
                        c_from,
                        c_to,
                        c_to - c_from,
                        cat,
                        snip
                    );
                }
            }
        }

        Command::Drift { threshold, json } => {
            use chrono::{DateTime, Duration, Utc};
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;
            let patterns: Vec<modules::dreaming::ExtractedPattern> =
                store.read_json("dreams/patterns.json").unwrap_or_default();

            let now = Utc::now();
            let cutoff_recent = now - Duration::days(7);
            let cutoff_prior = now - Duration::days(14);

            // Group confidences by category for the two windows.
            let mut recent: std::collections::HashMap<&str, (f64, usize)> =
                std::collections::HashMap::new();
            let mut prior: std::collections::HashMap<&str, (f64, usize)> =
                std::collections::HashMap::new();
            for p in &patterns {
                let Ok(ts) = DateTime::parse_from_rfc3339(&p.last_seen) else {
                    continue;
                };
                let ts = ts.with_timezone(&Utc);
                let bucket: Option<&mut std::collections::HashMap<&str, (f64, usize)>> =
                    if ts >= cutoff_recent {
                        Some(&mut recent)
                    } else if ts >= cutoff_prior {
                        Some(&mut prior)
                    } else {
                        None
                    };
                if let Some(b) = bucket {
                    let e = b.entry(p.category.as_str()).or_insert((0.0, 0));
                    e.0 += p.confidence;
                    e.1 += 1;
                }
            }

            let mut drifts: Vec<(String, f64, f64, f64, usize, usize)> = Vec::new(); // cat, prior_avg, recent_avg, rel_drop, n_prior, n_recent
            for (cat, (sum_p, n_p)) in &prior {
                if *n_p < 3 {
                    continue;
                } // Sample-size floor — noisy below 3.
                let prior_avg = sum_p / *n_p as f64;
                let (sum_r, n_r) = recent.get(cat).copied().unwrap_or((0.0, 0));
                if n_r < 3 {
                    continue;
                }
                let recent_avg = sum_r / n_r as f64;
                let rel_drop = (prior_avg - recent_avg) / prior_avg.max(1e-9);
                if rel_drop >= threshold {
                    drifts.push((cat.to_string(), prior_avg, recent_avg, rel_drop, *n_p, n_r));
                }
            }
            drifts.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());

            if json {
                for (cat, prior_avg, recent_avg, rel_drop, np, nr) in &drifts {
                    let v = serde_json::json!({
                        "category": cat,
                        "prior_avg": prior_avg,
                        "recent_avg": recent_avg,
                        "relative_drop": rel_drop,
                        "n_prior": np, "n_recent": nr,
                    });
                    println!("{}", v);
                }
            } else if drifts.is_empty() {
                println!(
                    "No category-level drift detected (threshold: {:.0}% week-over-week drop, min sample size 3 per window).",
                    threshold * 100.0,
                );
            } else {
                println!(
                    "{} categor{} drifted ≥ {:.0}% week-over-week:",
                    drifts.len(),
                    if drifts.len() == 1 { "y" } else { "ies" },
                    threshold * 100.0,
                );
                for (cat, prior_avg, recent_avg, rel_drop, np, nr) in &drifts {
                    println!(
                        "  {:<20} {:.2} → {:.2}  ({:+.0}%)  n={}/{}",
                        cat,
                        prior_avg,
                        recent_avg,
                        -rel_drop * 100.0,
                        np,
                        nr,
                    );
                }
            }
        }

        Command::AutoIntentions {
            dry_run,
            min_confidence,
        } => {
            let config = config::Config::load(&cli.config)?;
            let store = store::Store::new(config.data_dir().clone())?;
            let mut associations: Vec<modules::dreaming::Association> = store
                .read_json("dreams/associations.json")
                .unwrap_or_default();
            let patterns: Vec<modules::dreaming::ExtractedPattern> =
                store.read_json("dreams/patterns.json").unwrap_or_default();
            let pm = modules::prospective::ProspectiveModule::new(&config, &store);
            let (created, skipped) = pm.auto_promote_associations(
                &mut associations,
                &patterns,
                min_confidence,
                dry_run,
            )?;
            // Persist mutated associations only if we actually wrote intentions.
            if !dry_run && created > 0 {
                store.write_json("dreams/associations.json", &associations)?;
            }
            println!(
                "{}D8 auto-intentions: {} created, {} skipped (threshold: confidence ≥ {:.2}, actionable ∧ promoted ∧ ¬dismissed ∧ ¬already-promoted).",
                if dry_run { "[dry-run] " } else { "" },
                created,
                skipped,
                min_confidence,
            );
        }

        Command::PrunePatterns {
            dry_run,
            max_confidence,
            days,
            restore,
        } => {
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
                if p.confidence >= max_confidence {
                    return false;
                }
                // Unparseable last_seen → treat as dormant (very old).
                let last_seen = DateTime::parse_from_rfc3339(&p.last_seen)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(cutoff - Duration::days(1));
                last_seen < cutoff
            });

            if to_prune.is_empty() {
                println!(
                    "Nothing to prune. {} patterns kept (threshold: confidence < {:.2} AND last_seen > {} days old).",
                    to_keep.len(),
                    max_confidence,
                    days,
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
                ("logs/events.jsonl", keep_events, "hook events"),
                ("metacog/activity.jsonl", keep_activity, "metacog activity"),
                ("logs/signals.jsonl", keep_signals, "signals"),
                ("dreams/journal.jsonl", keep_journal, "dream journal"),
            ];

            let mut total_removed = 0usize;
            for (path, keep, label) in &targets {
                let current = store.count_jsonl(path)?;
                let would_remove = current.saturating_sub(*keep);
                if dry_run {
                    println!("[dry-run] {label}: {current} entries → would remove {would_remove}");
                } else {
                    let removed = store.prune_jsonl(path, *keep)?;
                    println!(
                        "{label}: removed {removed} of {current} entries ({} remain)",
                        current - removed
                    );
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
