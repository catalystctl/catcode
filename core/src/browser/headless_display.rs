//! Headless display bootstrap for the native browser runtime (Linux).
//!
//! WebKitGTK / tao need an X11 (or Wayland) display. In CI/SSH/agent hosts
//! with no `DISPLAY`, we spawn a private Xvfb and point `DISPLAY` at it so
//! create + screenshot work without the user wrapping the process in
//! `xvfb-run`.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

/// Kept alive for the process lifetime so the virtual framebuffer stays up.
static XVFB_CHILD: Mutex<Option<Child>> = Mutex::new(None);
static LAST_DISPLAY: std::sync::OnceLock<DisplayInfo> = std::sync::OnceLock::new();

/// Snapshot of the display the browser runtime is using (for capability responses).
pub fn current_display() -> Option<DisplayInfo> {
    LAST_DISPLAY.get().cloned()
}

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    /// e.g. `:99`
    pub display: String,
    /// true if we started Xvfb ourselves
    pub auto_xvfb: bool,
}

/// Ensure a usable display for the browser thread.
///
/// - If `DISPLAY` or `WAYLAND_DISPLAY` is already set → leave alone.
/// - Else if `Xvfb` is on PATH → start a private server and set `DISPLAY`.
/// - Else → structured error telling the operator to install Xvfb or set DISPLAY.
pub fn ensure_display() -> Result<DisplayInfo, String> {
    let force = std::env::var("CATALYST_CODE_BROWSER_FORCE_XVFB")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !force {
        if let Ok(d) = std::env::var("DISPLAY") {
            if !d.trim().is_empty() {
                let info = DisplayInfo {
                    display: d,
                    auto_xvfb: false,
                };
                let _ = LAST_DISPLAY.set(info.clone());
                return Ok(info);
            }
        }
        if let Ok(w) = std::env::var("WAYLAND_DISPLAY") {
            if !w.trim().is_empty() {
                let info = DisplayInfo {
                    display: format!("wayland:{w}"),
                    auto_xvfb: false,
                };
                let _ = LAST_DISPLAY.set(info.clone());
                return Ok(info);
            }
        }
    }

    let info = start_xvfb()?;
    let _ = LAST_DISPLAY.set(info.clone());
    Ok(info)
}

fn start_xvfb() -> Result<DisplayInfo, String> {
    // Reuse an already-started private Xvfb.
    if let Ok(guard) = XVFB_CHILD.lock() {
        if guard.is_some() {
            if let Ok(d) = std::env::var("DISPLAY") {
                if !d.is_empty() {
                    return Ok(DisplayInfo {
                        display: d,
                        auto_xvfb: true,
                    });
                }
            }
        }
    }

    if which("Xvfb").is_none() {
        return Err(
            "No DISPLAY/WAYLAND_DISPLAY and Xvfb not found on PATH. \
Install `xvfb` (Debian/Ubuntu: `apt install xvfb`) or run under \
`xvfb-run`, or export DISPLAY to a real X server."
                .into(),
        );
    }

    let n = find_free_display().ok_or_else(|| {
        "could not find a free X display slot for Xvfb (:90–:119 all locked)".to_string()
    })?;
    let display = format!(":{n}");

    let child = Command::new("Xvfb")
        .args([
            display.as_str(),
            "-screen",
            "0",
            "1920x1080x24",
            "-nolisten",
            "tcp",
            "-ac",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn Xvfb {display}: {e}"))?;

    // Wait until the lock file appears (server ready) or timeout.
    let lock = format!("/tmp/.X{n}-lock");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if Path::new(&lock).exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !Path::new(&lock).exists() {
        // Best-effort kill; drop child on error path.
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "Xvfb {display} started but lock {lock} never appeared"
        ));
    }

    // Process-global — intentional: GTK/WebKit read DISPLAY from the env.
    // SAFETY: called once before the browser event loop starts; no concurrent
    // env readers in unit tests of this path (browser e2e is #[ignore]/serial).
    unsafe {
        std::env::set_var("DISPLAY", &display);
    }

    if let Ok(mut guard) = XVFB_CHILD.lock() {
        *guard = Some(child);
    } else {
        // Mutex poisoned — still leave DISPLAY set; leak child by forgetting.
        std::mem::forget(child);
    }

    Ok(DisplayInfo {
        display,
        auto_xvfb: true,
    })
}

fn find_free_display() -> Option<u32> {
    // Prefer high numbers to avoid colliding with user :0 / CI :99.
    for n in (90..120).chain(200..220) {
        let lock = format!("/tmp/.X{n}-lock");
        if !Path::new(&lock).exists() {
            return Some(n);
        }
    }
    None
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let p = dir.join(bin);
            if p.is_file() {
                Some(p)
            } else {
                None
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_free_display_returns_some_slot() {
        // On a normal host at least one of :90–:119 is free.
        assert!(find_free_display().is_some());
    }

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some() || which("bash").is_some());
    }
}
