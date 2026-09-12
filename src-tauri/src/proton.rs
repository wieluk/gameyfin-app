//! Running Windows games through umu and Proton: the launcher, managed builds, and which
//! runtime a game gets.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use gameyfin_core::proton::{self, InstalledProton, ProtonFamily, ProtonRelease};
use gameyfin_core::{RuntimeContext, WindowsRuntime};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{blocking, CommandError, CommandResult, Context};
use crate::state::AppState;
use crate::wine::RELEASE_CHOICES;

/// Tells the Steam Runtime container which extra host folders to share.
pub const CONTAINER_MOUNTS_VAR: &str = "PRESSURE_VESSEL_FILESYSTEMS_RW";

static LAUNCHER: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Cached, since a missing python3 does not turn up while the app runs.
static PREFLIGHT: tokio::sync::OnceCell<Result<Launcher, String>> =
    tokio::sync::OnceCell::const_new();

#[derive(Debug, Clone)]
pub struct Launcher {
    pub path: PathBuf,
    pub version: Option<String>,
}

/// Needs the app handle for the resource directory, so it runs at startup.
pub fn remember_launcher(app: &AppHandle) {
    let found = crate::sidecar::bundled(app, "umu-run")
        .or_else(|| gameyfin_core::runtime::find_program("umu-run"));
    let _ = LAUNCHER.set(found);
}

/// The launcher, once it has run. The zipapp needs python3 3.10+, which a package cannot guarantee.
pub async fn umu_launcher() -> Result<Launcher, String> {
    PREFLIGHT
        .get_or_init(|| async {
            let Some(path) = LAUNCHER.get().cloned().flatten() else {
                return Err("umu-run is not part of this build.".to_string());
            };
            let mut command = tokio::process::Command::new(&path);
            command.arg("-v").stdin(std::process::Stdio::null());
            gameyfin_core::process::clean_for_host(&mut command);
            let out = match tokio::time::timeout(Duration::from_secs(20), command.output()).await {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => {
                    return Err(format!(
                        "umu-run could not start: {e}. It needs python3 3.10 or newer."
                    ))
                }
                Err(_) => return Err("umu-run did not respond.".to_string()),
            };
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            if !out.status.success() {
                let detail = text.trim().lines().last().unwrap_or_default().to_string();
                tracing::warn!(?path, %detail, "umu-run could not start");
                return Err(format!(
                    "umu-run could not start ({detail}). It needs python3 3.10 or newer."
                ));
            }
            let version = text
                .split_whitespace()
                .skip_while(|word| *word != "version")
                .nth(1)
                .map(str::to_string);
            tracing::info!(?path, ?version, "umu-run is ready");
            Ok(Launcher { path, version })
        })
        .await
        .clone()
}

/// Library folders outside home, /media, /mnt and /run/media are invisible in the container,
/// so an installer writing there would land in a copy that vanishes.
pub fn container_mounts(state: &AppState, runtime: &WindowsRuntime) -> Option<(String, String)> {
    if runtime.is_wine_family() {
        return None;
    }
    let roots = state.settings().library_roots();
    (!roots.is_empty()).then(|| (CONTAINER_MOUNTS_VAR.to_string(), roots.join(":")))
}

pub async fn runtime_context(
    state: &AppState,
    game_id: Option<i64>,
    program: Option<&Path>,
) -> RuntimeContext {
    let record = game_id.map(|id| state.library().record(id));
    // The game's own executable, so a prefix tool gets the runtime the game will use.
    let program = program.map(Path::to_path_buf).or_else(|| {
        let record = record.as_ref()?;
        Some(record.install_dir.clone()?.join(record.executable.clone()?))
    });
    RuntimeContext {
        config_dir: Some(state.config_dir()),
        umu_launcher: umu_launcher().await.ok().map(|launcher| launcher.path),
        proton_build: record
            .and_then(|record| record.proton_build)
            .or(state.settings().default_proton),
        program,
        supports_32bit: gameyfin_core::runtime::has_32bit_support(),
    }
}

/// The runtime a game runs with, honouring its override. A missing override is an error,
/// since silently using another runtime looks like the setting did nothing.
pub async fn runtime_for_game(
    state: &AppState,
    game_id: i64,
    program: Option<&Path>,
) -> CommandResult<WindowsRuntime> {
    let override_kind = state.library().record(game_id).runtime_override;
    let ctx = runtime_context(state, Some(game_id), program).await;
    let kind = override_kind.clone();
    let found = tokio::task::spawn_blocking(move || match kind.as_deref() {
        Some(kind) => gameyfin_core::find_windows_runtime(&ctx, kind),
        None => gameyfin_core::detect_windows_runtime(&ctx),
    })
    .await
    .context("could not look for a runtime")?;
    if let Some(runtime) = found {
        return Ok(runtime);
    }
    Err(CommandError::msg(
        match (override_kind, umu_launcher().await.err()) {
            (Some(kind), _) => format!(
                "This game is set to run with {kind}, which is not available on this system."
            ),
            (None, Some(problem)) => format!("{problem} {}", gameyfin_core::windows_runtime_hint()),
            (None, None) => gameyfin_core::windows_runtime_hint(),
        },
    ))
}

