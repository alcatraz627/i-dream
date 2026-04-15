//! Daemon lifecycle — start, stop, status, idle detection, consolidation orchestration.

use crate::api::ClaudeClient;
use crate::cli::DreamPhase;
use crate::config::Config;
use crate::dream_trace::{DreamTracer, EventKind, Phase as TracePhase};
use crate::events::{HookEvent, HookEventRecord};
use crate::modules::{
    dreaming::DreamingModule,
    introspection::{IntrospectionModule, ReasoningPatterns},
    metacog::{MetacogModule, ToolActivitySample},
    prospective::{Intention, Priority, ProspectiveModule, Trigger},
    Module,
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
use tokio::signal::unix::{signal as unix_signal, SignalKind};
use tracing::{debug, error, info, warn};

/// Relative path (under the data dir) of the hook-event log.
const EVENTS_LOG: &str = "logs/events.jsonl";

/// Dedicated log for UserSignal events from the UserPromptSubmit hook.
/// Separate from EVENTS_LOG so the dreaming module can scan sentiment
/// trends without filtering the general event stream.
const SIGNALS_LOG: &str = "logs/signals.jsonl";

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
            Some(ClaudeClient::new_subprocess(&config.budget.claude_code_cli_path))
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
    fn update_state<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut DaemonState),
    {
        let snapshot = {
            let mut state = self
                .state
                .lock()
                .map_err(|e| anyhow::anyhow!("daemon state mutex poisoned: {e}"))?;
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
    pub async fn run_foreground(&self) -> Result<()> {
        info!("i-dream daemon running in foreground (Ctrl+C to stop)");

        let check_interval =
            Duration::from_secs(self.config.idle.check_interval_minutes * 60);

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
        let mut sigterm = unix_signal(SignalKind::terminate())
            .context("Failed to install SIGTERM handler")?;

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
                }
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _addr)) => {
                            // Touch in-memory activity before handling —
                            // we count "connection received" as activity
                            // whether or not the event parses.
                            self.touch_last_activity();
                            if let Err(e) = handle_hook_connection(stream, &self.store).await {
                                warn!("Hook event handler failed: {e:#}");
                            }
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

        // Flush the in-memory state snapshot one last time so `status`
        // sees the final `last_activity` after a graceful SIGTERM.
        if let Err(e) = self.update_state(|_| {}) {
            debug!("Failed to persist state on shutdown: {e:#}");
        }

        info!("Daemon stopped");
        result
    }

    /// Acquire the PID file and run in the foreground.
    ///
    /// Despite the name, this function does NOT fork. It writes our PID
    /// into `daemon.pid`, runs the foreground loop to completion, and
    /// then cleans up the PID file on the way out. The backgrounding
    /// (nohup/launchd/systemd) is delegated to whatever supervisor
    /// starts the process.
    ///
    /// Refuses to start if `daemon.pid` points at a still-alive
    /// process — otherwise we'd end up with two daemons racing on the
    /// same Unix socket. A stale PID file (process is dead) is
    /// silently cleaned.
    pub async fn daemonize(&self) -> Result<()> {
        let pid_path = self.config.data_dir().join("daemon.pid");

        match read_pid_file(&pid_path) {
            Some(existing) if is_process_alive(existing) => {
                anyhow::bail!(
                    "Daemon already running (PID {existing}). \
                     Run `i-dream stop` first, or remove {} if you're sure it's stale.",
                    pid_path.display()
                );
            }
            Some(existing) => {
                warn!(
                    "Removing stale PID file at {} (PID {existing} is not alive)",
                    pid_path.display()
                );
                let _ = std::fs::remove_file(&pid_path);
            }
            None => {}
        }

        let pid = std::process::id();
        write_pid_file(&pid_path, pid)?;
        info!("Daemon started with PID {pid}");

        let result = self.run_foreground().await;

        // Always attempt to clean the PID file on exit — whether the
        // foreground loop returned Ok or Err. Best-effort: if the file
        // already vanished (someone ran `i-dream stop`), that's fine.
        if let Err(e) = std::fs::remove_file(&pid_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                debug!("Failed to remove PID file on shutdown: {e}");
            }
        }

        result
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

        let idle_duration = Utc::now() - last_activity;
        let threshold = chrono::Duration::hours(self.config.idle.threshold_hours as i64);

        Ok(idle_duration > threshold)
    }

    /// Run the full consolidation cycle, respecting budget and timeouts.
    async fn run_consolidation(&self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .context("API client unavailable — set ANTHROPIC_API_KEY or enable budget.use_claude_code_cli")?;

        let mut budget = self.config.budget.max_tokens_per_cycle;
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(self.config.budget.max_runtime_minutes * 60);

        info!("Starting consolidation (budget: {budget} tokens, deadline: {}min)",
            self.config.budget.max_runtime_minutes);

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

        // Phase 4: Housekeeping (no API budget)
        let prospective = ProspectiveModule::new(&self.config, &self.store);
        prospective.cleanup_expired()?;

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

        Ok(())
    }

    /// Manually trigger a dream cycle.
    ///
    /// The `All` case delegates to `module.run`, which owns its own
    /// tracer. Single-phase runs (`Sws`/`Rem`/`Wake`) create a tracer
    /// here and bracket the call with CycleStart/CycleEnd so their
    /// trace files look structurally identical to a full-cycle trace on
    /// the dashboard.
    pub async fn run_dream(&self, phase: DreamPhase) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .context("API client unavailable — set ANTHROPIC_API_KEY or enable budget.use_claude_code_cli")?;

        let module = DreamingModule::new(&self.config, &self.store);
        let budget = self.config.budget.max_tokens_per_cycle;

        match phase {
            DreamPhase::All => {
                module.run(client, budget).await?;
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
            }
        }

        Ok(())
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

    /// Get daemon status.
    ///
    /// Distinguishes three cases for the PID file:
    ///   - no file → "stopped"
    ///   - file exists and PID is alive → "running"
    ///   - file exists but PID is dead → "stopped (stale PID file)",
    ///     so operators can see something is wrong without having to
    ///     `ps` themselves.
    pub async fn status() -> Result<String> {
        let pid_path = pid_file_path();
        let data_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".claude/subconscious");

        let mut out = String::new();

        // Daemon status — verify liveness, don't trust the file alone.
        match read_pid_file(&pid_path) {
            Some(pid) if is_process_alive(pid) => {
                out.push_str(&format!("Daemon: running (PID {pid})\n"));
            }
            Some(pid) => {
                out.push_str(&format!(
                    "Daemon: stopped (stale PID file at {}, PID {pid} is not alive)\n",
                    pid_path.display()
                ));
            }
            None => {
                out.push_str("Daemon: stopped\n");
            }
        }

        // State
        let state_path = data_dir.join("state.json");
        if state_path.exists() {
            let content = std::fs::read_to_string(&state_path)?;
            let state: DaemonState = serde_json::from_str(&content)?;
            if let Some(last) = state.last_consolidation {
                out.push_str(&format!("Last consolidation: {last}\n"));
            }
            if let Some(activity) = state.last_activity {
                out.push_str(&format!("Last activity: {activity}\n"));
            }
            out.push_str(&format!("Total cycles: {}\n", state.total_cycles));
            out.push_str(&format!("Total tokens used: {}\n", state.total_tokens_used));
        }

        // Module health
        let modules = ["dreams", "metacog", "valence", "introspection", "intentions"];
        out.push_str("\nModules:\n");
        for module in &modules {
            let dir = data_dir.join(module);
            let status = if dir.exists() { "initialized" } else { "not initialized" };
            out.push_str(&format!("  {module}: {status}\n"));
        }

        Ok(out)
    }
}

