//! Daemon lifecycle — start, stop, status, idle detection, consolidation orchestration.

use crate::api::ClaudeClient;
use crate::cli::DreamPhase;
use crate::config::Config;
use crate::dream_trace::{DreamTracer, EventKind, Phase as TracePhase};
use crate::events::{HookEvent, HookEventRecord};
use crate::modules::{
    Module,
    dreaming::DreamingModule,
    insight_digest::InsightDigestModule,
    introspection::{IntrospectionModule, ReasoningPatterns},
    intuition::IntuitionModule,
    metacog::{MetacogModule, ToolActivitySample},
    prospective::{FiredRecord, Intention, Priority, ProspectiveModule, Trigger},
    registry::DomainRegistry,
    user_settings::UserSettings,
};
use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal;
use tokio::signal::unix::{SignalKind, signal as unix_signal};
use tracing::{debug, error, info, warn};

/// Relative path (under the data dir) of the hook-event log.
const EVENTS_LOG: &str = "logs/events.jsonl";

/// Dedicated log for UserSignal events from the UserPromptSubmit hook.
/// Separate from EVENTS_LOG so the dreaming module can scan sentiment
/// trends without filtering the general event stream.
const SIGNALS_LOG: &str = "logs/signals.jsonl";

/// Log of what was surfaced at each SessionStart — intention IDs and
/// whether introspection patterns were included. The valence module
/// reads this during consolidation to correlate session outcomes with
/// the insights that were active, closing the implicit feedback loop.
const SURFACED_LOG: &str = "valence/surfaced.jsonl";

/// A record of what the daemon surfaced into a session's context.
/// Written once per SessionStart that produces a non-empty briefing.
#[derive(Debug, Serialize, Deserialize)]
struct SurfacedBriefing {
    /// Daemon-side timestamp when the briefing was composed.
    ts: DateTime<Utc>,
    /// IDs of intentions that were included in the briefing.
    intention_ids: Vec<String>,
    /// Whether the introspection self-awareness section was included.
    has_introspection: bool,
}

/// Relative path of the metacog real-time tool-activity log. Written on
/// each `ToolUse` hook event as a lightweight heartbeat — counterpart to
/// the deep-sampling batch file `metacog/samples.jsonl`.
const METACOG_ACTIVITY_LOG: &str = "metacog/activity.jsonl";

/// Persistent daemon state, saved between consolidation cycles.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DaemonState {
    pub last_consolidation: Option<DateTime<Utc>>,
    pub total_cycles: u64,
    pub total_tokens_used: u64,
    pub last_activity: Option<DateTime<Utc>>,
    /// Populated after each usage check — nil when limits are disabled.
    #[serde(default)]
    pub usage: Option<UsageLimitStatus>,
}

impl DaemonState {
    /// Adopt whichever side has made more progress, field by field. The
    /// counters and timestamps here are all monotonic in intent, so "more
    /// progress" is simply the larger value; `usage` keeps the in-memory
    /// side when present (it is refreshed often and never regresses in a
    /// way that matters).
    pub fn merge_newer(&mut self, disk: DaemonState) {
        self.total_cycles = self.total_cycles.max(disk.total_cycles);
        self.total_tokens_used = self.total_tokens_used.max(disk.total_tokens_used);
        self.last_consolidation = match (self.last_consolidation, disk.last_consolidation) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        self.last_activity = match (self.last_activity, disk.last_activity) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if self.usage.is_none() {
            self.usage = disk.usage;
        }
    }
}

/// Rolling-window Claude Code token usage measured from session transcripts.
/// Written to state.json so the menubar widget can display it without
/// re-scanning transcripts on every refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageLimitStatus {
    /// Output tokens used by Claude Code sessions in the last 5 hours.
    pub output_tokens_5h: u64,
    /// Output tokens used in the last 7 days.
    pub output_tokens_7d: u64,
    /// Configured 5h threshold (0 = disabled).
    pub limit_5h: u64,
    /// Configured 7d threshold (0 = disabled).
    pub limit_7d: u64,
    /// Fraction of the 5h limit consumed (0.0–∞; 0.0 when disabled).
    pub pct_5h: f64,
    /// Fraction of the 7d limit consumed (0.0–∞; 0.0 when disabled).
    pub pct_7d: f64,
    /// True when either enabled window is at or above `warn_pct`.
    pub over_warn_threshold: bool,
    pub checked_at: DateTime<Utc>,
}

pub struct Daemon {
    config: Config,
    store: Store,
    /// Wrapped in a blocking `Mutex` for interior mutability across
    /// `&self` — the hook handler, consolidation loop, and signal
    /// shutdown all need to mutate it through the same `&self`.
    /// We use `std::sync::Mutex` (not `tokio::sync::Mutex`) because the
    /// critical sections are tiny field pokes with no `.await` inside.
    state: Mutex<DaemonState>,
    client: Option<ClaudeClient>,
    /// Guard against concurrent consolidation cycles. The periodic timer
    /// fires every `check_interval` minutes, but if the API call takes
    /// longer than `check_interval`, two cycles can overlap and burn
    /// double tokens. A CAS on this flag in `check_and_run` prevents it.
    cycle_in_progress: Arc<AtomicBool>,
}

