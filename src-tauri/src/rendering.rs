//! WebKitGTK's GPU renderer is far smoother but kills the process on some Wayland setups
//! (Error 71), so a start that dies before the page draws leaves the next one in software.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::CommandResult;

/// WebKitGTK's own switch: any value but "0" turns the GPU renderer off.
const ENV: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
const PREFS_FILE: &str = "rendering.json";
/// Written before the window draws and removed once it has: still there at the next start
/// means the process died first.
const PENDING_FILE: &str = "rendering-pending";
/// NVIDIA's explicit sync makes KWin drop the GPU-drawn window with a protocol error (Error 71).
const NV_SYNC: &str = "__NV_DISABLE_EXPLICIT_SYNC";

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Prefs {
    software: bool,
    /// Set by a start that died before drawing, rather than chosen.
    fell_back: bool,
    /// A fallback from before the NVIDIA fix got its one more try with it.
    nvidia_retried: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenderingStatus {
    /// Drawing with the GPU in this run.
    pub gpu: bool,
    /// What the next start uses.
    pub prefer_gpu: bool,
    /// Switched to software after a start died before its window drew.
    pub fell_back: bool,
    /// The variable was set outside the app, which always wins.
    pub from_environment: bool,
}

/// This run's mode and where its files live, fixed before the webview starts.
struct Startup {
    dir: PathBuf,
    gpu: bool,
    from_environment: bool,
    /// What to keep for the next start: a fallback after a crash, or a retry on NVIDIA.
    persist: Option<Prefs>,
    boot: Option<String>,
}

static STARTUP: OnceLock<Startup> = OnceLock::new();

/// What a start does, from what the last one left behind.
#[derive(Debug, PartialEq)]
struct Decision {
    prefs: Prefs,
    /// The last start died before drawing, on this boot.
    crashed: bool,
    /// An NVIDIA machine that fell back without the fix tries the GPU once more.
    retry: bool,
}

/// A marker from an earlier boot only means the machine went down with a hidden window.
fn decide(mut prefs: Prefs, pending: Option<&str>, boot: Option<&str>, nvidia: bool) -> Decision {
    let crashed = !prefs.software && pending.is_some_and(|p| p.trim() == boot.unwrap_or(""));
    let retry = !crashed && nvidia && prefs.software && prefs.fell_back && !prefs.nvidia_retried;
    if crashed {
        prefs.software = true;
        prefs.fell_back = true;
    } else if retry {
        prefs = Prefs {
            software: false,
            fell_back: false,
            nvidia_retried: true,
        };
    }
    Decision {
        prefs,
        crashed,
        retry,
    }
}

/// The proprietary driver, whose module the sandbox can see too.
fn nvidia_loaded() -> bool {
    Path::new("/sys/module/nvidia_drm").exists() || Path::new("/proc/driver/nvidia").exists()
}

fn boot_id() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .ok()
        .map(|id| id.trim().to_string())
}

fn read_prefs(dir: &Path) -> Prefs {
    std::fs::read(dir.join(PREFS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_prefs(dir: &Path, prefs: Prefs) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(&prefs).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(PREFS_FILE), json)
}

/// Picks this run's renderer. Before Tauri starts, since WebKit reads the variable once. Writes
/// nothing: a second copy runs this too, then hands over and exits without its exit handlers.
pub fn prepare(dir: PathBuf) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let nvidia = nvidia_loaded();
    let startup = start(dir, std::env::var_os(ENV), boot_id(), nvidia);
    if !startup.gpu && !startup.from_environment {
        std::env::set_var(ENV, "1");
    }
    if startup.gpu && nvidia && std::env::var_os(NV_SYNC).is_none() {
        tracing::info!("NVIDIA driver: drawing without its explicit sync, which KWin rejects");
        std::env::set_var(NV_SYNC, "1");
        // Only for this window: games keep the driver's default.
        gameyfin_core::process::keep_from_children(NV_SYNC);
    }
    let _ = STARTUP.set(startup);
}