/// Read and parse the daemon PID file, returning `None` if the file
/// doesn't exist or the contents are unparseable. Broken contents get
/// logged but produce `None` so callers can treat it as "no daemon".
fn read_pid_file(path: &Path) -> Option<i32> {
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
fn is_process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Safety: kill(2) with sig=0 is always safe — it performs checks
    // but delivers no signal, and has no side effects on the target.
    unsafe { libc::kill(pid, 0) == 0 }
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

fn pid_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude/subconscious/daemon.pid")
}

/// Remove any stale socket file before binding.
///
/// Unix socket files persist on disk — if a previous daemon crashed
/// without cleaning up, `bind()` will fail with `EADDRINUSE`. Removing
/// the file is safe because we're the only writer and the old process
/// is gone (otherwise the PID-file check in `stop()` would have caught it).
fn bind_socket(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        std::fs::remove_file(path).with_context(|| {
            format!("Failed to remove stale socket at {}", path.display())
        })?;
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

    // Read a single line — the hook scripts send exactly one JSON object.
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line).await?;
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
    if let HookEvent::UserSignal { .. } = &event {
        if let Err(e) = store.append_jsonl(SIGNALS_LOG, &record) {
            warn!("Failed to write user signal to signals log: {e:#}");
        }
    }

    // SessionStart is the only event that gets a non-empty response —
    // the hook script echoes whatever we write back into Claude's context.
    // For all other events we just ack with an empty body.
    let response = match &event {
        HookEvent::SessionStart { .. } => build_session_start_response(store),
        _ => String::new(),
    };
    writer.write_all(response.as_bytes()).await?;
    writer.shutdown().await?;

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
fn build_session_start_response(store: &Store) -> String {
    let mut sections: Vec<String> = Vec::new();

    // ── 1. Broadcast intentions ─────────────────────────────
    if let Some(section) = broadcast_intentions_section(store) {
        sections.push(section);
    }

    // ── 2. Introspection patterns ───────────────────────────
    if let Some(section) = introspection_patterns_section(store) {
        sections.push(section);
    }

    if sections.is_empty() {
        return String::new();
    }

    let mut out = String::from("# i-dream briefing\n\n");
    out.push_str(&sections.join("\n\n"));
    out.push('\n');
    out
}

