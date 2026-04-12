//! CLI integration tests — exercise the actual `i-dream` binary via
//! `assert_cmd` to lock the user-facing contract: argparse, exit codes,
//! stdout/stderr, and filesystem side-effects.
//!
//! ## Isolation strategy
//!
//! Almost every command eventually resolves paths through
//! `dirs::home_dir()`, which on Unix reads `$HOME`. So we create a
//! per-test `TempDir`, point `HOME` at it via `Command::env`, and the
//! entire `~/.claude/subconscious/` tree is redirected into the sandbox.
//! No test-mode conditionals are needed in the production code.
//!
//! ## What's NOT tested here
//!
//!   - `start` (long-running, would hang the test suite)
//!   - `dream` (requires `ANTHROPIC_API_KEY` and hits the live API)
//!   - `hooks install` / `hooks uninstall` (mutates the real Claude Code
//!     settings file — risky even in a sandbox)
//!
//! These commands are exercised via the in-source unit tests instead.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Build a fresh sandbox with `HOME` pointing into a tempdir and the
/// daemon data dir pre-created. Returns the tempdir (kept alive by the
/// caller) and the path to the data dir.
///
/// The subconscious data dir is pre-created because some commands
/// expect `~/.claude/subconscious/` to exist as an anchor, and the
/// daemon only creates it inside `Daemon::new()` — which we can't
/// always reach from the CLI surface under test.
fn sandbox() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join(".claude/subconscious");
    std::fs::create_dir_all(&data_dir).unwrap();
    (dir, data_dir)
}

/// Build a command invocation with `HOME` redirected into the given
/// tempdir. Also clears `XDG_CONFIG_HOME` and friends so `dirs::*`
/// doesn't escape the sandbox via XDG env vars.
fn cmd_in(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("i-dream").unwrap();
    cmd.env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME");
    cmd
}

// ── Smoke tests ────────────────────────────────────────────────
// Prove the binary builds, links, and at least gets to argparse.

#[test]
fn version_flag_prints_version() {
    // `--version` is the cheapest possible end-to-end check. If this
    // breaks, something catastrophic happened to the build or clap
    // configuration.
    let (dir, _) = sandbox();
    cmd_in(dir.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("i-dream"));
}

