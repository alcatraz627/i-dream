//! The menu bar widget, `i-dream-bar.app` (tools/widget2/).
//!
//! The widget reads only `i-dream status --json` and `i-dream reader --json`;
//! these verbs build, install and control it. The v1 widget in tools/menubar/
//! is no longer built or launched by anything.
//!
//! ```text
//!   i-dream widget build      compile tools/widget2 into tools/widget2/build/
//!   i-dream widget install    copy to ~/Applications and start at login (dev.i-dream.bar)
//!   i-dream widget uninstall  stop it and remove the LaunchAgent and the app
//!   i-dream widget start      launch the installed app (or the build)
//!   i-dream widget stop       quit it cleanly, so launchd does not relaunch it
//!   i-dream widget restart    stop, then start
//!   i-dream widget status     build freshness, install, running, at-login
//!   i-dream widget logs       tail ~/Library/Logs/i-dream-bar/i-dream-bar.log
//! ```

use crate::cli::WidgetAction;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const BINARY_NAME: &str = "i-dream-bar";
const BUNDLE_ID: &str = "dev.i-dream.bar";

pub fn manage(action: WidgetAction) -> Result<()> {
    match action {
        WidgetAction::Start => start(),
        WidgetAction::Stop => stop(),
        WidgetAction::Restart => {
            stop()?;
            std::thread::sleep(std::time::Duration::from_millis(600));
            start()
        }
        WidgetAction::Build => run_script("build.sh", &[]),
        WidgetAction::Status => run_script("build.sh", &["--status"]),
        WidgetAction::Logs { lines } => logs(lines),
        WidgetAction::Install => run_script("install.sh", &[]),
        WidgetAction::Uninstall => run_script("install.sh", &["--uninstall"]),
    }
}

fn start() -> Result<()> {
    if let Some(pid) = current_pid() {
        println!("Widget is already running (PID {pid}).");
        return Ok(());
    }
    let app = app_bundle()?;
    let ok = Command::new("open")
        .arg(&app)
        .status()
        .with_context(|| format!("Failed to open {}", app.display()))?
        .success();
    if !ok {
        bail!("open {} failed", app.display());
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    match current_pid() {
        Some(pid) => println!("Widget started (PID {pid}) from {}.", app.display()),
        None => println!("Widget launched but is not in the process list; see `i-dream widget logs`."),
    }
    Ok(())
}

/// Quit through the app's own quit path. The LaunchAgent relaunches the app
/// after any exit that is not clean, so a plain kill would bring it back.
fn stop() -> Result<()> {
    if current_pid().is_none() {
        println!("Widget is not running.");
        return Ok(());
    }
    let script = format!("tell application id \"{BUNDLE_ID}\" to quit");
    Command::new("osascript")
        .args(["-e", &script])
        .status()
        .context("Failed to invoke osascript")?;
    for _ in 0..10 {
        if current_pid().is_none() {
            println!("Widget stopped.");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    bail!("the widget did not quit; it may be busy, try again or use Activity Monitor");
}

fn logs(lines: usize) -> Result<()> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let log = home.join("Library/Logs/i-dream-bar/i-dream-bar.log");
    let status = Command::new("tail")
        .args(["-n", &lines.to_string()])
        .arg(&log)
        .status()
        .context("Failed to invoke tail")?;
    if !status.success() {
        bail!("{} does not exist yet; the widget writes it once installed", log.display());
    }
    Ok(())
}

fn run_script(name: &str, args: &[&str]) -> Result<()> {
    let script = project_root()?.join("tools/widget2").join(name);
    let status = Command::new("bash")
        .arg(&script)
        .args(args)
        .status()
        .with_context(|| format!("Failed to run {}", script.display()))?;
    if !status.success() {
        bail!("{name} exited with status {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

fn current_pid() -> Option<u32> {
    let out = Command::new("pgrep").args(["-x", BINARY_NAME]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|s| s.trim().parse().ok())
}

/// The installed app in ~/Applications, else the in-tree build.
fn app_bundle() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        let installed = PathBuf::from(home).join("Applications/i-dream-bar.app");
        // The v1 widget installed under the same name; only a v2 bundle carries src-hash.
        if installed.join("Contents/Resources/src-hash").exists() {
            return Ok(installed);
        }
    }
    let built = project_root()?.join("tools/widget2/build/i-dream-bar.app");
    if built.exists() {
        return Ok(built);
    }
    bail!("no i-dream-bar.app installed or built; run `i-dream widget build`")
}

/// The checkout that holds tools/widget2: up from the executable (in-tree
/// builds), the compile-time manifest dir (`cargo install --path .`), then up
/// from the working directory.
fn project_root() -> Result<PathBuf> {
    let has = |d: &Path| d.join("tools/widget2/build.sh").exists();
    let walk = |start: Option<PathBuf>| -> Option<PathBuf> {
        let mut dir = start;
        for _ in 0..8 {
            let d = dir?;
            if has(&d) {
                return Some(d);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
        None
    };
    if let Some(d) = walk(std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf))) {
        return Ok(d);
    }
    let baked = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if has(&baked) {
        return Ok(baked);
    }
    if let Some(d) = walk(std::env::current_dir().ok()) {
        return Ok(d);
    }
    bail!(
        "could not find tools/widget2/build.sh above the executable, in {}, or above the working directory",
        env!("CARGO_MANIFEST_DIR")
    )
}