impl Daemon {
    pub async fn new(config: Config) -> Result<Self> {
        let store = Store::new(config.data_dir())?;
        store.init_dirs()?;

        let state = if store.exists("state.json") {
            store.read_json("state.json").unwrap_or_default()
        } else {
            DaemonState::default()
        };

        // API client is optional — some commands don't need it.
        // When use_claude_code_cli is set, shell out to `claude --print`
        // instead of the direct API (no ANTHROPIC_API_KEY needed).
        let client = if config.budget.use_claude_code_cli {
            Some(ClaudeClient::new_subprocess(
                &config.budget.claude_code_cli_path,
            ))
        } else {
            ClaudeClient::new().ok()
        };

        Ok(Self {
            config,
            store,
            state: Mutex::new(state),
            client,
            cycle_in_progress: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Mutate the in-memory state and persist the new snapshot to
    /// `state.json`. Callers pass a closure so the mutation and the
    /// write are paired — nothing updates `state` without flushing.
    ///
    /// Before applying the mutation, any progress a sibling writer landed
    /// on disk is adopted (`merge_newer`). Daemon generations overlap: a
    /// SIGTERM'd daemon's in-flight cycle can finish and write AFTER its
    /// successor booted, and the successor's eventual shutdown flush would
    /// otherwise clobber that cycle back off the record (the 1311→1310
    /// step-back, root-caused 2026-07-18).
    fn update_state<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut DaemonState),
    {
        let snapshot = {
            let mut state = self
                .state
                .lock()
                .map_err(|e| anyhow::anyhow!("daemon state mutex poisoned: {e}"))?;
            if let Ok(disk) = self.store.read_json::<DaemonState>("state.json") {
                state.merge_newer(disk);
            }
            f(&mut state);
            serde_json::to_value(&*state)?
        };
        self.store.write_json("state.json", &snapshot)?;
        Ok(())
    }

    /// Lightweight, disk-free state touch. Used on the hot path (every
    /// hook event) to keep `last_activity` fresh without hammering
    /// `state.json` — the disk snapshot is taken at coarser intervals
    /// (end of each consolidation cycle, graceful shutdown).
    fn touch_last_activity(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.last_activity = Some(Utc::now());
        }
    }

    /// Run in the foreground (blocking).
    ///
    /// Three concurrent responsibilities, multiplexed via `tokio::select!`:
    ///   1. Ctrl-C handler (graceful shutdown)
    ///   2. Periodic idle check → consolidation
    ///   3. Unix socket listener for hook events
    ///
    /// The listener is bound once before the loop so we can clean up
    /// the socket file deterministically on exit.
    ///
    /// PID file management lives here (not in `daemonize`) so the file
    /// is written regardless of how the daemon is launched — whether via
    /// `i-dream start --daemonize`, a launchd plist, or plain foreground
    /// mode. Without this, `i-dream dashboard` and `i-dream status` see
    /// a missing PID file and falsely report STOPPED.
    pub async fn run_foreground(&self) -> Result<()> {
        info!("i-dream daemon running in foreground (Ctrl+C to stop)");

        // ── PID file ─────────────────────────────────────────────
        let pid_path = self.config.data_dir().join("daemon.pid");
        match read_pid_file(&pid_path) {
            Some(existing)
                if is_process_alive(existing)
                    && crate::status::process_exe_path(existing)
                        .is_none_or(|exe| exe.file_name().is_some_and(|n| n == "i-dream")) =>
            {
                // Identity-checked: a recycled PID belonging to some other
                // process must not block startup forever (ps lookup failure
                // conservatively counts as "could be us").
                anyhow::bail!(
                    "Daemon already running (PID {existing}). \
                     Run `i-dream stop` first, or remove {} if you're sure it's stale.",
                    pid_path.display()
                );
            }
            Some(existing) => {
                warn!(
                    "Removing stale PID file at {} (PID {existing} is not a live i-dream process)",
                    pid_path.display()
                );
                let _ = std::fs::remove_file(&pid_path);
            }
            None => {}
        }
        let pid = std::process::id();
        write_pid_file(&pid_path, pid)?;
        info!("Daemon started with PID {pid}");

        let check_interval = Duration::from_secs(self.config.idle.check_interval_minutes * 60);

        // Bind Unix socket for hook events.
        let socket_path = self.config.data_dir().join("daemon.sock");
        bind_socket(&socket_path)?;
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("Failed to bind {}", socket_path.display()))?;
        info!("Hook socket listening on {}", socket_path.display());

        // Install a SIGTERM handler for supervisor-driven shutdown.
        //
        // `signal::ctrl_c()` only catches SIGINT, which is what the
        // terminal sends on Ctrl+C. Any supervisor — launchd, systemd,
        // Docker, even a manual `kill $PID` — uses SIGTERM instead,
        // and without this handler tokio falls back to the process
        // default (instant termination) and the cleanup code below
        // never runs. That means a stale PID file, a stale socket
        // file, and a missed `state.json` flush after every restart.
        //
        // The stream is constructed outside the loop so the handler
        // stays installed for the whole daemon lifetime — dropping
        // the `Signal` would reset the signal disposition back to
        // the default.
        let mut sigterm =
            unix_signal(SignalKind::terminate()).context("Failed to install SIGTERM handler")?;

        let result: Result<()> = loop {
            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!("Received SIGINT (Ctrl+C), shutting down");
                    break Ok(());
                }
                _ = sigterm.recv() => {
                    info!("Received SIGTERM (supervisor shutdown), shutting down");
                    break Ok(());
                }
                _ = tokio::time::sleep(check_interval) => {
                    self.check_and_run().await;
                    self.check_and_run_briefing().await;
                }
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _addr)) => {
                            // Touch in-memory activity before handling —
                            // we count "connection received" as activity
                            // whether or not the event parses.
                            self.touch_last_activity();
                            // Spawn the handler so the accept loop stays
                            // responsive to the next connection and to
                            // consolidation-timer wakeups.
                            let store = self.store.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_hook_connection(stream, &store).await {
                                    warn!("Hook event handler failed: {e:#}");
                                }
                            });
                        }
                        Err(e) => {
                            error!("Socket accept failed: {e:#}");
                        }
                    }
                }
            }
        };

        // Best-effort cleanup — don't let a missing file block shutdown.
        if let Err(e) = std::fs::remove_file(&socket_path) {
            debug!("Failed to remove socket file on shutdown: {e}");
        }

        // Always attempt to clean the PID file on exit — whether the
        // foreground loop returned Ok or Err. Best-effort: if the file
        // already vanished (someone ran `i-dream stop`), that's fine.
        if let Err(e) = std::fs::remove_file(&pid_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            debug!("Failed to remove PID file on shutdown: {e}");
        }

        // Flush the in-memory state snapshot one last time so `status`
        // sees the final `last_activity` after a graceful SIGTERM.
        if let Err(e) = self.update_state(|_| {}) {
            debug!("Failed to persist state on shutdown: {e:#}");
        }

        info!("Daemon stopped");
        result
    }

    /// Start the daemon — now just delegates to `run_foreground`, which
    /// handles PID file management itself.
    ///
    /// Kept for backwards compatibility with callers that pass
    /// `--daemonize`; the actual work (PID locking, socket binding, idle
    /// loop) is all in `run_foreground`.
    pub async fn daemonize(&self) -> Result<()> {
        self.run_foreground().await
    }

    /// D4 (2026-05-01): wall-clock check for the Sunday morning briefing.
    /// Cheap — early-exits on weekday/hour mismatch before any I/O. The
    /// underlying module guarantees one-fire-per-ISO-week via state.json.
    async fn check_and_run_briefing(&self) {
        let bm =
            crate::modules::weekly_briefing::WeeklyBriefingModule::new(&self.config, &self.store);
        if !bm.should_run_now() {
            return;
        }
        let client = match crate::api::ClaudeClient::for_config(&self.config) {
            Ok(c) => c,
            Err(e) => {
                warn!("weekly briefing: failed to construct API client: {e:#}");
                return;
            }
        };
        match bm.run(&client).await {
            Ok(Some((tokens, path))) => {
                info!(
                    "weekly briefing: wrote {} ({tokens} tokens)",
                    path.display()
                );
            }
            Ok(None) => {
                // should_run_now said yes but the inner check refused (race
                // with manual --force run). Silent skip.
            }
            Err(e) => {
                warn!("weekly briefing failed: {e:#}");
            }
        }
    }

    /// Check idle state and run consolidation if appropriate.
    ///
    /// Uses a CAS on `cycle_in_progress` to ensure at most one consolidation
    /// cycle runs at a time. Without this guard, a slow API call (>check_interval)
    /// causes the timer to fire again while the previous cycle is still running,
    /// doubling or tripling token consumption.
    async fn check_and_run(&self) {
        match self.should_consolidate() {
            Ok(true) => {
                // Atomically claim the cycle slot. If another cycle is
                // already in progress, the compare_exchange fails (returns Err)
                // and we skip silently — the running cycle will do the work.
                if self
                    .cycle_in_progress
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    debug!("Consolidation already in progress, skipping this check");
                    return;
                }
                info!("Idle threshold reached, starting consolidation cycle");
                // Abort if Claude Code session usage is over the warn threshold.
                if self.check_usage_limit() {
                    warn!(
                        "Usage over warn threshold — skipping automatic consolidation cycle. Trigger manually to override."
                    );
                    self.cycle_in_progress.store(false, Ordering::SeqCst);
                    return;
                }
                let result = self.run_consolidation().await;
                self.cycle_in_progress.store(false, Ordering::SeqCst);
                if let Err(e) = result {
                    error!("Consolidation cycle failed: {e:#}");
                }
            }
            Ok(false) => {
                // Not idle enough yet
            }
            Err(e) => {
                warn!("Failed to check idle state: {e:#}");
            }
        }
    }

    /// Determine if we should run a consolidation cycle.
    fn should_consolidate(&self) -> Result<bool> {
        let activity_path = crate::config::expand_tilde(&self.config.idle.activity_signal);

        let last_activity = if activity_path.exists() {
            let metadata = std::fs::metadata(&activity_path)?;
            let modified = metadata.modified()?;
            DateTime::<Utc>::from(modified)
        } else {
            // No activity file means no recent activity
            Utc::now() - chrono::Duration::hours(self.config.idle.threshold_hours as i64 + 1)
        };

        // Re-read user settings on every check so widget frequency changes
        // take effect without a daemon restart.
        let settings = UserSettings::load(&self.config.data_dir());
        let threshold_hours = settings.effective_threshold_hours(self.config.idle.threshold_hours);
        let threshold_secs = (threshold_hours * 3600.0) as i64;

        let idle_secs = (Utc::now() - last_activity).num_seconds();

        let next_dream_secs = threshold_secs - idle_secs;
        if next_dream_secs > 0 {
            let h = next_dream_secs / 3600;
            let m = (next_dream_secs % 3600) / 60;
            debug!("Next dream in {h}h {m}m (threshold: {threshold_hours:.1}h)");
        }

        Ok(idle_secs >= threshold_secs)
    }

    /// Scan recent Claude Code session transcripts and compute rolling-window
    /// token usage. Writes the result into `state.json` so the menubar can
    /// display it without re-scanning.
    ///
    /// Returns `true` when usage is over the configured warn threshold and
    /// consolidation should be skipped or confirmed.
    fn check_usage_limit(&self) -> bool {
        let cfg = &self.config.limits;
        // Both thresholds disabled — skip the scan entirely.
        if cfg.output_tokens_5h == 0 && cfg.output_tokens_7d == 0 {
            return false;
        }

        let projects_dir = crate::config::expand_tilde(&self.config.ingestion.projects_dir);
        let now = Utc::now();
        let cutoff_5h = now - chrono::Duration::hours(5);
        let cutoff_7d = now - chrono::Duration::days(7);

        let mut tokens_5h: u64 = 0;
        let mut tokens_7d: u64 = 0;

        let Ok(projects) = std::fs::read_dir(&projects_dir) else {
            return false;
        };

        for project in projects.flatten() {
            let Ok(sessions) = std::fs::read_dir(project.path()) else {
                continue;
            };
            for session in sessions.flatten() {
                let path = session.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                // Quick mtime check to skip files untouched in 7 days.
                if let Ok(meta) = session.metadata()
                    && let Ok(modified) = meta.modified()
                {
                    let age = std::time::SystemTime::now()
                        .duration_since(modified)
                        .unwrap_or_default();
                    if age.as_secs() > 7 * 86_400 + 3600 {
                        continue;
                    }
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for line in content.lines() {
                    // Parse only assistant entries (they carry usage).
                    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
                        continue;
                    };
                    if val.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                        continue;
                    }
                    let out_tokens = val
                        .pointer("/usage/outputTokens")
                        .or_else(|| val.pointer("/usage/output_tokens"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if out_tokens == 0 {
                        continue;
                    }
                    let ts_str = val.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                    let ts = ts_str.parse::<DateTime<Utc>>().unwrap_or(now);
                    if ts >= cutoff_7d {
                        tokens_7d += out_tokens;
                        if ts >= cutoff_5h {
                            tokens_5h += out_tokens;
                        }
                    }
                }
            }
        }

        let pct_5h = if cfg.output_tokens_5h > 0 {
            tokens_5h as f64 / cfg.output_tokens_5h as f64
        } else {
            0.0
        };
        let pct_7d = if cfg.output_tokens_7d > 0 {
            tokens_7d as f64 / cfg.output_tokens_7d as f64
        } else {
            0.0
        };
        let over = pct_5h >= cfg.warn_pct || pct_7d >= cfg.warn_pct;

        let status = UsageLimitStatus {
            output_tokens_5h: tokens_5h,
            output_tokens_7d: tokens_7d,
            limit_5h: cfg.output_tokens_5h,
            limit_7d: cfg.output_tokens_7d,
            pct_5h,
            pct_7d,
            over_warn_threshold: over,
            checked_at: now,
        };

        info!(
            "Usage check: 5h={tokens_5h}/{} ({:.0}%), 7d={tokens_7d}/{} ({:.0}%), over={}",
            cfg.output_tokens_5h,
            pct_5h * 100.0,
            cfg.output_tokens_7d,
            pct_7d * 100.0,
            over
        );

        // Persist into state.json for the menubar to read.
        let _ = self.update_state(|s| {
            s.usage = Some(status);
        });

        over
    }

    /// Run the full consolidation cycle, respecting budget and timeouts.
    async fn run_consolidation(&self) -> Result<()> {
        let client = self.client.as_ref().context(
            "API client unavailable — set ANTHROPIC_API_KEY or enable budget.use_claude_code_cli",
        )?;

        // Enumerate the subconscious-domain registry. Today the registry is
        // observation-only — native modules still flow through the
        // module-specific phases below. Stage 2+ of docs/14-dreaming-plugins.md
        // will move dispatch into the registry; building it here on every
        // cycle catches construction failures early and surfaces the
        // registered domain set in logs.
        let registry = DomainRegistry::boot(&self.config, &self.store);
        debug!(
            "Domain registry: {} native domains [{}]",
            registry.len(),
            registry
                .iter()
                .map(|d| d.name())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let mut budget = self.config.budget.max_tokens_per_cycle;
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(self.config.budget.max_runtime_minutes * 60);

        info!(
            "Starting consolidation (budget: {budget} tokens, deadline: {}min)",
            self.config.budget.max_runtime_minutes
        );

        // Phase 1: Dreaming (50% of budget)
        if self.config.modules.dreaming.enabled {
            let module = DreamingModule::new(&self.config, &self.store);
            if module.should_run()? {
                let dreaming_budget = budget / 2;
                info!("Running dreaming module (budget: {dreaming_budget} tokens)");
                match tokio::time::timeout(
                    deadline - tokio::time::Instant::now(),
                    module.run(client, dreaming_budget),
                )
                .await
                {
                    Ok(Ok(tokens)) => {
                        budget = budget.saturating_sub(tokens);
                        info!("Dreaming complete ({tokens} tokens used)");
                    }
                    Ok(Err(e)) => error!("Dreaming failed: {e:#}"),
                    Err(_) => warn!("Dreaming timed out"),
                }
            }
        }

        // Phase 2: Metacognitive analysis (25% of remaining budget)
        if self.config.modules.metacog.enabled && budget > 0 {
            let module = MetacogModule::new(&self.config, &self.store);
            if module.should_run()? {
                let metacog_budget = budget / 2;
                info!("Running metacog module (budget: {metacog_budget} tokens)");
                match tokio::time::timeout(
                    deadline - tokio::time::Instant::now(),
                    module.run(client, metacog_budget),
                )
                .await
                {
                    Ok(Ok(tokens)) => {
                        budget = budget.saturating_sub(tokens);
                        info!("Metacog complete ({tokens} tokens used)");
                    }
                    Ok(Err(e)) => error!("Metacog failed: {e:#}"),
                    Err(_) => warn!("Metacog timed out"),
                }
            }
        }

        // Phase 3: Introspection (remaining budget)
        if self.config.modules.introspection.enabled && budget > 0 {
            let module = IntrospectionModule::new(&self.config, &self.store);
            if module.should_run()? {
                info!("Running introspection module (budget: {budget} tokens)");
                match tokio::time::timeout(
                    deadline - tokio::time::Instant::now(),
                    module.run(client, budget),
                )
                .await
                {
                    Ok(Ok(tokens)) => {
                        budget = budget.saturating_sub(tokens);
                        info!("Introspection complete ({tokens} tokens used)");
                    }
                    Ok(Err(e)) => error!("Introspection failed: {e:#}"),
                    Err(_) => warn!("Introspection timed out"),
                }
            }
        }

        // Phase 4: Intuition (no API budget — pure local transcript analysis)
        if self.config.modules.intuition.enabled {
            let module = IntuitionModule::new(&self.config, &self.store);
            if module.should_run()? {
                info!("Running intuition module (valence collection)");
                match module.run(client, 0).await {
                    Ok(_) => {
                        info!("Intuition complete");
                        // Backfill implicit feedback on surfaced insights
                        // now that valence outcomes are fresh.
                        match module.backfill_insight_feedback() {
                            Ok(n) if n > 0 => info!("Backfilled {n} insight feedback records"),
                            Ok(_) => {}
                            Err(e) => warn!("Insight feedback backfill failed: {e:#}"),
                        }
                    }
                    Err(e) => error!("Intuition failed: {e:#}"),
                }
            }
        }

        // Phase 5: Insight Digest (3h cooldown, capped at 512 tokens)
        if budget > 0 {
            let module = InsightDigestModule::new(&self.config, &self.store);
            if module.should_run()? {
                let digest_budget = budget.min(512);
                info!("Running insight digest (budget: {digest_budget} tokens)");
                match tokio::time::timeout(
                    deadline - tokio::time::Instant::now(),
                    module.run(client, digest_budget),
                )
                .await
                {
                    Ok(Ok(tokens)) => {
                        budget = budget.saturating_sub(tokens);
                        info!("Insight digest complete ({tokens} tokens used)");
                    }
                    Ok(Err(e)) => error!("Insight digest failed: {e:#}"),
                    Err(_) => warn!("Insight digest timed out"),
                }
            }
        }

        // Phase 6: Housekeeping (no API budget)
        let prospective = ProspectiveModule::new(&self.config, &self.store);
        prospective.cleanup_expired()?;

        // Phase 7 (Wave 1 item 6): engine-driven cadence — run every declared
        // external domain's consolidation that has come due (pin decay, atone
        // consolidate, …). Replaces the hand-written per-domain launchd
        // plists. Runs before the lane-health reading below so this cycle's
        // verdict already reflects the dispatch.
        self.dispatch_domain_consolidations(&registry);

        // Record the cycle in persistent state. `used` is derived from
        // the original budget minus whatever's left — saturates at zero
        // if modules somehow overspent (shouldn't, but defensive).
        let used = self
            .config
            .budget
            .max_tokens_per_cycle
            .saturating_sub(budget);
        let cycle_num = {
            let s = self
                .state
                .lock()
                .map_err(|e| anyhow::anyhow!("daemon state mutex poisoned: {e}"))?;
            s.total_cycles + 1
        };
        self.update_state(|s| {
            s.last_consolidation = Some(Utc::now());
            s.total_cycles += 1;
            s.total_tokens_used += used;
        })?;

        info!(
            "[cycle {cycle_num}] complete — used {used} tokens ({budget} remaining of {})",
            self.config.budget.max_tokens_per_cycle
        );

        // Wave 2 items 9+10 — reinforcement: fade every pattern a little, feed
        // this cycle's honored/rejected feedback back onto the source patterns,
        // and evict the weakest so the store keeps what it uses. Runs after
        // dreaming (fresh patterns/associations) and intuition (fresh feedback),
        // and is the single writer of patterns.json's strength dimension.
        match crate::consolidation::reinforce::run_cycle(&self.store) {
            Ok(r) if r.reactivated
                + r.weakened
                + r.stale_skipped
                + r.known_skipped
                + r.evicted
                + r.forgotten
                > 0 =>
            {
                info!(
                    "Reinforcement: {} reactivated, {} weakened, {} stale-skipped, {} known-skipped, {} forgotten, {} evicted, {} surviving",
                    r.reactivated,
                    r.weakened,
                    r.stale_skipped,
                    r.known_skipped,
                    r.forgotten,
                    r.evicted,
                    r.surviving
                )
            }
            Ok(_) => {}
            Err(e) => warn!("Reinforcement pass failed: {e:#}"),
        }

        // Wave 1 item 7 — universal retention: archive each bounded store's
        // overflow into its _archived/<date>/ (traces beyond 30d, all but the
        // 10 newest snapshots, JSONL logs beyond 10k lines). Runs before the
        // lane-health reading so this cycle's verdict reflects the reap.
        for r in crate::modules::registry::run_retention() {
            if r.archived > 0 {
                info!(
                    "Retention: {} → {} overflow entries archived",
                    r.store, r.archived
                );
            }
        }

        // Wave 0: measure every experience lane and record a red/yellow/green
        // reading to dreams/lane-health.jsonl, so a dead lane names itself on
        // the menubar instead of rotting silently. Never fails the cycle.
        if let Err(e) = crate::modules::registry::write_lane_health(&self.store, cycle_num) {
            warn!("Lane-health computation failed: {e:#}");
        }

        // Run post-consolidation hooks (dream-metrics refresh, etc.)
        Self::run_post_wake_hooks();

        // D6 v2 — auto-regenerate per-project briefs for any project
        // whose patterns moved this cycle. Cheap because we already
        // recomputed graph_metrics + the brief module is project-scoped.
        // Errors are logged but don't fail the cycle (briefs are an
        // ergonomic surface, not load-bearing).
        if let Some(client) = self.client.as_ref() {
            self.regen_dirty_project_briefs(client).await;
        }

        // D17 daemon-side weekly auto-prune (opt-in via config).
        if self.config.modules.dreaming.auto_prune_weekly
            && let Err(e) = Self::weekly_auto_prune_patterns(&self.store)
        {
            tracing::warn!("auto-prune (D17) failed: {e:#}");
        }

        // D8 daemon-side — opt-in. Idempotent via Association.auto_intention_id
        // so re-running across cycles only acts on newly-eligible associations.
        if self.config.modules.dreaming.auto_intentions_after_cycle
            && let Err(e) = self.cycle_auto_intentions()
        {
            tracing::warn!("auto-intentions (D8) failed: {e:#}");
        }

        // D19 daemon-side — opt-in drift warnings. Just logs to tracing,
        // doesn't write any files. Surfaces in the daemon's normal log
        // stream so the user notices via existing observability.
        if self.config.modules.dreaming.drift_warnings
            && let Err(e) = Self::cycle_drift_warnings(&self.store)
        {
            tracing::warn!("drift check (D19) failed: {e:#}");
        }

        // M17 daemon-side — auto-snapshot. On by default. Lets
        // `snapshot-diff` answer "what changed last cycle?" without
        // any manual snapshot command. Bounded to most-recent 30.
        if self.config.modules.dreaming.auto_snapshot_each_cycle
            && let Err(e) = Self::cycle_auto_snapshot(&self.store)
        {
            tracing::warn!("auto-snapshot (M17) failed: {e:#}");
        }

        Ok(())
    }

    /// Wave 1 item 6 — the engine drives every declared domain cadence.
    ///
    /// External domains declare `[consolidation] cadence` in their manifest,
    /// but until now only hand-written launchd plists actually ran any of
    /// them (atone had one; pinned never did, so pins never decayed). Each
    /// cycle this walks the registry and runs `consolidate()` for every
    /// external domain whose cadence has elapsed, tracking last-run stamps in
    /// `dreams/domain-cadence.json`. Failures log and leave the stamp
    /// unadvanced so the next cycle retries; nothing here fails the cycle.
    fn dispatch_domain_consolidations(&self, registry: &DomainRegistry) {
        let mut state: DomainCadenceState = if self.store.exists(DOMAIN_CADENCE_STATE) {
            self.store
                .read_json(DOMAIN_CADENCE_STATE)
                .unwrap_or_default()
        } else {
            DomainCadenceState::default()
        };
        let (ran, failed) = dispatch_due_consolidations(registry, &mut state, Utc::now());
        if ran > 0 || failed > 0 {
            info!("Domain cadence dispatch: {ran} consolidations ran, {failed} failed");
        }
        if ran > 0
            && let Err(e) = self.store.write_json(DOMAIN_CADENCE_STATE, &state)
        {
            warn!("Cannot persist domain-cadence state: {e:#}");
        }
    }

    /// M17 daemon hook — write the snapshot that `snapshot-diff` reads.
    ///
    /// Writing only. Bounding this directory belongs to retention, which
    /// archives old snapshots instead of deleting them; this used to hard-delete
    /// everything past the newest 30, which the plan of record forbids (archive
    /// before delete). Two owners of one directory, one of them destructive, is
    /// how a store quietly loses history — so there is now one owner.
    fn cycle_auto_snapshot(store: &Store) -> Result<()> {
        let _ = crate::graph_metrics::snapshot_for_diff(store)?;
        Ok(())
    }

    /// D8 daemon hook — promote eligible associations using the configured
    /// threshold. Reuses ProspectiveModule::auto_promote_associations so the
    /// CLI and daemon paths share one implementation.
    fn cycle_auto_intentions(&self) -> Result<()> {
        let mut associations: Vec<crate::modules::dreaming::Association> = self
            .store
            .read_json("dreams/associations.json")
            .unwrap_or_default();
        let patterns: Vec<crate::modules::dreaming::ExtractedPattern> = self
            .store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        let pm = crate::modules::prospective::ProspectiveModule::new(&self.config, &self.store);
        let threshold = self.config.modules.dreaming.auto_intention_threshold;
        let (created, _skipped) =
            pm.auto_promote_associations(&mut associations, &patterns, threshold, false)?;
        if created > 0 {
            self.store
                .write_json("dreams/associations.json", &associations)?;
            tracing::info!(
                created,
                threshold,
                "D8 daemon: promoted associations to intentions"
            );
        }
        Ok(())
    }

    /// D19 daemon hook — emit a tracing::warn for each category whose
    /// average confidence dropped ≥10% week-over-week. Mirrors the
    /// `drift` CLI command's logic (sample-size floor of 3 per window).
    fn cycle_drift_warnings(store: &Store) -> Result<()> {
        use chrono::{DateTime, Duration, Utc};
        let patterns: Vec<crate::modules::dreaming::ExtractedPattern> =
            store.read_json("dreams/patterns.json").unwrap_or_default();
        let now = Utc::now();
        let cutoff_recent = now - Duration::days(7);
        let cutoff_prior = now - Duration::days(14);
        let mut recent: std::collections::HashMap<&str, (f64, usize)> =
            std::collections::HashMap::new();
        let mut prior: std::collections::HashMap<&str, (f64, usize)> =
            std::collections::HashMap::new();
        for p in &patterns {
            let Ok(ts) = DateTime::parse_from_rfc3339(&p.last_seen) else {
                continue;
            };
            let ts = ts.with_timezone(&Utc);
            let bucket = if ts >= cutoff_recent {
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
        for (cat, (sum_p, n_p)) in &prior {
            if *n_p < 3 {
                continue;
            }
            let prior_avg = sum_p / *n_p as f64;
            let (sum_r, n_r) = recent.get(cat).copied().unwrap_or((0.0, 0));
            if n_r < 3 {
                continue;
            }
            let recent_avg = sum_r / n_r as f64;
            let rel_drop = (prior_avg - recent_avg) / prior_avg.max(1e-9);
            if rel_drop >= 0.10 {
                tracing::warn!(
                    category = cat,
                    prior_avg,
                    recent_avg,
                    relative_drop = rel_drop,
                    n_prior = n_p,
                    n_recent = n_r,
                    "D19: category-level confidence drift detected (≥10% week-over-week drop)",
                );
            }
        }
        Ok(())
    }

    /// D17 — runs at most once per ISO week. State tracked in
    /// dreams/auto-prune-state.json (last_run_iso). Conservative defaults:
    /// confidence < 0.40 AND last_seen older than 60 days. Backups are
    /// always written to dreams/pruned/<ts>.json so the user can restore
    /// via `i-dream prune-patterns --restore <ts>`.
    fn weekly_auto_prune_patterns(store: &Store) -> Result<()> {
        use chrono::{DateTime, Datelike, Duration, Utc};

        #[derive(serde::Serialize, serde::Deserialize, Default)]
        struct State {
            last_run_iso_week: String,
            last_run_ts: String,
            last_pruned: usize,
        }

        let now = Utc::now();
        let iso = now.iso_week();
        let this_week = format!("{}-W{:02}", iso.year(), iso.week());
        let state: State = store
            .read_json("dreams/auto-prune-state.json")
            .unwrap_or_default();
        if state.last_run_iso_week == this_week {
            return Ok(());
        }

        let cutoff = now - Duration::days(60);
        let all: Vec<crate::modules::dreaming::ExtractedPattern> =
            store.read_json("dreams/patterns.json").unwrap_or_default();
        let (to_prune, to_keep): (Vec<_>, Vec<_>) = all.into_iter().partition(|p| {
            if p.confidence >= 0.40 {
                return false;
            }
            let last_seen = DateTime::parse_from_rfc3339(&p.last_seen)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(cutoff - Duration::days(1));
            last_seen < cutoff
        });

        // Even when nothing is pruned, advance the iso-week marker so we
        // don't re-scan every cycle within the same week.
        let stamp = now.format("%Y%m%d-%H%M%S").to_string();
        if !to_prune.is_empty() {
            let backup_rel = format!("dreams/pruned/{}.json", stamp);
            if let Some(parent) = store.path(&backup_rel).parent() {
                std::fs::create_dir_all(parent)?;
            }
            store.write_json(&backup_rel, &to_prune)?;
            store.write_json("dreams/patterns.json", &to_keep)?;
            tracing::info!(pruned = to_prune.len(), backup = %store.path(&backup_rel).display(),
                "D17 auto-prune (weekly) ran");
        }
        let new_state = State {
            last_run_iso_week: this_week,
            last_run_ts: now.to_rfc3339(),
            last_pruned: to_prune.len(),
        };
        store.write_json("dreams/auto-prune-state.json", &new_state)?;
        Ok(())
    }

    /// D6 v2: refresh per-project briefs that have new pattern activity
    /// since the last brief generation. Compares each project's most
    /// recent pattern last_seen against the brief file's mtime; if newer,
    /// regenerates the brief.
    async fn regen_dirty_project_briefs(&self, client: &ClaudeClient) {
        use crate::modules::dreaming::ExtractedPattern;
        use std::collections::HashMap;
        let patterns: Vec<ExtractedPattern> = self
            .store
            .read_json("dreams/patterns.json")
            .unwrap_or_default();
        if patterns.is_empty() {
            return;
        }
        // Map project_id → max(last_seen) across its patterns.
        let mut latest: HashMap<String, chrono::DateTime<chrono::Utc>> = HashMap::new();
        for p in &patterns {
            for proj in &p.source_projects {
                if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&p.last_seen) {
                    let ts_utc = ts.with_timezone(&chrono::Utc);
                    latest
                        .entry(proj.clone())
                        .and_modify(|cur| {
                            if ts_utc > *cur {
                                *cur = ts_utc;
                            }
                        })
                        .or_insert(ts_utc);
                }
            }
        }
        let pbm =
            crate::modules::project_briefs::ProjectBriefsModule::new(&self.config, &self.store);
        let mut regen = 0u32;
        for (proj, ts) in latest {
            let brief_path = self.store.path(&format!("dreams/project-briefs/{proj}.md"));
            // Regenerate if missing OR pattern activity is newer than the brief mtime.
            let needs = !brief_path.exists()
                || std::fs::metadata(&brief_path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(|sys| chrono::DateTime::<chrono::Utc>::from(sys) < ts)
                    .unwrap_or(true);
            if !needs {
                continue;
            }
            match pbm.generate_for_project(client, &proj).await {
                Ok((tokens, _)) => {
                    info!("D6 v2: regenerated brief for {proj} ({tokens} tokens)");
                    regen += 1;
                }
                Err(e) => warn!("D6 v2: brief regen failed for {proj}: {e:#}"),
            }
        }
        if regen > 0 {
            info!("D6 v2: refreshed {regen} project brief(s) post-cycle");
        }
    }

    /// Manually trigger a dream cycle.
    ///
    /// The `All` case delegates to `module.run`, which owns its own
    /// tracer. Single-phase runs (`Sws`/`Rem`/`Wake`) create a tracer
    /// here and bracket the call with CycleStart/CycleEnd so their
    /// trace files look structurally identical to a full-cycle trace on
    /// the dashboard.
    pub async fn run_dream(&self, phase: DreamPhase) -> Result<()> {
        let client = self.client.as_ref().context(
            "API client unavailable — set ANTHROPIC_API_KEY or enable budget.use_claude_code_cli",
        )?;

        // Check usage limits before spending API tokens. The CLI and menubar
        // both surface this as a warning rather than a hard block, but the
        // daemon records the status so callers can display it.
        if self.check_usage_limit() {
            warn!(
                "Usage over warn threshold ({:.0}% of 5h limit, {:.0}% of 7d limit). \
                 Proceeding with manual trigger.",
                self.store
                    .read_json::<DaemonState>("state.json")
                    .ok()
                    .and_then(|s| s.usage)
                    .map(|u| u.pct_5h * 100.0)
                    .unwrap_or(0.0),
                self.store
                    .read_json::<DaemonState>("state.json")
                    .ok()
                    .and_then(|s| s.usage)
                    .map(|u| u.pct_7d * 100.0)
                    .unwrap_or(0.0),
            );
        }

        let module = DreamingModule::new(&self.config, &self.store);
        let budget = self.config.budget.max_tokens_per_cycle;

        match phase {
            DreamPhase::All => {
                // Run the full consolidation pipeline (all modules) so
                // manual dream cycles don't silently skip InsightDigest,
                // Metacog, and Intuition.
                self.run_consolidation().await?;
                return Ok(());
            }
            DreamPhase::Sws => {
                let tracer = DreamTracer::new(&self.store);
                tracer.emit(
                    TracePhase::Init,
                    EventKind::CycleStart,
                    "manual: sws only".to_string(),
                    vec![],
                    vec![tracer.trace_rel_path().to_string()],
                )?;
                let (tokens, _, _) = module.run_sws(client, budget, &tracer).await?;
                tracer.note(
                    TracePhase::Done,
                    EventKind::CycleEnd,
                    format!("total_tokens={tokens}"),
                )?;
            }
            DreamPhase::Rem => {
                let tracer = DreamTracer::new(&self.store);
                tracer.emit(
                    TracePhase::Init,
                    EventKind::CycleStart,
                    "manual: rem only".to_string(),
                    vec![],
                    vec![tracer.trace_rel_path().to_string()],
                )?;
                let (tokens, _) = module.run_rem(client, budget, &tracer).await?;
                tracer.note(
                    TracePhase::Done,
                    EventKind::CycleEnd,
                    format!("total_tokens={tokens}"),
                )?;
            }
            DreamPhase::Wake => {
                let tracer = DreamTracer::new(&self.store);
                tracer.emit(
                    TracePhase::Init,
                    EventKind::CycleStart,
                    "manual: wake only".to_string(),
                    vec![],
                    vec![tracer.trace_rel_path().to_string()],
                )?;
                let (tokens, _) = module.run_wake(client, budget, &tracer).await?;
                tracer.note(
                    TracePhase::Done,
                    EventKind::CycleEnd,
                    format!("total_tokens={tokens}"),
                )?;
                Self::run_post_wake_hooks();
            }
        }

        Ok(())
    }

    /// Spawn post-wake hook scripts (dream-metrics refresh, insight injection, etc.)
    ///
    /// These scripts run fire-and-forget — failures are logged but do not
    /// block the consolidation cycle. The hooks live under
    /// `~/.claude/subconscious/hooks/` and `~/.claude/scripts/`.
    fn run_post_wake_hooks() {
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => return,
        };

        let hooks = [
            home.join(".claude/scripts/dream-metrics.sh"),
            home.join(".claude/subconscious/hooks/post-wake.sh"),
        ];

        for hook in &hooks {
            if hook.exists() {
                info!("Running post-wake hook: {}", hook.display());
                match std::process::Command::new("bash")
                    .arg(hook)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                {
                    Ok(mut child) => {
                        // Wait briefly for the script — these are lightweight.
                        match child.wait() {
                            Ok(status) if status.success() => {
                                info!("Post-wake hook succeeded: {}", hook.display());
                            }
                            Ok(status) => {
                                let stderr = child.stderr.take().map(|mut s| {
                                    let mut buf = String::new();
                                    std::io::Read::read_to_string(&mut s, &mut buf).ok();
                                    buf
                                });
                                warn!(
                                    "Post-wake hook exited {}: {} ({})",
                                    status,
                                    hook.display(),
                                    stderr.unwrap_or_default().trim()
                                );
                            }
                            Err(e) => warn!("Post-wake hook wait failed: {e:#}"),
                        }
                    }
                    Err(e) => warn!("Failed to spawn post-wake hook {}: {e:#}", hook.display()),
                }
            }
        }
    }

    /// Stop a running daemon, verifying liveness and waiting for exit.
    ///
    /// Protocol:
    ///   1. If no PID file → nothing to do.
    ///   2. If PID file exists but process is dead → clean stale file,
    ///      report, return. **Never signal a stale PID** — it may have
    ///      been recycled by an unrelated process.
    ///   3. Send SIGTERM, poll for up to 3 s for the process to exit.
    ///   4. If still alive, fall back to SIGKILL and give it 200 ms.
    ///   5. Remove the PID file as the final step (the daemon's own
    ///      shutdown path also tries to remove it, whichever wins is
    ///      fine — `NotFound` is ignored).
    pub async fn stop() -> Result<()> {
        let pid_path = pid_file_path();
        let pid = match read_pid_file(&pid_path) {
            Some(p) => p,
            None => {
                println!("No daemon running (no PID file found)");
                return Ok(());
            }
        };

        if !is_process_alive(pid) {
            println!("Stale PID file (PID {pid} is not alive), cleaning up");
            let _ = std::fs::remove_file(&pid_path);
            return Ok(());
        }

        // Liveness is not identity: a recycled PID can belong to any
        // process, and signaling a stranger is the one unrecoverable
        // mistake here. Only signal a process that is actually i-dream.
        if let Some(exe) = crate::status::process_exe_path(pid)
            && exe.file_name().is_none_or(|n| n != "i-dream")
        {
            println!(
                "PID file points at PID {pid}, but that process is {} — \
                 not signaling it. Cleaning up the stale PID file.",
                exe.display()
            );
            let _ = std::fs::remove_file(&pid_path);
            return Ok(());
        }

        // Send SIGTERM. Safety: we verified the PID is alive and the
        // kill(2) syscall with a valid signal is always well-defined.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }

        // Poll for graceful exit (30 × 100 ms = 3 s).
        let exited = wait_for_exit(pid, 30, Duration::from_millis(100)).await;

        if !exited {
            warn!("Daemon (PID {pid}) did not exit on SIGTERM, sending SIGKILL");
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            // Give the kernel a moment to reap.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = std::fs::remove_file(&pid_path);
            println!("Force-stopped daemon (PID {pid})");
        } else {
            let _ = std::fs::remove_file(&pid_path);
            println!("Stopped daemon (PID {pid})");
        }
        Ok(())
    }

}

/// Read and parse the daemon PID file, returning `None` if the file
/// doesn't exist or the contents are unparseable. Broken contents get
/// logged but produce `None` so callers can treat it as "no daemon".
pub(crate) fn read_pid_file(path: &Path) -> Option<i32> {
    let content = std::fs::read_to_string(path).ok()?;
    match content.trim().parse::<i32>() {
        Ok(pid) => Some(pid),
        Err(e) => {
            warn!("PID file at {} is corrupt: {e}", path.display());
            None
        }
    }
}

/// Atomically write a PID to the PID file. Uses tmp+rename so a
/// reader will never observe a half-written file.
fn write_pid_file(path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("pid.tmp");
    std::fs::write(&tmp, pid.to_string())
        .with_context(|| format!("Failed to write PID file tmp at {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename PID file to {}", path.display()))?;
    Ok(())
}

/// Check whether a PID refers to a process we could signal.
///
/// Uses `kill(pid, 0)` — the null signal, which performs the usual
/// permission and existence checks without actually delivering a
/// signal. Returns `true` iff the process exists. This is the portable
/// Unix idiom and is exactly what `systemctl` / `docker` do.
pub(crate) fn is_process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Safety: kill(2) with sig=0 is always safe — it performs checks
    // but delivers no signal, and has no side effects on the target.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Relative path (under the data dir) of the engine-dispatch cadence ledger.
const DOMAIN_CADENCE_STATE: &str = "dreams/domain-cadence.json";

/// When each external domain's consolidation last ran under engine dispatch.
/// Persisted at `dreams/domain-cadence.json` so cadences survive daemon
/// restarts.
#[derive(Debug, Default, Serialize, Deserialize)]
struct DomainCadenceState {
    #[serde(default)]
    last_run: std::collections::HashMap<String, DateTime<Utc>>,
}

/// Parse a manifest cadence word into a period. Vocabulary in the wild:
/// "daily", "weekly", "every-2-days" (plus "hourly"/"every-N-hours" for
/// symmetry). Unknown values yield None — the caller warns and skips, so a
/// typo'd manifest can't silently settle on some default rhythm.
fn parse_cadence(s: &str) -> Option<chrono::Duration> {
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "hourly" => return Some(chrono::Duration::hours(1)),
        "daily" => return Some(chrono::Duration::days(1)),
        "weekly" => return Some(chrono::Duration::weeks(1)),
        _ => {}
    }
    let rest = s.strip_prefix("every-")?;
    if let Some(n) = rest.strip_suffix("-days") {
        return n.parse::<i64>().ok().map(chrono::Duration::days);
    }
    if let Some(n) = rest.strip_suffix("-hours") {
        return n.parse::<i64>().ok().map(chrono::Duration::hours);
    }
    None
}

/// Run `consolidate()` for every external domain whose cadence has elapsed.
/// Returns `(ran, failed)`. Success advances the domain's last-run stamp;
/// failure leaves it, so the next cycle retries. Native modules (their
/// synthetic manifests carry no script) and disabled specs are skipped.
fn dispatch_due_consolidations(
    registry: &DomainRegistry,
    state: &mut DomainCadenceState,
    now: DateTime<Utc>,
) -> (usize, usize) {
    let (mut ran, mut failed) = (0usize, 0usize);
    for d in registry.iter() {
        let spec = &d.manifest().consolidation;
        if !spec.enabled || spec.script.is_none() {
            continue;
        }
        let Some(period) = parse_cadence(&spec.cadence) else {
            warn!(
                "Domain '{}' has unparseable consolidation cadence '{}' — skipping",
                d.name(),
                spec.cadence
            );
            continue;
        };
        if let Some(last) = state.last_run.get(d.name())
            && now - *last < period
        {
            continue;
        }
        match d.consolidate() {
            Ok(report) => {
                info!(
                    "Domain '{}' consolidated: {} events, {}ms{}",
                    d.name(),
                    report.events_processed,
                    report.runtime_ms,
                    report
                        .note
                        .as_deref()
                        .map(|n| format!(" ({n})"))
                        .unwrap_or_default()
                );
                state.last_run.insert(d.name().to_string(), now);
                ran += 1;
            }
            Err(e) => {
                warn!("Domain '{}' consolidation failed: {e:#}", d.name());
                failed += 1;
            }
        }
    }
    (ran, failed)
}

/// Poll `is_process_alive` up to `attempts` times waiting `interval`
/// between each check. Returns `true` as soon as the process is gone,
/// `false` if it was still alive at the final check.
async fn wait_for_exit(pid: i32, attempts: u32, interval: Duration) -> bool {
    for _ in 0..attempts {
        if !is_process_alive(pid) {
            return true;
        }
        tokio::time::sleep(interval).await;
    }
    !is_process_alive(pid)
}

pub(crate) fn pid_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude/subconscious/daemon.pid")
}

