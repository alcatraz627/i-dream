//! Daemon lifecycle — start, stop, status, idle detection, consolidation orchestration.

use crate::api::ClaudeClient;
use crate::cli::DreamPhase;
use crate::config::Config;
use crate::events::{HookEvent, HookEventRecord};
use crate::modules::{
    dreaming::DreamingModule, introspection::IntrospectionModule,
    metacog::MetacogModule, prospective::ProspectiveModule, Module,
};
use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal;
use tracing::{debug, error, info, warn};

/// Relative path (under the data dir) of the hook-event log.
const EVENTS_LOG: &str = "logs/events.jsonl";

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
    state: DaemonState,
    client: Option<ClaudeClient>,
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

        // API client is optional — some commands don't need it
        let client = ClaudeClient::new().ok();

        Ok(Self {
            config,
            store,
            state,
            client,
        })
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

        let result: Result<()> = loop {
            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!("Received shutdown signal");
                    break Ok(());
                }
                _ = tokio::time::sleep(check_interval) => {
                    self.check_and_run().await;
                }
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _addr)) => {
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

        info!("Daemon stopped");
        result
    }

    /// Daemonize (fork to background).
    pub async fn daemonize(&self) -> Result<()> {
        // Write PID file
        let pid = std::process::id();
        let pid_path = self.config.data_dir().join("daemon.pid");
        std::fs::write(&pid_path, pid.to_string())?;
        info!("Daemon started with PID {pid}");

        self.run_foreground().await
    }

    /// Check idle state and run consolidation if appropriate.
    async fn check_and_run(&self) {
        match self.should_consolidate() {
            Ok(true) => {
                info!("Idle threshold reached, starting consolidation cycle");
                if let Err(e) = self.run_consolidation().await {
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
            .context("ANTHROPIC_API_KEY not set — cannot run analysis")?;

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

        info!(
            "Consolidation cycle complete (tokens remaining: {budget}/{})",
            self.config.budget.max_tokens_per_cycle
        );

        Ok(())
    }

    /// Manually trigger a dream cycle.
    pub async fn run_dream(&self, phase: DreamPhase) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .context("ANTHROPIC_API_KEY not set")?;

        let module = DreamingModule::new(&self.config, &self.store);
        let budget = self.config.budget.max_tokens_per_cycle;

        match phase {
            DreamPhase::All => {
                module.run(client, budget).await?;
            }
            DreamPhase::Sws => {
                module.run_sws(client, budget).await?;
            }
            DreamPhase::Rem => {
                module.run_rem(client, budget).await?;
            }
            DreamPhase::Wake => {
                module.run_wake(client, budget).await?;
            }
        }

        Ok(())
    }

    /// Stop a running daemon by PID.
    pub async fn stop() -> Result<()> {
        let pid_path = pid_file_path();
        if !pid_path.exists() {
            println!("No daemon running (no PID file found)");
            return Ok(());
        }

        let pid_str = std::fs::read_to_string(&pid_path)?;
        let pid: i32 = pid_str.trim().parse()?;

        // Send SIGTERM
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }

        std::fs::remove_file(&pid_path)?;
        println!("Stopped daemon (PID {pid})");
        Ok(())
    }

    /// Get daemon status.
    pub async fn status() -> Result<String> {
        let pid_path = pid_file_path();
        let data_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".claude/subconscious");

        let mut out = String::new();

        // Daemon status
        if pid_path.exists() {
            let pid = std::fs::read_to_string(&pid_path)?.trim().to_string();
            out.push_str(&format!("Daemon: running (PID {pid})\n"));
        } else {
            out.push_str("Daemon: stopped\n");
        }

        // State
        let state_path = data_dir.join("state.json");
        if state_path.exists() {
            let content = std::fs::read_to_string(&state_path)?;
            let state: DaemonState = serde_json::from_str(&content)?;
            if let Some(last) = state.last_consolidation {
                out.push_str(&format!("Last consolidation: {last}\n"));
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

    // Response is empty for now. Task #4 will populate it for SessionStart
    // with surfaced intuitions + matched intentions, which the hook script
    // then echoes into Claude's context.
    let response = match &event {
        HookEvent::SessionStart { .. } => String::new(),
        _ => String::new(),
    };
    writer.write_all(response.as_bytes()).await?;
    writer.shutdown().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
