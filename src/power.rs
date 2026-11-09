//! Whether this laptop will stay awake long enough for a slow background job.
//!
//! A timer that comes due while the Mac sleeps fires during a dark wake: a
//! couple of seconds on battery with the lid shut. A model call started then is
//! frozen by the next sleep and fails on a later wake without reaching the API.

use std::process::Command;

/// Why slow work should wait, or `None` when it can go ahead. A machine whose
/// state cannot be read counts as awake, so a broken check never stops the work.
pub fn sleep_bound_reason() -> Option<String> {
    let state = pmset(&["-g", "systemstate"])?;
    let source = pmset(&["-g", "ps"])?;
    is_sleep_bound(&state, &source).then(|| "the Mac is in a dark wake on battery".to_string())
}

fn pmset(args: &[&str]) -> Option<String> {
    let out = Command::new("/usr/bin/pmset").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A dark wake is a wake without the display. On mains power macOS holds a dark
/// wake open for maintenance work, so only battery counts as sleep-bound.
fn is_sleep_bound(system_state: &str, power_source: &str) -> bool {
    system_state.contains("Capabilities")
        && !system_state.contains("Graphics")
        && power_source.contains("Battery Power")
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWAKE: &str = "Current System Capabilities are: CPU Graphics Audio Network\nCurrent Power State: 4\n";
    const DARK: &str = "Current System Capabilities are: CPU Network\nCurrent Power State: 4\n";
    const BATTERY: &str = "Now drawing from 'Battery Power'\n";
    const MAINS: &str = "Now drawing from 'AC Power'\n";

    #[test]
    fn only_a_dark_wake_on_battery_is_sleep_bound() {
        assert!(is_sleep_bound(DARK, BATTERY));
        assert!(!is_sleep_bound(DARK, MAINS));
        assert!(!is_sleep_bound(AWAKE, BATTERY));
        assert!(!is_sleep_bound(AWAKE, MAINS));
    }

    #[test]
    fn unreadable_power_output_counts_as_awake() {
        assert!(!is_sleep_bound("", BATTERY));
        assert!(!is_sleep_bound("something else entirely", BATTERY));
    }
}