/// Remove any stale socket file before binding, but only if no live daemon
/// is already listening on it.
///
/// Strategy:
///   1. If the socket file exists, try connecting to it.
///   2. If the connect succeeds, a daemon is alive — return an error so the
///      caller knows not to proceed (duplicate-start prevention).
///   3. If the connect fails (ECONNREFUSED / ENOENT / similar), the socket
///      is stale — remove it and proceed normally.
///   4. If no file exists, just ensure the parent directory is present.
fn bind_socket(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        // Probe: if another daemon is listening, connecting will succeed.
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => {
                anyhow::bail!(
                    "A daemon is already running on socket {}. \
                     Use `i-dream stop` first.",
                    path.display()
                );
            }
            Err(_) => {
                // Socket file is stale — safe to remove.
                std::fs::remove_file(path).with_context(|| {
                    format!("Failed to remove stale socket at {}", path.display())
                })?;
            }
        }
    }
    Ok(())
}

/// Handle a single hook-script connection.
///
/// Protocol: the client writes one JSON line and closes the write half.
/// We parse it into a `HookEvent`, append to `logs/events.jsonl`, touch
/// the activity signal via the `last_activity` field (task #6 will wire
/// this into state.json), and write an empty response.
///
/// Task #4 (SessionStart response injection) will populate the response
/// body with matched intuitions/intentions for `SessionStart` events.
async fn handle_hook_connection(stream: UnixStream, store: &Store) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Read a single line — the hook scripts send exactly one JSON object,
    // newline-terminated. The timeout bounds a client that connects and
    // then neither terminates its line nor closes (each such connection
    // would otherwise pin a task indefinitely).
    let mut line = String::new();
    let bytes_read = match tokio::time::timeout(
        Duration::from_secs(10),
        reader.read_line(&mut line),
    )
    .await
    {
        Ok(read) => read?,
        Err(_) => {
            debug!("Hook connection sent no complete line within 10s, dropping");
            return Ok(());
        }
    };
    if bytes_read == 0 {
        debug!("Empty hook connection, ignoring");
        return Ok(());
    }

    let trimmed = line.trim();
    let event: HookEvent = serde_json::from_str(trimmed)
        .with_context(|| format!("Invalid hook event payload: {trimmed}"))?;
    debug!("Received hook event: {event:?}");

    let record = HookEventRecord::new(event.clone());
    store.append_jsonl(EVENTS_LOG, &record)?;

    // Per-event side effects. `ToolUse` gets a lightweight realtime
    // sample written to metacog/activity.jsonl — the per-tool heartbeat
    // that complements the deep batch sampling done during consolidation.
    // Best-effort: a failed activity write must not drop the event ack.
    if let HookEvent::ToolUse { tool, ts } = &event {
        let sample = ToolActivitySample {
            received_at: record.received_at,
            tool: tool.clone(),
            hook_ts: *ts,
        };
        if let Err(e) = store.append_jsonl(METACOG_ACTIVITY_LOG, &sample) {
            warn!("Failed to write metacog activity sample: {e:#}");
        }
    }

    // `UserSignal` gets a secondary write to logs/signals.jsonl so the
    // dreaming module can query sentiment trends independently of the
    // general event stream. Best-effort like the metacog activity write.
    if let HookEvent::UserSignal { .. } = &event
        && let Err(e) = store.append_jsonl(SIGNALS_LOG, &record)
    {
        warn!("Failed to write user signal to signals log: {e:#}");
    }

    // D3 v2 (2026-05-01): if this user prompt is a correction and any
    // dream-spawned intention fired in the recent past, infer that the
    // surfaced intention was unhelpful and write an auto-downvote into
    // insight-feedback.jsonl. The next Wake cycle will apply it; per D3 v1,
    // confidence dropping below 0.2 then marks the source association
    // dismissed permanently.
    if let HookEvent::UserSignal {
        correction: true, ..
    } = &event
        && let Err(e) = auto_downvote_recently_fired_intentions(store)
    {
        warn!("D3 v2 auto-downvote failed: {e:#}");
    }

    // SessionStart is the only event that gets a non-empty response —
    // the hook script echoes whatever we write back into Claude's context.
    // For all other events we just ack with an empty body.
    let build_started = std::time::Instant::now();
    let response = match &event {
        HookEvent::SessionStart { cwd, .. } => {
            let (text, intention_ids, has_introspection) =
                build_session_start_response(store, cwd.as_deref());
            // Persist what was surfaced so the valence module can
            // correlate session outcomes with active insights.
            if !intention_ids.is_empty() || has_introspection {
                let briefing = SurfacedBriefing {
                    ts: Utc::now(),
                    intention_ids,
                    has_introspection,
                };
                if let Err(e) = store.append_jsonl(SURFACED_LOG, &briefing) {
                    warn!("Failed to log surfaced briefing: {e:#}");
                }
            }
            text
        }
        _ => String::new(),
    };
    if !response.is_empty() {
        // A hung-up client (its recv timeout beat our build) is a delivery
        // miss, not a malfunction — log it as such, with the latency that
        // caused it, instead of the generic handler-failed warn.
        let write_result = async {
            writer.write_all(response.as_bytes()).await?;
            writer.shutdown().await
        }
        .await;
        if let Err(e) = write_result {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                debug!(
                    "SessionStart briefing undeliverable — client hung up first \
                     (briefing {} bytes, built in {}ms)",
                    response.len(),
                    build_started.elapsed().as_millis()
                );
                return Ok(());
            }
            return Err(e.into());
        }
    }

    Ok(())
}