/// [`prepare`] without the process-wide parts, so a crashed start can be replayed in a test.
fn start(
    dir: PathBuf,
    env: Option<std::ffi::OsString>,
    boot: Option<String>,
    nvidia: bool,
) -> Startup {
    if let Some(value) = env {
        let gpu = value == "0";
        tracing::info!(gpu, "{ENV} is set outside the app");
        return Startup {
            dir,
            gpu,
            from_environment: true,
            persist: None,
            boot,
        };
    }

    let pending = std::fs::read_to_string(dir.join(PENDING_FILE)).ok();
    let decision = decide(
        read_prefs(&dir),
        pending.as_deref(),
        boot.as_deref(),
        nvidia,
    );
    let gpu = !decision.prefs.software;
    if decision.retry {
        tracing::info!("trying the GPU again on NVIDIA, now with explicit sync off");
    }
    tracing::info!(gpu, "renderer chosen");
    Startup {
        dir,
        gpu,
        from_environment: false,
        persist: (decision.crashed || decision.retry).then_some(decision.prefs),
        boot,
    }
}

/// Registered right after the single-instance plugin, so its setup runs only in the copy that
/// keeps going, and before any window exists.
pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("rendering")
        .setup(|_, _| {
            if let Some(startup) = STARTUP.get() {
                arm(startup);
            }
            Ok(())
        })
        .build()
}

fn arm(startup: &Startup) {
    if startup.from_environment {
        return;
    }
    let dir = &startup.dir;
    if let Some(prefs) = startup.persist {
        if prefs.software {
            tracing::warn!("the last start closed before its window drew; drawing without the GPU");
        }
        if let Err(e) = write_prefs(dir, prefs) {
            tracing::warn!("could not keep the renderer choice: {e}");
        }
    }
    if startup.gpu {
        // A first start has no config folder yet, and is the one most likely to crash.
        let marked = std::fs::create_dir_all(dir).and_then(|()| {
            std::fs::write(
                dir.join(PENDING_FILE),
                startup.boot.as_deref().unwrap_or(""),
            )
        });
        if let Err(e) = marked {
            tracing::warn!("could not mark the start: {e}");
        }
    } else {
        let _ = std::fs::remove_file(dir.join(PENDING_FILE));
    }
}

fn settle(startup: &Startup) {
    let _ = std::fs::remove_file(startup.dir.join(PENDING_FILE));
}

/// The window drew, or the app is closing normally: either way the GPU did not kill it.
pub fn settled() {
    if let Some(startup) = STARTUP.get() {
        settle(startup);
    }
}

fn status(startup: &Startup) -> RenderingStatus {
    let prefs = read_prefs(&startup.dir);
    RenderingStatus {
        gpu: startup.gpu,
        prefer_gpu: if startup.from_environment {
            startup.gpu
        } else {
            !prefs.software
        },
        fell_back: prefs.fell_back,
        from_environment: startup.from_environment,
    }
}

/// `None` where there is no choice to make, which is anywhere but Linux.
#[tauri::command]
pub async fn rendering_status() -> CommandResult<Option<RenderingStatus>> {
    Ok(STARTUP.get().map(status))
}

#[tauri::command]
pub async fn set_gpu_rendering(on: bool) -> CommandResult<Option<RenderingStatus>> {
    let Some(startup) = STARTUP.get() else {
        return Ok(None);
    };
    write_prefs(
        &startup.dir,
        Prefs {
            software: !on,
            fell_back: false,
            ..read_prefs(&startup.dir)
        },
    )
    .map_err(|e| crate::error::CommandError::Message(format!("could not save: {e}")))?;
    Ok(Some(status(startup)))
}

