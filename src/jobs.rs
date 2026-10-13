//! Running i-dream's scheduled jobs early or late.
//!
//! The jobs themselves live in the gcc scheduler. "Run now" runs one through
//! `gcc-schedule run`, so the run is recorded like a scheduled fire. "Later"
//! puts a hold on the job, which makes its regular fire stand down, and adds a
//! one-shot gcc job that runs it at the chosen time. Holds live in
//! `~/.claude/i-dream/holds.json`.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The jobs a hold can delay: their regular command checks it.
pub const DELAYABLE: [&str; 2] = ["i-dream-reader-daily", "i-dream-reader-weekly"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hold {
    pub until: DateTime<Utc>,
    /// The one-shot gcc job that runs the delayed job at `until`.
    pub oneshot: String,
}

fn holds_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    Ok(home.join(".claude/i-dream/holds.json"))
}

pub fn load_holds() -> BTreeMap<String, Hold> {
    holds_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_holds(h: &BTreeMap<String, Hold>) -> Result<()> {
    let path = holds_path()?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(h)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// The time a job is held until, if the hold is still in the future.
pub fn held_until(name: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    load_holds().get(name).map(|h| h.until).filter(|u| *u > now)
}

/// The next time a gcc schedule fires after `now`, for "daily@HH:MM",
/// "weekly@<dow>@HH:MM" and "monthly@day<N>@HH:MM". Anything else is None.
pub fn next_fire(fire_at: &str, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let p: Vec<&str> = fire_at.split('@').collect();
    let at = |d: chrono::NaiveDate, t: NaiveTime| Local.from_local_datetime(&d.and_time(t)).earliest();
    let time = |s: &str| NaiveTime::parse_from_str(s, "%H:%M").ok();
    let today = now.date_naive();
    match p.as_slice() {
        ["daily", hm] => {
            let t = time(hm)?;
            (0..2).filter_map(|i| at(today + Duration::days(i), t)).find(|x| *x > now)
        }
        ["weekly", dow, hm] => {
            let t = time(hm)?;
            let want: Weekday = dow.parse().ok()?;
            (0..8)
                .map(|i| today + Duration::days(i))
                .filter(|d| d.weekday() == want)
                .filter_map(|d| at(d, t))
                .find(|x| *x > now)
        }
        ["monthly", day, hm] => {
            let t = time(hm)?;
            let n: u32 = day.trim_start_matches("day").parse().ok()?;
            (0..62)
                .map(|i| today + Duration::days(i))
                .filter(|d| d.day() == n)
                .filter_map(|d| at(d, t))
                .find(|x| *x > now)
        }
        _ => None,
    }
}

fn schedule_tool() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot resolve home dir")?;
    let p = home.join(".claude/scripts/schedule/schedule.sh");
    if !p.exists() {
        bail!("gcc-schedule not found at {}", p.display());
    }
    Ok(p)
}

fn schedule(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("bash")
        .arg(schedule_tool()?)
        .args(args)
        .output()
        .context("run gcc-schedule")?;
    if !out.status.success() {
        bail!(
            "gcc-schedule {} failed: {}{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn known(name: &str) -> Result<()> {
    if crate::reader::scheduled_jobs().iter().any(|j| j.name == name) {
        Ok(())
    } else {
        bail!("{name} is not an i-dream job in the gcc schedule registry")
    }
}

/// Drop a job's hold and retire its one-shot, if any.
pub fn cancel(name: &str) -> Result<Option<Hold>> {
    let mut holds = load_holds();
    let Some(h) = holds.remove(name) else { return Ok(None) };
    save_holds(&holds)?;
    // A one-shot that already fired has retired itself; that is not an error.
    let _ = schedule(&["rm", &h.oneshot]);
    Ok(Some(h))
}

/// Start a job now, ahead of its schedule, clearing any hold first. The run
/// is detached so it outlives the caller; its output goes to the job's usual
/// launchd logs.
pub fn run_now(name: &str) -> Result<u32> {
    known(name)?;
    cancel(name)?;
    let logs = dirs::home_dir().context("home")?.join(".claude/logs/launchd");
    std::fs::create_dir_all(&logs)?;
    let open = |ext: &str| {
        std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("{name}.{ext}.log")))
    };
    let child = std::process::Command::new("bash")
        .arg(job_script(name)?)
        .stdin(std::process::Stdio::null())
        .stdout(open("out")?)
        .stderr(open("err")?)
        .spawn()
        .context("start gcc-schedule run")?;
    Ok(child.id())
}

/// Hold a job until `until` and arrange for it to run then.
pub fn delay(name: &str, until: DateTime<Local>, now: DateTime<Local>) -> Result<Hold> {
    known(name)?;
    if !DELAYABLE.contains(&name) {
        bail!("{name} cannot be delayed; only {} can", DELAYABLE.join(" and "));
    }
    if until <= now + Duration::minutes(1) {
        bail!("pick a time at least a minute from now");
    }
    cancel(name)?;
    let oneshot = format!("{name}-later-{}", until.format("%m%d-%H%M"));
    let bin = dirs::home_dir().context("home")?.join(".local/bin/i-dream");
    let command = format!("{} jobs fire {name}", bin.display());
    schedule(&[
        "add",
        "--name",
        &oneshot,
        "--at",
        &until.format("%Y-%m-%dT%H:%M").to_string(),
        "--command",
        &command,
        "--alert",
        "0",
        "--description",
        &format!("i-dream: {name}, delayed from the widget"),
    ])?;
    let hold = Hold { until: until.with_timezone(&Utc), oneshot };
    let mut holds = load_holds();
    holds.insert(name.to_string(), hold.clone());
    save_holds(&holds)?;
    Ok(hold)
}

/// What the one-shot runs at the delayed time: clear the hold, then run the
/// job's own script so its history shows the fire.
pub fn fire(name: &str) -> Result<i32> {
    let mut holds = load_holds();
    holds.remove(name);
    save_holds(&holds)?;
    let status = std::process::Command::new("bash").arg(job_script(name)?).status().context("run the job script")?;
    Ok(status.code().unwrap_or(1))
}

/// The wrapper gcc-schedule generated for a job. Running it is what a
/// scheduled fire does, including the run record in the scheduler's history;
/// `gcc-schedule run` skips that record.
fn job_script(name: &str) -> Result<PathBuf> {
    let p = dirs::home_dir().context("home")?.join(".claude/scheduled").join(name).join("script.sh");
    if !p.exists() {
        bail!("no script for {name} at {}", p.display());
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(s: &str) -> DateTime<Local> {
        Local.from_local_datetime(&chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()).unwrap()
    }

    #[test]
    fn next_fire_covers_each_schedule_shape() {
        // 2026-10-13 is a Tuesday.
        let now = local("2026-10-13 16:00");
        assert_eq!(next_fire("daily@04:30", now), Some(local("2026-10-14 04:30")));
        assert_eq!(next_fire("daily@17:00", now), Some(local("2026-10-13 17:00")));
        assert_eq!(next_fire("weekly@wed@11:00", now), Some(local("2026-10-14 11:00")));
        assert_eq!(next_fire("weekly@tue@13:00", now), Some(local("2026-10-20 13:00")), "earlier today has passed");
        assert_eq!(next_fire("weekly@tue@17:00", now), Some(local("2026-10-13 17:00")));
        assert_eq!(next_fire("monthly@day1@04:10", now), Some(local("2026-11-01 04:10")));
        assert_eq!(next_fire("custom-schedule", now), None);
    }
}