/// D3 v2 helper: scan intentions/fired.jsonl for FiredRecord rows from
/// the last 10 minutes; for each, look up the originating intention in
/// intentions/registry.jsonl, parse its `action.source` for the
/// "dream-wake:<assoc_id>" tag the Wake phase writes, and append an
/// auto-downvote entry to dreams/insight-feedback.jsonl tagged
/// `source: "auto-correction"`. Idempotent within a window — the same
/// fire can produce multiple auto-downvotes if multiple corrections land
/// in quick succession; that's accepted (the Wake handler caps confidence
/// at 0.0 anyway).
fn auto_downvote_recently_fired_intentions(store: &Store) -> Result<()> {
    use serde_json::json;
    const WINDOW_MIN: i64 = 10;

    let fired: Vec<FiredRecord> = store
        .read_jsonl("intentions/fired.jsonl")
        .unwrap_or_default();
    if fired.is_empty() {
        return Ok(());
    }
    let cutoff = Utc::now() - chrono::Duration::minutes(WINDOW_MIN);
    let recent: Vec<&FiredRecord> = fired.iter().filter(|r| r.fired_at >= cutoff).collect();
    if recent.is_empty() {
        return Ok(());
    }

    let registry: Vec<Intention> = store
        .read_jsonl("intentions/registry.jsonl")
        .unwrap_or_default();
    if registry.is_empty() {
        return Ok(());
    }

    let intention_by_id: std::collections::HashMap<&str, &Intention> =
        registry.iter().map(|i| (i.id.as_str(), i)).collect();

    let now_ts = Utc::now().to_rfc3339();
    let mut downvoted = 0usize;
    for fired_record in &recent {
        let Some(intent) = intention_by_id.get(fired_record.intention_id.as_str()) else {
            continue;
        };
        let Some(assoc_id) = intent.action.source.strip_prefix("dream-wake:") else {
            continue;
        };
        let entry = json!({
            "insight_id": assoc_id,
            "rating": "down",
            "source": "auto-correction",
            "ts": now_ts.clone(),
            "intention_id": fired_record.intention_id,
        });
        if let Err(e) = store.append_jsonl("dreams/insight-feedback.jsonl", &entry) {
            warn!("D3 v2: failed to append auto-downvote: {e:#}");
        } else {
            downvoted += 1;
        }
    }
    if downvoted > 0 {
        info!(
            "D3 v2: auto-downvoted {downvoted} dream-spawned intention(s) after correction signal"
        );
    }
    Ok(())
}