#[tauri::command]
pub async fn confirm_rendering() -> CommandResult<()> {
    settled();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOT: Option<&str> = Some("boot-a");

    #[test]
    fn a_clean_last_start_keeps_the_gpu() {
        let decision = decide(Prefs::default(), None, BOOT, false);
        assert!(!decision.crashed);
        assert!(!decision.prefs.software);
    }

    #[test]
    fn a_start_that_died_on_this_boot_falls_back() {
        let decision = decide(Prefs::default(), Some("boot-a\n"), BOOT, false);
        assert!(decision.crashed);
        assert_eq!(
            decision.prefs,
            Prefs {
                software: true,
                fell_back: true,
                nvidia_retried: false
            }
        );
    }

    #[test]
    fn a_marker_from_an_earlier_boot_is_a_shutdown_not_a_crash() {
        // Started hidden in the tray, never drawn, then the machine was turned off.
        let decision = decide(Prefs::default(), Some("boot-old"), BOOT, false);
        assert!(!decision.crashed);
        assert!(!decision.prefs.software);
    }

    #[test]
    fn software_by_choice_is_not_reported_as_a_fallback() {
        let chosen = Prefs {
            software: true,
            ..Prefs::default()
        };
        let decision = decide(chosen, Some("boot-a"), BOOT, false);
        assert!(!decision.crashed);
        assert_eq!(decision.prefs, chosen);
    }

    #[test]
    fn without_a_boot_id_any_leftover_marker_counts() {
        assert!(decide(Prefs::default(), Some(""), None, false).crashed);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-rendering-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn boot() -> Option<String> {
        BOOT.map(str::to_string)
    }

    /// A start of the copy that keeps running, which reaches Tauri's setup.
    fn run(dir: &Path) -> Startup {
        let startup = start(dir.to_path_buf(), None, boot(), false);
        arm(&startup);
        startup
    }

    #[test]
    fn a_second_copy_that_hands_over_leaves_nothing_behind() {
        let dir = scratch("duplicate");
        settle(&run(&dir));
        // Launched again while running: it exits from the single-instance check, skipping exit handlers.
        start(dir.clone(), None, boot(), false);
        assert!(run(&dir).gpu);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_copy_during_the_first_ones_start_keeps_nothing() {
        let dir = scratch("double-click");
        let first = run(&dir);
        // Sees the first copy's mark before it drew, and would read it as a crash.
        assert!(!start(dir.clone(), None, boot(), false).gpu);
        settle(&first);
        assert!(run(&dir).gpu);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_start_killed_before_drawing_leaves_the_next_in_software() {
        // No folder yet, as on a first start.
        let dir = scratch("crash");
        assert!(run(&dir).gpu);
        // The process dies here, so nothing settles it.
        let next = run(&dir);
        assert!(!next.gpu);
        assert_eq!(
            status(&next),
            RenderingStatus {
                gpu: false,
                prefer_gpu: false,
                fell_back: true,
                from_environment: false
            }
        );
        // Software stays, and is not reported as a fresh crash.
        assert!(!run(&dir).gpu);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_start_that_drew_keeps_the_gpu_next_time() {
        let dir = scratch("drew");
        settle(&run(&dir));
        assert!(run(&dir).gpu);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_environment_wins_and_touches_nothing() {
        let dir = scratch("env");
        let startup = start(dir.clone(), Some("1".into()), boot(), false);
        arm(&startup);
        assert!(!startup.gpu && startup.from_environment);
        assert!(start(dir.clone(), Some("0".into()), boot(), false).gpu);
        assert!(!dir.exists());
    }

    #[test]
    fn a_fallback_from_before_the_nvidia_fix_gets_one_more_try() {
        let dir = scratch("nvidia");
        let start_on = |nvidia| {
            let startup = start(dir.clone(), None, boot(), nvidia);
            arm(&startup);
            startup
        };
        write_prefs(
            &dir,
            Prefs {
                software: true,
                fell_back: true,
                ..Prefs::default()
            },
        )
        .unwrap();
        // Not on another GPU: what fell back stays in software there.
        assert!(!start_on(false).gpu);
        assert!(start_on(true).gpu);
        // That try died too, so software stays for good.
        assert!(!start_on(true).gpu);
        assert!(!start_on(true).gpu);
        assert!(read_prefs(&dir).nvidia_retried);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_retry_that_draws_keeps_the_gpu() {
        let dir = scratch("nvidia-drew");
        write_prefs(
            &dir,
            Prefs {
                software: true,
                fell_back: true,
                ..Prefs::default()
            },
        )
        .unwrap();
        let retry = start(dir.clone(), None, boot(), true);
        arm(&retry);
        settle(&retry);
        let next = start(dir.clone(), None, boot(), true);
        assert!(next.gpu && next.persist.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preferences_survive_a_round_trip_and_a_missing_file() {
        let dir = scratch("prefs");
        assert_eq!(read_prefs(&dir), Prefs::default());
        let prefs = Prefs {
            software: true,
            fell_back: true,
            nvidia_retried: true,
        };
        write_prefs(&dir, prefs).unwrap();
        assert_eq!(read_prefs(&dir), prefs);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