#[test]
fn help_flag_lists_all_subcommands() {
    // The `--help` output is the de-facto public contract for which
    // commands exist. If a command disappears from help unexpectedly,
    // this test fires before users notice.
    let (dir, _) = sandbox();
    cmd_in(dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("start"))
        .stdout(predicate::str::contains("stop"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("dream"))
        .stdout(predicate::str::contains("inspect"))
        .stdout(predicate::str::contains("hooks"))
        .stdout(predicate::str::contains("config"));
}

#[test]
fn unknown_subcommand_errors() {
    // clap should reject unknown subcommands with a non-zero exit and
    // a usage hint on stderr. This is the baseline argparse contract.
    let (dir, _) = sandbox();
    cmd_in(dir.path())
        .arg("nonsense")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage").or(predicate::str::contains("unrecognized")));
}

// ── `status` command ──────────────────────────────────────────
// This is the most-touched read-only command and exercises the
// PID-file + state.json reconciliation from Task #7. The three
// interesting cases — no file, live PID, stale PID — map directly to
// the three branches of `Daemon::status()`.

#[test]
fn status_without_daemon_reports_stopped() {
    let (dir, data_dir) = sandbox();

    cmd_in(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Daemon: stopped"));

    // Sanity-check that HOME was actually honored — the data dir we
    // pre-created should still be empty (status is read-only).
    assert!(data_dir.exists());
    assert!(!data_dir.join("daemon.pid").exists());
}

#[test]
fn status_with_stale_pid_file_reports_stale() {
    // Write a PID that couldn't possibly be alive — i32::MAX is well
    // above the kernel's max PID on both Linux and macOS. Status
    // should detect it via `kill(pid, 0)` and report "stale PID file"
    // instead of falsely claiming the daemon is running.
    let (dir, data_dir) = sandbox();
    let pid_path = data_dir.join("daemon.pid");
    std::fs::write(&pid_path, i32::MAX.to_string()).unwrap();

    cmd_in(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("stale PID file"))
        .stdout(predicate::str::contains(&i32::MAX.to_string()));

    // Status is read-only: it reports the stale file but does NOT
    // remove it. That's `stop`'s job.
    assert!(pid_path.exists(), "status must not delete the PID file");
}

#[test]
fn status_reports_module_dirs() {
    // `status` enumerates known module directories and labels each
    // "initialized" / "not initialized". On a fresh sandbox with
    // nothing pre-created, all five should be "not initialized".
    let (dir, _) = sandbox();

    cmd_in(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Modules:"))
        .stdout(predicate::str::contains("dreams:"))
        .stdout(predicate::str::contains("metacog:"))
        .stdout(predicate::str::contains("valence:"))
        .stdout(predicate::str::contains("introspection:"))
        .stdout(predicate::str::contains("intentions:"));
}

// ── `stop` command ────────────────────────────────────────────
// Task #7 hardened this: it must never signal a stale PID, and must
// clean stale files instead. These tests lock that contract at the
// CLI surface.

#[test]
fn stop_without_daemon_prints_friendly_message() {
    let (dir, _) = sandbox();

    cmd_in(dir.path())
        .arg("stop")
        .assert()
        .success()
        .stdout(predicate::str::contains("No daemon running"));
}

#[test]
fn stop_with_stale_pid_cleans_up_silently() {
    // Write a stale PID. `stop` should:
    //   1. Detect it's dead via kill(pid, 0)
    //   2. NOT send SIGTERM to a recycled/unrelated PID
    //   3. Remove the PID file
    //   4. Report cleanup and exit 0
    let (dir, data_dir) = sandbox();
    let pid_path = data_dir.join("daemon.pid");
    std::fs::write(&pid_path, i32::MAX.to_string()).unwrap();

    cmd_in(dir.path())
        .arg("stop")
        .assert()
        .success()
        .stdout(predicate::str::contains("Stale PID file"));

    // The file must be gone after stop completes.
    assert!(
        !pid_path.exists(),
        "stop must remove the stale PID file, but it still exists at {}",
        pid_path.display()
    );
}

#[test]
fn stop_with_corrupt_pid_file_does_not_panic() {
    // A garbled PID file should be treated the same as "no daemon" —
    // `read_pid_file` returns None, and `stop` reports "No daemon
    // running" rather than crashing on a parse error.
    let (dir, data_dir) = sandbox();
    let pid_path = data_dir.join("daemon.pid");
    std::fs::write(&pid_path, "not-a-number\n").unwrap();

    cmd_in(dir.path())
        .arg("stop")
        .assert()
        .success()
        .stdout(predicate::str::contains("No daemon running"));
}

// ── `config` command ──────────────────────────────────────────
// Prints the current config as TOML. The key contract: the output
// must itself be valid TOML (round-trippable), and if a `--config`
// path is provided, it must read from that file instead of the
// default location.

#[test]
fn config_without_file_prints_defaults() {
    // No config file at `~/.claude/subconscious/config.toml` → fall
    // back to defaults. The `config` subcommand should emit them as
    // TOML. We parse the output back to guard against any dirty
    // formatting that would break `toml::from_str`.
    let (dir, _) = sandbox();

    let output = cmd_in(dir.path())
        .arg("config")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let rendered = String::from_utf8(output).unwrap();
    let parsed: toml::Value = toml::from_str(&rendered)
        .expect("`i-dream config` output must be valid TOML");
    // Check a couple of sentinel keys from the default config.
    assert!(parsed.get("daemon").is_some(), "missing [daemon] section");
    assert!(parsed.get("budget").is_some(), "missing [budget] section");
    assert!(parsed.get("modules").is_some(), "missing [modules] section");
}

#[test]
fn config_respects_explicit_config_arg() {
    // Write a minimal but fully-populated config to a tempfile and
    // pass `--config <path>`. The output should reflect OUR file's
    // values, not the defaults.
    //
    // The config has to be a full Config struct (all required fields)
    // because Config::load parses strictly. We tweak one sentinel
    // value (log_level = "debug") so we can verify the file was
    // actually read.
    let (dir, _) = sandbox();
    let config_path = dir.path().join("custom.toml");

    let toml_body = r#"
[daemon]
socket_path = "/tmp/custom.sock"
log_level = "debug"
max_concurrent_modules = 4

[idle]
threshold_hours = 8
check_interval_minutes = 30
activity_signal = "/tmp/activity"

[budget]
max_tokens_per_cycle = 99999
max_runtime_minutes = 5
model = "claude-sonnet-4-6"
model_heavy = "claude-opus-4-6"

[modules.dreaming]
enabled = true
sws_enabled = true
rem_enabled = false
wake_enabled = false
min_sessions_since_last = 1
journal_max_entries = 100

[modules.metacog]
enabled = false
sample_rate = 0.1
triggered_sample_rate = 0.5
trigger_on_correction = true
trigger_on_multi_failure = true
max_samples_per_session = 10

[modules.intuition]
enabled = false
min_occurrences = 2
decay_halflife_days = 14.0
priming_decay_hours = 2.0
max_valence_entries = 500

[modules.introspection]
enabled = false
sample_rate = 0.1
report_interval_days = 14
min_chains_for_report = 5

[modules.prospective]
enabled = false
max_active_intentions = 10
default_expiry_days = 7
match_threshold = 0.8

[hooks]
session_start = false
post_tool_use = false
stop = false
pre_compact = false
"#;
    std::fs::write(&config_path, toml_body).unwrap();

    cmd_in(dir.path())
        .arg("--config")
        .arg(&config_path)
        .arg("config")
        .assert()
        .success()
        .stdout(predicate::str::contains("log_level = \"debug\""))
        .stdout(predicate::str::contains("max_tokens_per_cycle = 99999"));
}

// ── `inspect` command ─────────────────────────────────────────
// `inspect <module>` prints a module's current state. The module
// registry is small and closed, so we test the happy path for a
// known module and the rejection path for an unknown name.

#[test]
fn inspect_unknown_module_errors() {
    let (dir, _) = sandbox();
    cmd_in(dir.path())
        .arg("inspect")
        .arg("totallybogusmodule")
        .assert()
        .failure();
}

#[test]
fn inspect_known_module_succeeds_on_empty_store() {
    // With a fresh sandbox there's no data yet, but `inspect dreaming`
    // should still return a report (likely empty / "no data") rather
    // than error out. This guards against the regression where the
    // inspect command assumes data exists.
    let (dir, _) = sandbox();
    cmd_in(dir.path())
        .arg("inspect")
        .arg("dreaming")
        .assert()
        .success();
}