/// Compose the markdown briefing the hook echoes into Claude's context
/// at session start.
///
/// SessionStart carries no message text (only a timestamp), so we can
/// only surface context-free signal:
///   1. Broadcast intentions — `Trigger::Time` entries where the
///      `after` gate has passed and `keywords` is empty. Context-gated
///      intentions (Event/Context triggers) need the first user prompt
///      to match against and are deferred until we can hook
///      UserPromptSubmit.
///   2. Reasoning patterns from the latest introspection report —
///      recent strengths, weaknesses, and common assumptions.
///
/// Returns an empty string when nothing is worth surfacing. An empty
/// body is the correct no-op signal for the shell hook — it writes
/// nothing into Claude's context.
fn build_session_start_response(store: &Store, cwd: Option<&str>) -> (String, Vec<String>, bool) {
    let mut sections: Vec<String> = Vec::new();
    let mut surfaced_ids: Vec<String> = Vec::new();
    let mut has_introspection = false;

    // ── 0. D6 — per-project brief if cwd provided and brief exists ──
    if let Some(cwd_str) = cwd {
        // We don't have a Config here, so build the module without one —
        // generation needs config but read_for_cwd doesn't. Construct a
        // minimal stand-in: pass the same Store; the read path doesn't
        // touch config.
        let id = crate::modules::project_briefs::ProjectBriefsModule::encode_cwd(cwd_str);
        let path = store.path(&format!("dreams/project-briefs/{id}.md"));
        if let Ok(brief) = std::fs::read_to_string(&path) {
            let trimmed = brief.trim();
            if !trimmed.is_empty() {
                sections.push(format!("## Project brief\n\n{trimmed}"));
            }
        }
    }

    // ── 1. Broadcast intentions ─────────────────────────────
    if let Some((section, ids)) = broadcast_intentions_section(store) {
        sections.push(section);
        surfaced_ids = ids;
    }

    // ── 2. Introspection patterns ───────────────────────────
    if let Some(section) = introspection_patterns_section(store) {
        sections.push(section);
        has_introspection = true;
    }

    if sections.is_empty() {
        return (String::new(), Vec::new(), false);
    }

    let mut out = String::from("# i-dream briefing\n\n");
    out.push_str(&sections.join("\n\n"));
    out.push('\n');
    (out, surfaced_ids, has_introspection)
}

/// Surface active intentions from the registry into the session briefing.
///
/// Includes both broadcast-ready (Time triggers with no keywords) and
/// Context-trigger intentions. Context triggers are behavioral rules
/// derived from dream insights — they don't need keyword matching against
/// a specific message because they apply broadly to session behavior.
/// Surfacing them here closes the dream→session feedback loop.
///
/// Each surfaced intention gets its fire_count incremented and a
/// FiredRecord logged so we can track engagement.
fn broadcast_intentions_section(store: &Store) -> Option<(String, Vec<String>)> {
    let mut registry: Vec<Intention> = store
        .read_jsonl("intentions/registry.jsonl")
        .unwrap_or_default();
    if registry.is_empty() {
        return None;
    }

    let now = Utc::now();
    let surfaced_ids: Vec<String> = registry
        .iter()
        .filter(|intent| intent.expires > now)
        .filter(|intent| intent.fire_count < intent.max_fires)
        .filter(|intent| match &intent.trigger {
            // Broadcast Time triggers (original behavior)
            Trigger::Time { after, keywords } => *after <= now && keywords.is_empty(),
            // Context triggers — behavioral rules from dream insights
            Trigger::Context { .. } => true,
            // Event triggers need keyword matching — skip at session start
            Trigger::Event { .. } => false,
        })
        .map(|i| i.id.clone())
        .collect();

    if surfaced_ids.is_empty() {
        return None;
    }

    // Increment fire_count for all surfaced intentions and rewrite registry.
    for intent in &mut registry {
        if surfaced_ids.contains(&intent.id) {
            intent.fire_count += 1;
            intent.last_fired = Some(now);
        }
    }
    // Atomically rewrite the JSONL registry with updated fire counts.
    let registry_path = store.path("intentions/registry.jsonl");
    let tmp_path = registry_path.with_extension("tmp");
    let lines: String = registry
        .iter()
        .filter_map(|i| serde_json::to_string(i).ok())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    if let Err(e) =
        std::fs::write(&tmp_path, &lines).and_then(|_| std::fs::rename(&tmp_path, &registry_path))
    {
        warn!("Failed to update intention fire counts: {e:#}");
    }

    // Log fired records
    for id in &surfaced_ids {
        let record = FiredRecord {
            intention_id: id.clone(),
            fired_at: now,
            session_id: String::new(), // SessionStart has no session ID
            was_relevant: None,
        };
        let _ = store.append_jsonl("intentions/fired.jsonl", &record);
    }

    // Build the output section, sorted by priority.
    let mut to_surface: Vec<&Intention> = registry
        .iter()
        .filter(|i| surfaced_ids.contains(&i.id))
        .collect();
    to_surface.sort_by_key(|i| match i.action.priority {
        Priority::High => 0,
        Priority::Medium => 1,
        Priority::Low => 2,
    });

    let mut s = format!("## Behavioral rules ({})", to_surface.len());
    for intent in to_surface {
        let tag = match intent.action.priority {
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        };
        s.push_str(&format!("\n- [{tag}] {}", intent.action.message));
    }
    Some((s, surfaced_ids))
}