/// Filter the intention registry to "broadcast-ready" entries — those
/// that can fire without needing to match against a user prompt.
fn broadcast_intentions_section(store: &Store) -> Option<String> {
    let registry: Vec<Intention> = store
        .read_jsonl("intentions/registry.jsonl")
        .unwrap_or_default();
    if registry.is_empty() {
        return None;
    }

    let now = Utc::now();
    let mut broadcast: Vec<&Intention> = registry
        .iter()
        .filter(|intent| intent.expires > now)
        .filter(|intent| intent.fire_count < intent.max_fires)
        .filter(|intent| matches!(
            &intent.trigger,
            Trigger::Time { after, keywords } if *after <= now && keywords.is_empty()
        ))
        .collect();

    if broadcast.is_empty() {
        return None;
    }

    // High priority first, then medium, then low.
    broadcast.sort_by_key(|i| match i.action.priority {
        Priority::High => 0,
        Priority::Medium => 1,
        Priority::Low => 2,
    });

    let mut s = format!("## Reminders ({})", broadcast.len());
    for intent in broadcast {
        let tag = match intent.action.priority {
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        };
        s.push_str(&format!("\n- [{tag}] {}", intent.action.message));
    }
    Some(s)
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
        assert_eq!(records[0].event, HookEvent::SessionStart { ts: 42 });
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
        assert_eq!(records[0].event, HookEvent::SessionStart { ts: 100 });
        assert_eq!(
            records[1].event,
            HookEvent::ToolUse { tool: "Read".into(), ts: 101 }
        );
        assert_eq!(
            records[2].event,
            HookEvent::ToolUse { tool: "Edit".into(), ts: 102 }
        );
        assert_eq!(records[3].event, HookEvent::SessionEnd { ts: 103 });

        // Task #6: the two tool_use events should ALSO have been sampled
        // to metacog/activity.jsonl as real-time heartbeat records. The
        // session_start/session_end events must NOT appear there.
        let activity: Vec<ToolActivitySample> =
            store.read_jsonl(METACOG_ACTIVITY_LOG).unwrap();
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

        let samples: Vec<ToolActivitySample> =
            store.read_jsonl(METACOG_ACTIVITY_LOG).unwrap();
        assert_eq!(samples.len(), 1, "tool_use must produce exactly one sample");
        assert_eq!(samples[0].tool, "Grep");
        assert_eq!(samples[0].hook_ts, 777);
        assert!(samples[0].received_at >= before && samples[0].received_at <= after,
            "received_at must be set to the daemon-side receive time");
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
            stream.write_all(b"{\"event\":\"session_start\",\"ts\":1}\n").await.unwrap();
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
        let out = build_session_start_response(&store);
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
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let out = build_session_start_response(&store);
        assert!(out.contains("# i-dream briefing"), "missing header: {out}");
        assert!(out.contains("## Reminders (1)"), "missing section: {out}");
        assert!(out.contains("[high]"), "missing priority tag: {out}");
        assert!(out.contains("Update CHANGELOG for v0.5.0"), "missing message: {out}");
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
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

        let out = build_session_start_response(&store);
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
        store.append_jsonl("intentions/registry.jsonl", &event_intention).unwrap();

        let out = build_session_start_response(&store);
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
        store.append_jsonl(
            "intentions/registry.jsonl",
            &broadcast_intention("low-1", "Low thing", Priority::Low, ago),
        ).unwrap();
        store.append_jsonl(
            "intentions/registry.jsonl",
            &broadcast_intention("high-1", "High thing", Priority::High, ago),
        ).unwrap();
        store.append_jsonl(
            "intentions/registry.jsonl",
            &broadcast_intention("med-1", "Medium thing", Priority::Medium, ago),
        ).unwrap();

        let out = build_session_start_response(&store);
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
        store.append_jsonl("intentions/registry.jsonl", &expired).unwrap();

        // Max-fired
        let mut maxed = broadcast_intention(
            "max-1",
            "Already fired out",
            Priority::High,
            chrono::Duration::hours(-1),
        );
        maxed.fire_count = 5;
        maxed.max_fires = 5;
        store.append_jsonl("intentions/registry.jsonl", &maxed).unwrap();

        let out = build_session_start_response(&store);
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
        store.write_json("introspection/patterns.json", &patterns).unwrap();

        let out = build_session_start_response(&store);
        assert!(out.contains("## Self-awareness"), "missing section: {out}");
        assert!(out.contains("methodical search"), "missing strength: {out}");
        assert!(out.contains("premature optimization"), "missing weakness: {out}");
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
        store.append_jsonl("intentions/registry.jsonl", &intention).unwrap();

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
        store.write_json("introspection/patterns.json", &patterns).unwrap();

        let out = build_session_start_response(&store);
        assert!(out.contains("## Reminders"), "missing reminders: {out}");
        assert!(out.contains("## Self-awareness"), "missing self-awareness: {out}");
        assert!(out.contains("Weekly review"));
        assert!(out.contains("incremental verification"));
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
        store.write_json("introspection/patterns.json", &patterns).unwrap();

        let out = build_session_start_response(&store);
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
}
