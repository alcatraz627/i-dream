//! Daemon lifecycle — start, stop, status, idle detection, consolidation orchestration.

use crate::api::ClaudeClient;
use crate::cli::DreamPhase;
use crate::config::Config;
use crate::events::{HookEvent, HookEventRecord};
use crate::modules::{
    dreaming::DreamingModule,
    introspection::{IntrospectionModule, ReasoningPatterns},
    metacog::MetacogModule,
    prospective::{Intention, Priority, ProspectiveModule, Trigger},
    Module,
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
}