/// Surface strengths/weaknesses/assumptions from the latest
/// introspection report, if one exists.
fn introspection_patterns_section(store: &Store) -> Option<String> {
    if !store.exists("introspection/patterns.json") {
        return None;
    }
    let patterns: ReasoningPatterns = store.read_json("introspection/patterns.json").ok()?;

    let strengths = patterns.strength_patterns.join(", ");
    let weaknesses = patterns.weakness_patterns.join(", ");
    let assumptions = patterns.common_assumptions.join(", ");

    // If every field is empty there's nothing worth surfacing.
    if strengths.is_empty() && weaknesses.is_empty() && assumptions.is_empty() {
        return None;
    }

    let mut s = String::from("## Self-awareness");
    if !strengths.is_empty() {
        s.push_str(&format!("\nRecent strengths: {strengths}"));
    }
    if !weaknesses.is_empty() {
        s.push_str(&format!("\nWatch for: {weaknesses}"));
    }
    if !assumptions.is_empty() {
        s.push_str(&format!("\nCommon assumptions: {assumptions}"));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::introspection::Trend;
    use crate::modules::prospective::Action;
    use tempfile::TempDir;
    use tokio::io::AsyncReadExt;

    // ── Engine-driven cadence dispatch (Wave 1 item 6) ────────

    #[test]
    fn cadence_words_parse() {
        assert_eq!(parse_cadence("daily"), Some(chrono::Duration::days(1)));
        assert_eq!(parse_cadence("weekly"), Some(chrono::Duration::weeks(1)));
        assert_eq!(
            parse_cadence("every-2-days"),
            Some(chrono::Duration::days(2))
        );
        assert_eq!(
            parse_cadence("Every-12-Hours"),
            Some(chrono::Duration::hours(12))
        );
        assert_eq!(parse_cadence("manifest"), None);
        assert_eq!(parse_cadence(""), None);
    }

    /// Build a throwaway external domain whose consolidate script appends a
    /// line to `ran.log`, exiting with `exit_code`.
    fn test_external_domain(root: &Path, exit_code: i32) -> crate::modules::external_domain::ExternalDomain {
        use std::os::unix::fs::PermissionsExt;
        let script = root.join("consolidate.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho ran >> {}\nexit {exit_code}\n",
                root.join("ran.log").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(root.join("events.jsonl"), "").unwrap();
        let manifest_path = root.join("dom.toml");
        std::fs::write(
            &manifest_path,
            format!(
                r#"
[domain]
name = "testdom"
version = "0"
description = "cadence-dispatch test domain"
root = "{root}"

[event_stream]
path = "{root}/events.jsonl"
format = "jsonl"
id_field = "id"
ts_field = "ts"

[consolidation]
enabled = true
type = "external_script"
script = "{root}/consolidate.sh"
cadence = "daily"
timeout = "10s"
"#,
                root = root.display()
            ),
        )
        .unwrap();
        let m = crate::modules::external_domain::load_manifest(&manifest_path).unwrap();
        crate::modules::external_domain::ExternalDomain::from_manifest(m).unwrap()
    }

    #[test]
    fn dispatch_runs_due_domain_then_waits_out_its_cadence() {
        let dir = TempDir::new().unwrap();
        let ed = test_external_domain(dir.path(), 0);
        let registry = DomainRegistry::from_domains(vec![Box::new(ed)]);
        let mut state = DomainCadenceState::default();
        let t0 = Utc::now();

        // No prior stamp → due → runs.
        assert_eq!(dispatch_due_consolidations(&registry, &mut state, t0), (1, 0));
        // Immediately again → inside the daily cadence → skipped.
        assert_eq!(dispatch_due_consolidations(&registry, &mut state, t0), (0, 0));
        // A day and change later → due again.
        let t1 = t0 + chrono::Duration::hours(25);
        assert_eq!(dispatch_due_consolidations(&registry, &mut state, t1), (1, 0));
        let runs = std::fs::read_to_string(dir.path().join("ran.log")).unwrap();
        assert_eq!(runs.lines().count(), 2);
    }

    #[test]
    fn dispatch_failure_leaves_stamp_unset_for_retry() {
        let dir = TempDir::new().unwrap();
        let ed = test_external_domain(dir.path(), 1);
        let registry = DomainRegistry::from_domains(vec![Box::new(ed)]);
        let mut state = DomainCadenceState::default();
        assert_eq!(
            dispatch_due_consolidations(&registry, &mut state, Utc::now()),
            (0, 1)
        );
        assert!(
            state.last_run.is_empty(),
            "failed run must not advance the stamp"
        );
    }

    // Live one-shot: boot the REAL registry and run pinned's consolidation —
    // the docs/24 item-6 validation target (pins finally decay, active.md
    // regenerates). Ignored by default: it touches live pin data, with the
    // same effects as one daemon dispatch.
    // Run: cargo test dispatch_pinned_consolidation_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn dispatch_pinned_consolidation_live() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let config = Config::default();
        let store = Store::new(home.join(".claude/subconscious")).unwrap();
        let registry = DomainRegistry::boot(&config, &store);
        let pinned = registry
            .get("pinned")
            .expect("pinned domain must be discovered from its inline manifest");
        let report = pinned.consolidate().expect("pinned consolidation runs");
        println!(
            "pinned consolidated: events={}, runtime={}ms, note={:?}",
            report.events_processed, report.runtime_ms, report.note
        );
        let decay = home.join(".claude/pinned/_decay-state.json");
        let age = std::fs::metadata(&decay)
            .and_then(|m| m.modified())
            .map(|t| {
                std::time::SystemTime::now()
                    .duration_since(t)
                    .unwrap_or_default()
            })
            .expect("decay state exists");
        assert!(
            age.as_secs() < 300,
            "decay state should be freshly rewritten, is {}s old",
            age.as_secs()
        );
    }

    #[test]
    fn dispatch_skips_native_domains() {
        struct Stub;
        impl crate::modules::Module for Stub {
            fn should_run(&self) -> Result<bool> {
                Ok(false)
            }
            fn run(
                &self,
                _client: &ClaudeClient,
                _budget_tokens: u64,
            ) -> impl std::future::Future<Output = Result<u64>> + Send {
                async { Ok(0) }
            }
        }
        let registry = DomainRegistry::from_domains(vec![Box::new(
            crate::modules::NativeAdapter::new("native-stub", Stub),
        )]);
        let mut state = DomainCadenceState::default();
        assert_eq!(
            dispatch_due_consolidations(&registry, &mut state, Utc::now()),
            (0, 0)
        );
        assert!(state.last_run.is_empty());
    }

    // ── Socket listener end-to-end ────────────────────────────
    // This is the only test in the project that actually spins up
    // a real Unix socket. It verifies the full round-trip of the
    // hook-script protocol:
    //   client writes a JSON line → daemon parses → event lands
    //   in logs/events.jsonl with a daemon-side timestamp.
    //
    // If this breaks, hook-to-daemon communication is dead even
    // though the event schema tests still pass.

    #[tokio::test]
    async fn handle_hook_connection_persists_event_to_jsonl() {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        // Bind a throwaway socket inside the tempdir
        let socket_path = dir.path().join("test.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        // Client task: connect and write a real session_start payload.
        // We read the response to EOF as the sync point — explicit
        // shutdown() races with the server's close on macOS.
        let client_path = socket_path.clone();
        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&client_path).await.unwrap();
            let payload = r#"{"event":"session_start","ts":42}"#;
            stream.write_all(payload.as_bytes()).await.unwrap();
            stream.write_all(b"\n").await.unwrap();
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf).await;
        });

        // Server side: accept + handle
        let (stream, _) = listener.accept().await.unwrap();
        handle_hook_connection(stream, &store).await.unwrap();
        client.await.unwrap();

        // Verify persistence
        let records: Vec<HookEventRecord> = store.read_jsonl(EVENTS_LOG).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].event,
            HookEvent::SessionStart { ts: 42, cwd: None }
        );
    }

    #[tokio::test]
    async fn handle_hook_connection_rejects_malformed_payload() {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let socket_path = dir.path().join("bad.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let client_path = socket_path.clone();
        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&client_path).await.unwrap();
            // Not valid JSON for any HookEvent variant
            stream.write_all(b"not json\n").await.unwrap();
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf).await;
        });

        let (stream, _) = listener.accept().await.unwrap();
        let result = handle_hook_connection(stream, &store).await;
        client.await.unwrap();

        assert!(result.is_err(), "Bad payload should produce an error");
        // And nothing should have been written
        assert_eq!(store.count_jsonl(EVENTS_LOG).unwrap(), 0);
    }

    #[tokio::test]
    async fn handle_hook_connection_handles_multiple_events_in_sequence() {
        // The listener calls handle_hook_connection once per accept.
        // This test verifies that multiple sequential events all land
        // in order — the order guarantee is what lets the metacog
        // module correlate tool_use events with their session bounds.
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let socket_path = dir.path().join("seq.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let payloads = [
            r#"{"event":"session_start","ts":100}"#,
            r#"{"event":"tool_use","tool":"Read","ts":101}"#,
            r#"{"event":"tool_use","tool":"Edit","ts":102}"#,
            r#"{"event":"session_end","ts":103}"#,
        ];

        for payload in payloads {
            let client_path = socket_path.clone();
            let payload_owned = payload.to_string();
            let client = tokio::spawn(async move {
                let mut stream = UnixStream::connect(&client_path).await.unwrap();
                stream.write_all(payload_owned.as_bytes()).await.unwrap();
                stream.write_all(b"\n").await.unwrap();
                let mut buf = Vec::new();
                let _ = stream.read_to_end(&mut buf).await;
            });
            let (stream, _) = listener.accept().await.unwrap();
            handle_hook_connection(stream, &store).await.unwrap();
            client.await.unwrap();
        }

        let records: Vec<HookEventRecord> = store.read_jsonl(EVENTS_LOG).unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(
            records[0].event,
            HookEvent::SessionStart { ts: 100, cwd: None }
        );
        assert_eq!(
            records[1].event,
            HookEvent::ToolUse {
                tool: "Read".into(),
                ts: 101
            }
        );
        assert_eq!(
            records[2].event,
            HookEvent::ToolUse {
                tool: "Edit".into(),
                ts: 102
            }
        );
        assert_eq!(records[3].event, HookEvent::SessionEnd { ts: 103 });

        // Task #6: the two tool_use events should ALSO have been sampled
        // to metacog/activity.jsonl as real-time heartbeat records. The
        // session_start/session_end events must NOT appear there.
        let activity: Vec<ToolActivitySample> = store.read_jsonl(METACOG_ACTIVITY_LOG).unwrap();
        assert_eq!(
            activity.len(),
            2,
            "Only the two tool_use events should land in the activity log"
        );
        assert_eq!(activity[0].tool, "Read");
        assert_eq!(activity[0].hook_ts, 101);
        assert_eq!(activity[1].tool, "Edit");
        assert_eq!(activity[1].hook_ts, 102);
    }

    // ── PostToolUse → metacog activity sampling (Task #6) ─────
    // Every tool_use event from the shell hook must land in
    // metacog/activity.jsonl as a lightweight heartbeat sample.
    // This is the realtime counterpart to the deep post-session
    // sampling that happens during consolidation. If this breaks,
    // metacog loses its per-tool heartbeat signal and the daemon
    // has no way to prioritize which sessions to deep-sample.

    #[tokio::test]
    async fn tool_use_writes_metacog_activity_sample() {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let socket_path = dir.path().join("tool.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let client_path = socket_path.clone();
        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&client_path).await.unwrap();
            let payload = r#"{"event":"tool_use","tool":"Grep","ts":777}"#;
            stream.write_all(payload.as_bytes()).await.unwrap();
            stream.write_all(b"\n").await.unwrap();
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf).await;
        });

        let (stream, _) = listener.accept().await.unwrap();
        let before = Utc::now();
        handle_hook_connection(stream, &store).await.unwrap();
        let after = Utc::now();
        client.await.unwrap();

        let samples: Vec<ToolActivitySample> = store.read_jsonl(METACOG_ACTIVITY_LOG).unwrap();
        assert_eq!(samples.len(), 1, "tool_use must produce exactly one sample");
        assert_eq!(samples[0].tool, "Grep");
        assert_eq!(samples[0].hook_ts, 777);
        assert!(
            samples[0].received_at >= before && samples[0].received_at <= after,
            "received_at must be set to the daemon-side receive time"
        );
    }

    #[tokio::test]
    async fn session_start_does_not_write_activity_sample() {
        // Only tool_use events produce activity samples. SessionStart
        // and SessionEnd must not pollute the activity log — they're
        // not tool heartbeats.
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let socket_path = dir.path().join("start.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let client_path = socket_path.clone();
        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&client_path).await.unwrap();
            stream
                .write_all(b"{\"event\":\"session_start\",\"ts\":1}\n")
                .await
                .unwrap();
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf).await;
        });

        let (stream, _) = listener.accept().await.unwrap();
        handle_hook_connection(stream, &store).await.unwrap();
        client.await.unwrap();

        // The events log should have the session_start event…
        assert_eq!(store.count_jsonl(EVENTS_LOG).unwrap(), 1);
        // …but the activity log should be empty / nonexistent.
        assert_eq!(store.count_jsonl(METACOG_ACTIVITY_LOG).unwrap(), 0);
    }

    #[test]
    fn bind_socket_removes_stale_file() {
        let dir = TempDir::new().unwrap();
        let socket_path = dir.path().join("stale.sock");
        // Simulate a stale socket file from a crashed previous run
        std::fs::write(&socket_path, "").unwrap();
        assert!(socket_path.exists());

        bind_socket(&socket_path).unwrap();
        assert!(!socket_path.exists(), "Stale file should be removed");
    }

    #[test]
    fn bind_socket_creates_parent_dir() {
        let dir = TempDir::new().unwrap();
        let socket_path = dir.path().join("nested/subdir/new.sock");
        assert!(!socket_path.parent().unwrap().exists());

        bind_socket(&socket_path).unwrap();
        assert!(socket_path.parent().unwrap().exists());
    }

    // ── SessionStart briefing composer ────────────────────────
    // build_session_start_response is what the hook script echoes
    // into Claude's context at session start. It has to:
    //   1. Return empty when there's nothing worth saying
    //   2. Surface time-unlocked broadcast intentions only
    //   3. Ignore keyword-gated intentions (they need a prompt to
    //      match against — SessionStart has no text)
    //   4. Surface introspection strengths/weaknesses when present
    //
    // These tests lock the minimum-signal contract: we don't want
    // the daemon injecting noise into every new session.

    fn mk_store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();
        (dir, store)
    }

    fn broadcast_intention(
        id: &str,
        message: &str,
        priority: Priority,
        after_offset: chrono::Duration,
    ) -> Intention {
        Intention {
            id: id.into(),
            trigger: Trigger::Time {
                after: Utc::now() + after_offset,
                keywords: vec![],
            },
            action: Action {
                message: message.into(),
                priority,
                source: "test".into(),
            },
            created: Utc::now() - chrono::Duration::days(1),
            expires: Utc::now() + chrono::Duration::days(7),
            fire_count: 0,
            max_fires: 5,
            last_fired: None,
        }
    }

    #[test]
    fn session_start_response_empty_when_no_data() {
        let (_dir, store) = mk_store();
        let (out, _, _) = build_session_start_response(&store, None);
        assert!(
            out.is_empty(),
            "Empty store should yield empty response, got: {out:?}"
        );
    }

    #[test]
    fn session_start_response_surfaces_broadcast_intention() {
        let (_dir, store) = mk_store();
        // after = 1 hour ago, keywords empty → broadcastable
        let intention = broadcast_intention(
            "b-1",
            "Update CHANGELOG for v0.5.0",
            Priority::High,
            chrono::Duration::hours(-1),
        );
        store
            .append_jsonl("intentions/registry.jsonl", &intention)
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        assert!(out.contains("# i-dream briefing"), "missing header: {out}");
        assert!(
            out.contains("## Behavioral rules (1)"),
            "missing section: {out}"
        );
        assert!(out.contains("[high]"), "missing priority tag: {out}");
        assert!(
            out.contains("Update CHANGELOG for v0.5.0"),
            "missing message: {out}"
        );
    }

    #[test]
    fn session_start_response_skips_future_gated_intention() {
        let (_dir, store) = mk_store();
        // after is 1 hour in the future → NOT broadcastable yet
        let intention = broadcast_intention(
            "future-1",
            "Scheduled reminder",
            Priority::Medium,
            chrono::Duration::hours(1),
        );
        store
            .append_jsonl("intentions/registry.jsonl", &intention)
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        assert!(
            out.is_empty(),
            "Future-gated time intentions should not fire at session start: {out:?}"
        );
    }

    #[test]
    fn session_start_response_skips_keyword_gated_triggers() {
        let (_dir, store) = mk_store();
        // Event trigger with keywords — can't match SessionStart (no text)
        let event_intention = Intention {
            id: "evt-1".into(),
            trigger: Trigger::Event {
                condition: "kw".into(),
                keywords: vec!["deploy".into()],
                file_patterns: vec![],
            },
            action: Action {
                message: "Check deploy config".into(),
                priority: Priority::High,
                source: "test".into(),
            },
            created: Utc::now(),
            expires: Utc::now() + chrono::Duration::days(7),
            fire_count: 0,
            max_fires: 3,
            last_fired: None,
        };
        store
            .append_jsonl("intentions/registry.jsonl", &event_intention)
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        assert!(
            out.is_empty(),
            "Keyword-gated intentions need a prompt to match — must not surface at session start: {out:?}"
        );
    }

    #[test]
    fn session_start_response_sorts_intentions_by_priority() {
        let (_dir, store) = mk_store();
        // Insert in low → high → medium order; expected order in
        // output is high, medium, low.
        let ago = chrono::Duration::hours(-1);
        store
            .append_jsonl(
                "intentions/registry.jsonl",
                &broadcast_intention("low-1", "Low thing", Priority::Low, ago),
            )
            .unwrap();
        store
            .append_jsonl(
                "intentions/registry.jsonl",
                &broadcast_intention("high-1", "High thing", Priority::High, ago),
            )
            .unwrap();
        store
            .append_jsonl(
                "intentions/registry.jsonl",
                &broadcast_intention("med-1", "Medium thing", Priority::Medium, ago),
            )
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        let high_pos = out.find("High thing").expect("high missing");
        let med_pos = out.find("Medium thing").expect("medium missing");
        let low_pos = out.find("Low thing").expect("low missing");
        assert!(high_pos < med_pos, "High should precede Medium");
        assert!(med_pos < low_pos, "Medium should precede Low");
    }

    #[test]
    fn session_start_response_skips_expired_and_maxed_intentions() {
        let (_dir, store) = mk_store();
        // Expired
        let mut expired = broadcast_intention(
            "exp-1",
            "Expired broadcast",
            Priority::High,
            chrono::Duration::hours(-1),
        );
        expired.expires = Utc::now() - chrono::Duration::days(1);
        store
            .append_jsonl("intentions/registry.jsonl", &expired)
            .unwrap();

        // Max-fired
        let mut maxed = broadcast_intention(
            "max-1",
            "Already fired out",
            Priority::High,
            chrono::Duration::hours(-1),
        );
        maxed.fire_count = 5;
        maxed.max_fires = 5;
        store
            .append_jsonl("intentions/registry.jsonl", &maxed)
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        assert!(
            out.is_empty(),
            "Expired and maxed intentions must not surface: {out:?}"
        );
    }

    #[test]
    fn session_start_response_surfaces_introspection_patterns() {
        let (_dir, store) = mk_store();
        let patterns = ReasoningPatterns {
            last_updated: Utc::now(),
            average_depth: 4.0,
            average_breadth: 2.5,
            fixation_rate: 0.1,
            assumption_rate: 0.2,
            overconfidence_rate: 0.15,
            common_assumptions: vec!["file exists".into(), "API is stable".into()],
            strength_patterns: vec!["methodical search".into()],
            weakness_patterns: vec!["premature optimization".into()],
            trend: Trend {
                calibration_improving: true,
                depth_trend: "stable".into(),
                breadth_trend: "stable".into(),
            },
        };
        store
            .write_json("introspection/patterns.json", &patterns)
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        assert!(out.contains("## Self-awareness"), "missing section: {out}");
        assert!(out.contains("methodical search"), "missing strength: {out}");
        assert!(
            out.contains("premature optimization"),
            "missing weakness: {out}"
        );
        assert!(out.contains("file exists"), "missing assumption: {out}");
    }

    #[test]
    fn session_start_response_combines_all_sections() {
        let (_dir, store) = mk_store();
        // One broadcast intention + a patterns file
        let intention = broadcast_intention(
            "combo-1",
            "Weekly review",
            Priority::Medium,
            chrono::Duration::hours(-1),
        );
        store
            .append_jsonl("intentions/registry.jsonl", &intention)
            .unwrap();

        let patterns = ReasoningPatterns {
            last_updated: Utc::now(),
            average_depth: 3.0,
            average_breadth: 2.0,
            fixation_rate: 0.0,
            assumption_rate: 0.0,
            overconfidence_rate: 0.0,
            common_assumptions: vec![],
            strength_patterns: vec!["incremental verification".into()],
            weakness_patterns: vec![],
            trend: Trend {
                calibration_improving: true,
                depth_trend: "stable".into(),
                breadth_trend: "stable".into(),
            },
        };
        store
            .write_json("introspection/patterns.json", &patterns)
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        assert!(
            out.contains("## Behavioral rules"),
            "missing behavioral rules: {out}"
        );
        assert!(
            out.contains("## Self-awareness"),
            "missing self-awareness: {out}"
        );
        assert!(out.contains("Weekly review"));
        assert!(out.contains("incremental verification"));
    }

    #[test]
    fn session_start_response_returns_surfaced_intention_ids() {
        let (_dir, store) = mk_store();
        let ago = chrono::Duration::hours(-1);
        store
            .append_jsonl(
                "intentions/registry.jsonl",
                &broadcast_intention("surf-a", "Rule A", Priority::High, ago),
            )
            .unwrap();
        store
            .append_jsonl(
                "intentions/registry.jsonl",
                &broadcast_intention("surf-b", "Rule B", Priority::Low, ago),
            )
            .unwrap();

        let (out, ids, has_intro) = build_session_start_response(&store, None);
        assert!(!out.is_empty());
        assert_eq!(ids.len(), 2, "expected 2 surfaced IDs, got: {ids:?}");
        assert!(ids.contains(&"surf-a".to_string()));
        assert!(ids.contains(&"surf-b".to_string()));
        assert!(!has_intro, "no introspection patterns → false");
    }

    #[test]
    fn session_start_response_flags_introspection() {
        let (_dir, store) = mk_store();
        let patterns = ReasoningPatterns {
            last_updated: Utc::now(),
            average_depth: 3.0,
            average_breadth: 2.0,
            fixation_rate: 0.0,
            assumption_rate: 0.0,
            overconfidence_rate: 0.0,
            common_assumptions: vec!["X".into()],
            strength_patterns: vec![],
            weakness_patterns: vec![],
            trend: Trend {
                calibration_improving: true,
                depth_trend: "stable".into(),
                breadth_trend: "stable".into(),
            },
        };
        store
            .write_json("introspection/patterns.json", &patterns)
            .unwrap();

        let (_out, ids, has_intro) = build_session_start_response(&store, None);
        assert!(ids.is_empty(), "no intentions → empty ids");
        assert!(has_intro, "introspection patterns present → true");
    }

    #[test]
    fn session_start_response_empty_patterns_contribute_nothing() {
        let (_dir, store) = mk_store();
        // Patterns file exists but every surfaceable field is empty
        let patterns = ReasoningPatterns {
            last_updated: Utc::now(),
            average_depth: 3.0,
            average_breadth: 2.0,
            fixation_rate: 0.0,
            assumption_rate: 0.0,
            overconfidence_rate: 0.0,
            common_assumptions: vec![],
            strength_patterns: vec![],
            weakness_patterns: vec![],
            trend: Trend {
                calibration_improving: true,
                depth_trend: "stable".into(),
                breadth_trend: "stable".into(),
            },
        };
        store
            .write_json("introspection/patterns.json", &patterns)
            .unwrap();

        let (out, _, _) = build_session_start_response(&store, None);
        assert!(
            out.is_empty(),
            "Patterns with no surfaceable content should not produce a section: {out:?}"
        );
    }

    // ── Daemon process management hardening (Task #7) ──────────
    // These tests cover the PID-file helpers and the liveness probe
    // without actually forking or signaling real daemons. The rule
    // is: never signal a stale PID (it may have been recycled), and
    // never overwrite a live daemon's PID file.

    #[test]
    fn is_process_alive_reports_true_for_self() {
        // Our own PID must always be alive from our point of view.
        // If this ever returns false, the probe is broken.
        let my_pid = std::process::id() as i32;
        assert!(
            is_process_alive(my_pid),
            "is_process_alive({my_pid}) returned false for current process",
        );
    }

    #[test]
    fn is_process_alive_reports_false_for_nonexistent_pid() {
        // PID 0x7FFF_FFFF is outside any realistic PID range on Linux
        // and macOS (both cap at well below 2^31-1), so it should
        // always read as dead. Also check a few zero/negative guards.
        assert!(!is_process_alive(i32::MAX));
        assert!(!is_process_alive(0));
        assert!(!is_process_alive(-1));
    }

    #[test]
    fn read_pid_file_returns_none_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.pid");
        assert_eq!(read_pid_file(&path), None);
    }

    #[test]
    fn read_pid_file_parses_integer_contents() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ok.pid");
        std::fs::write(&path, "12345\n").unwrap();
        assert_eq!(read_pid_file(&path), Some(12345));
    }

    #[test]
    fn read_pid_file_returns_none_for_corrupt_contents() {
        // A garbled PID file mustn't crash the daemon or cause a
        // parse error — it's treated as "no daemon, clean to start".
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.pid");
        std::fs::write(&path, "not-a-pid").unwrap();
        assert_eq!(read_pid_file(&path), None);
    }

    #[test]
    fn write_pid_file_creates_parent_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested/subdir/daemon.pid");
        assert!(!path.parent().unwrap().exists());
        write_pid_file(&path, 42).unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "42");
    }

    #[test]
    fn write_pid_file_is_atomic_via_rename() {
        // We can't directly observe the rename from outside, but we
        // can at least verify that the tmp file is cleaned up after
        // a successful write.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("atomic.pid");
        write_pid_file(&path, 99).unwrap();
        let tmp = path.with_extension("pid.tmp");
        assert!(!tmp.exists(), "tmp file should have been renamed away");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "99");
    }

    #[tokio::test]
    async fn wait_for_exit_returns_immediately_for_dead_pid() {
        // With a nonexistent PID, wait_for_exit should return true
        // on the first iteration without consuming the full budget.
        let start = std::time::Instant::now();
        let result = wait_for_exit(i32::MAX, 50, Duration::from_millis(100)).await;
        assert!(result, "wait_for_exit should see a nonexistent PID as dead");
        // Should be near-instant — if this took the full 5 s budget,
        // the early-return branch is broken.
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "wait_for_exit took too long for a dead pid: {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn wait_for_exit_returns_false_when_pid_stays_alive() {
        let my_pid = std::process::id() as i32;
        let start = std::time::Instant::now();
        let result = wait_for_exit(my_pid, 3, Duration::from_millis(20)).await;
        assert!(!result, "wait_for_exit should time out on a live pid");
        // Budget is 3 × 20 ms = 60 ms minimum; allow generous slop.
        assert!(start.elapsed() >= Duration::from_millis(55));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    // ── DaemonState persistence ───────────────────────────────
    //
    // `Config::data_dir()` is hardcoded to `~/.claude/subconscious`,
    // so `Daemon::new()` can't be routed to a tempdir via config.
    // Instead we build the `Daemon` struct directly — all fields are
    // private but accessible from this same-module test.

    fn mk_daemon_with_store(store: Store) -> Daemon {
        Daemon {
            config: Config::default(),
            store,
            state: Mutex::new(DaemonState::default()),
            client: None,
            cycle_in_progress: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn update_state_adopts_disk_progress_before_writing() {
        // The 1311→1310 step-back class: a sibling daemon generation's
        // in-flight cycle wrote newer values to disk; this instance's
        // stale in-memory snapshot (here: the default zeros) must adopt
        // that progress instead of clobbering it — even on a no-op flush
        // like the SIGTERM shutdown's update_state(|_| {}).
        let (dir, store) = mk_store();
        let sibling_write = DaemonState {
            total_cycles: 1311,
            total_tokens_used: 999,
            last_consolidation: Some(Utc::now()),
            last_activity: None,
            usage: None,
        };
        store.write_json("state.json", &sibling_write).unwrap();
        let daemon = mk_daemon_with_store(store);

        // The shutdown-flush shape: mutate nothing, just persist.
        daemon.update_state(|_| {}).unwrap();
        let reloaded: DaemonState = daemon.store.read_json("state.json").unwrap();
        assert_eq!(reloaded.total_cycles, 1311, "no-op flush must not step back");
        assert_eq!(reloaded.total_tokens_used, 999);
        assert!(reloaded.last_consolidation.is_some());

        // And an increment lands ON TOP of the adopted progress, so the
        // next cycle gets a fresh number instead of reusing 1311.
        daemon.update_state(|s| s.total_cycles += 1).unwrap();
        let reloaded: DaemonState = daemon.store.read_json("state.json").unwrap();
        assert_eq!(reloaded.total_cycles, 1312);
        drop(dir);
    }

    #[test]
    fn merge_newer_takes_the_larger_of_each_field() {
        let earlier = Utc::now() - chrono::Duration::hours(48);
        let later = Utc::now();
        let mut mem = DaemonState {
            total_cycles: 1310,
            total_tokens_used: 100,
            last_consolidation: Some(earlier),
            last_activity: Some(later),
            usage: None,
        };
        let disk = DaemonState {
            total_cycles: 1311,
            total_tokens_used: 50,
            last_consolidation: Some(later),
            last_activity: Some(earlier),
            usage: None,
        };
        mem.merge_newer(disk);
        assert_eq!(mem.total_cycles, 1311, "disk was ahead on cycles");
        assert_eq!(mem.total_tokens_used, 100, "memory was ahead on tokens");
        assert_eq!(mem.last_consolidation, Some(later));
        assert_eq!(mem.last_activity, Some(later));
    }

    #[test]
    fn update_state_persists_to_disk() {
        // update_state is the single write-path for state.json — if
        // this roundtrip breaks, total_cycles will silently reset to
        // zero on every daemon restart and status will be useless.
        let (dir, store) = mk_store();
        let daemon = mk_daemon_with_store(store);

        daemon
            .update_state(|s| {
                s.total_cycles = 7;
                s.total_tokens_used = 12345;
                s.last_consolidation = Some(Utc::now());
            })
            .unwrap();

        // Read it back from disk, not from the in-memory field.
        let reloaded: DaemonState = daemon.store.read_json("state.json").unwrap();
        assert_eq!(reloaded.total_cycles, 7);
        assert_eq!(reloaded.total_tokens_used, 12345);
        assert!(reloaded.last_consolidation.is_some());
        drop(dir);
    }

    #[test]
    fn update_state_accumulates_across_calls() {
        // Each call replaces the snapshot on disk. Accumulated counters
        // like total_cycles need to be additive across calls — we test
        // this by calling update_state twice and checking the final
        // disk state is the sum, not just the last call's values.
        let (_dir, store) = mk_store();
        let daemon = mk_daemon_with_store(store);

        daemon.update_state(|s| s.total_cycles += 1).unwrap();
        daemon.update_state(|s| s.total_cycles += 1).unwrap();
        daemon.update_state(|s| s.total_cycles += 3).unwrap();

        let reloaded: DaemonState = daemon.store.read_json("state.json").unwrap();
        assert_eq!(reloaded.total_cycles, 5);
    }

    #[test]
    fn touch_last_activity_updates_memory_without_disk_write() {
        // touch_last_activity is on the hot path (every hook event),
        // so it must NOT touch the disk. We verify by reading the
        // mutex directly and confirming state.json is absent.
        let (_dir, store) = mk_store();
        let daemon = mk_daemon_with_store(store);

        // state.json does not yet exist (fresh store).
        assert!(!daemon.store.exists("state.json"));

        daemon.touch_last_activity();

        // Still no state.json on disk.
        assert!(
            !daemon.store.exists("state.json"),
            "touch_last_activity must not write to disk"
        );
        // But in-memory state has been updated.
        let state = daemon.state.lock().unwrap();
        assert!(state.last_activity.is_some());
    }

    // ── D8/D19 daemon-hook coverage ───────────────────────────
    // The CLI paths are tested separately; these verify the daemon
    // wrappers preserve the same behavior contract:
    //   - cycle_drift_warnings: pure read, no mutation
    //   - weekly_auto_prune_patterns: writes backup + state.json,
    //     no-ops on second run within the same ISO week.

    use crate::modules::dreaming::{Association, ExtractedPattern};

    fn make_pattern(
        id: &str,
        conf: f64,
        last_seen: chrono::DateTime<chrono::Utc>,
    ) -> ExtractedPattern {
        ExtractedPattern {
            id: id.into(),
            pattern: format!("test pattern {id}"),
            valence: "neutral".into(),
            confidence: conf,
            category: "domain".into(),
            occurrences: 1,
            first_seen: last_seen.to_rfc3339(),
            last_seen: last_seen.to_rfc3339(),
            source_sessions: vec![],
            source_projects: vec![],
            occurrence_history: vec![],
            strength: conf,
            ease: 2.5,
            reactivations: 0,
        }
    }

    #[test]
    fn d19_cycle_drift_warnings_runs_without_panic_on_empty_store() {
        // Smoke test: zero patterns → no warnings emitted, no panic.
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();
        // Should be a no-op (and never error) on an empty store.
        Daemon::cycle_drift_warnings(&store).unwrap();
    }

    #[test]
    fn d17_weekly_auto_prune_writes_backup_and_state() {
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let now = chrono::Utc::now();
        let recent = now - chrono::Duration::days(5);
        let dormant = now - chrono::Duration::days(90);
        let patterns = vec![
            make_pattern("keep-recent", 0.30, recent), // dormant rule fails → keep
            make_pattern("keep-confident", 0.80, dormant), // confidence rule fails → keep
            make_pattern("prune-me", 0.20, dormant),   // both rules trigger → prune
        ];
        store.write_json("dreams/patterns.json", &patterns).unwrap();

        Daemon::weekly_auto_prune_patterns(&store).unwrap();

        // Patterns.json should have the two survivors.
        let after: Vec<ExtractedPattern> = store.read_json("dreams/patterns.json").unwrap();
        assert_eq!(after.len(), 2);
        assert!(after.iter().all(|p| p.id != "prune-me"));

        // State file written; re-running same week should be a no-op.
        assert!(store.exists("dreams/auto-prune-state.json"));
        let state_path = store.path("dreams/auto-prune-state.json");
        let mtime_before = std::fs::metadata(&state_path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        Daemon::weekly_auto_prune_patterns(&store).unwrap();
        let mtime_after = std::fs::metadata(&state_path).unwrap().modified().unwrap();
        // Same ISO week → state file untouched (no rewrite).
        assert_eq!(
            mtime_before, mtime_after,
            "second call within same ISO week should not rewrite state file",
        );
    }

    #[test]
    fn d8_cycle_auto_intentions_idempotent_via_auto_intention_id() {
        // Daemon-side wrapper: an eligible association gets promoted on
        // first call, but a second call doesn't duplicate it.
        let dir = TempDir::new().unwrap();
        let store = Store::new(dir.path().to_path_buf()).unwrap();
        store.init_dirs().unwrap();

        let now = chrono::Utc::now();
        let pat = make_pattern("p1", 0.95, now);
        store
            .write_json("dreams/patterns.json", &vec![pat])
            .unwrap();
        let assoc = Association {
            id: "a1".into(),
            patterns_linked: vec!["p1".into()],
            hypothesis: "test".into(),
            confidence: 0.95,
            actionable: true,
            suggested_rule: Some("prefer X over Y when Z".into()),
            promoted: true,
            dismissed: false,
            auto_intention_id: None,
        };
        store
            .write_json("dreams/associations.json", &vec![assoc])
            .unwrap();

        let daemon = mk_daemon_with_store(store.clone());

        // First call promotes.
        daemon.cycle_auto_intentions().unwrap();
        let after1: Vec<Association> = store.read_json("dreams/associations.json").unwrap();
        assert!(
            after1[0].auto_intention_id.is_some(),
            "first call should set auto_intention_id"
        );
        let intentions_count_1 = store
            .read_jsonl::<crate::modules::prospective::Intention>("intentions/registry.jsonl")
            .unwrap_or_default()
            .len();
        assert_eq!(intentions_count_1, 1);

        // Second call is a no-op — auto_intention_id already set.
        daemon.cycle_auto_intentions().unwrap();
        let intentions_count_2 = store
            .read_jsonl::<crate::modules::prospective::Intention>("intentions/registry.jsonl")
            .unwrap_or_default()
            .len();
        assert_eq!(intentions_count_2, 1, "second call must not duplicate");
    }
}
