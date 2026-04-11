//! Daemon lifecycle — start, stop, status, idle detection, consolidation orchestration.

use crate::api::ClaudeClient;
use crate::cli::DreamPhase;
use crate::config::Config;
use crate::modules::{
    dreaming::DreamingModule, introspection::IntrospectionModule,
    metacog::MetacogModule, prospective::ProspectiveModule, Module,
};
use crate::store::Store;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::signal;
use tracing::{error, info, warn};

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
    pub async fn run_foreground(&self) -> Result<()> {
        info!("i-dream daemon running in foreground (Ctrl+C to stop)");

        let check_interval =
            Duration::from_secs(self.config.idle.check_interval_minutes * 60);

        loop {
            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!("Received shutdown signal");
                    break;
                }
                _ = tokio::time::sleep(check_interval) => {
                    self.check_and_run().await;
                }
            }
        }

        info!("Daemon stopped");
        Ok(())
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