/// Downloads UMU-Proton when there is no build yet. Not fatal: Steam's builds or Wine remain.
/// The caller holds the runtime lock.
pub async fn ensure_default(app: &AppHandle, state: &AppState) {
    if umu_launcher().await.is_err() || !proton::installed(&state.config_dir()).is_empty() {
        return;
    }
    let release = match proton::latest_release(&state.http(), ProtonFamily::UmuProton).await {
        Ok(release) => release,
        Err(e) => return tracing::warn!(error = %e, "could not look up UMU-Proton"),
    };
    if let Err(e) = download(app, state, &release).await {
        tracing::warn!("{e}");
    }
}

async fn download(
    app: &AppHandle,
    state: &AppState,
    release: &ProtonRelease,
) -> CommandResult<InstalledProton> {
    tracing::info!(tag = %release.tag, "downloading Proton");
    let downloader = gameyfin_core::Downloader::new(state.transfer_http());
    let installed = proton::install(
        &state.config_dir(),
        &state.http(),
        release,
        &downloader,
        crate::progress::emitter(app, "proton-progress"),
    )
    .await
    .context(format!("Could not install {}", release.tag))?;
    let _ = app.emit("proton-changed", ());
    Ok(installed)
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProtonStatus {
    /// Gameyfin's builds, then Steam's.
    pub installed: Vec<InstalledProton>,
    pub default_build: Option<String>,
    /// What a game without its own pin runs with.
    pub in_use: Option<String>,
    pub latest_umu: Option<ProtonRelease>,
    pub latest_ge: Option<ProtonRelease>,
    pub umu_tags: Vec<String>,
    pub ge_tags: Vec<String>,
    pub launcher_version: Option<String>,
    pub launcher_problem: Option<String>,
    /// Whether 32-bit programs can run in the container at all.
    pub supports_32bit: bool,
}

#[tauri::command]
pub async fn proton_status(state: State<'_, AppState>) -> CommandResult<ProtonStatus> {
    let config_dir = state.config_dir();
    let default_build = state.settings().default_proton;
    let http = state.http();
    // An unreachable feed is empty rather than an error, so what is installed still shows.
    let listed = |family: ProtonFamily| {
        let http = http.clone();
        async move {
            proton::releases(&http, family).await.unwrap_or_else(|e| {
                tracing::warn!(family = family.as_str(), error = %e, "could not check for Proton");
                Vec::new()
            })
        }
    };
    let (umu, ge) = tokio::join!(
        listed(ProtonFamily::UmuProton),
        listed(ProtonFamily::GeProton)
    );
    let latest = |releases: &[ProtonRelease]| {
        releases
            .iter()
            .find(|r| !proton::is_prerelease_tag(&r.tag))
            .cloned()
    };
    let tags = |releases: &[ProtonRelease]| {
        releases
            .iter()
            .take(RELEASE_CHOICES)
            .map(|r| r.tag.clone())
            .collect()
    };

    let installed = blocking("could not list Proton builds", move || {
        Ok::<_, std::convert::Infallible>(proton::available(&config_dir))
    })
    .await?;
    let launcher = umu_launcher().await;
    Ok(ProtonStatus {
        in_use: proton::pick(&installed, default_build.as_deref()).map(|b| b.name),
        latest_umu: latest(&umu),
        latest_ge: latest(&ge),
        umu_tags: tags(&umu),
        ge_tags: tags(&ge),
        installed,
        default_build,
        launcher_version: launcher.as_ref().ok().and_then(|l| l.version.clone()),
        launcher_problem: launcher.err(),
        supports_32bit: gameyfin_core::runtime::has_32bit_support(),
    })
}

/// The newest release of a family, or one named tag.
#[tauri::command]
pub async fn install_proton(
    app: AppHandle,
    state: State<'_, AppState>,
    family: String,
    tag: Option<String>,
) -> CommandResult<InstalledProton> {
    let family = ProtonFamily::parse(&family)
        .ok_or_else(|| CommandError::msg(format!("{family} is not a Proton family.")))?;
    let releases = proton::releases(&state.http(), family)
        .await
        .context(format!("could not look up {}", family.label()))?;
    let release = match tag {
        Some(tag) => releases.into_iter().find(|r| r.tag == tag),
        None => releases
            .into_iter()
            .find(|r| !proton::is_prerelease_tag(&r.tag)),
    }
    .ok_or_else(|| CommandError::msg(format!("That {} release was not found.", family.label())))?;
    let _runtime_lock = state.runtime_lock().await;
    download(&app, &state, &release).await
}

/// A game pinned to the removed build runs the default next time.
#[tauri::command]
pub async fn remove_proton(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> CommandResult<()> {
    proton::remove(&state.config_dir(), &name)
        .await
        .context(format!("could not remove {name}"))?;
    if state.settings().default_proton.as_deref() == Some(name.as_str()) {
        state.set_settings(|s| s.default_proton = None).await?;
    }
    let _ = app.emit("proton-changed", ());
    Ok(())
}
