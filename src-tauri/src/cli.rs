//! `gameyfin-app --launch <id>`, so a shortcut takes the same path as pressing Play. A
//! second process hands its arguments over rather than fighting over the library file.

use tauri::{AppHandle, Manager};

/// The game id in a `--launch` argument. Unknown arguments belong to the webview runtime.
pub fn launch_target(args: &[String]) -> Option<i64> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--launch=") {
            return value.parse().ok();
        }
        if arg == "--launch" {
            return iter.next().and_then(|value| value.parse().ok());
        }
    }
    None
}

/// Carries the launcher's activation token, since a running copy only receives arguments.
const ACTIVATION_FLAG: &str = "--activation-token=";

/// The token a Wayland compositor wants before it lets a window come to the front.
pub fn activation_token(args: &[String]) -> Option<String> {
    args.iter()
        .skip(1)
        .find_map(|arg| arg.strip_prefix(ACTIVATION_FLAG))
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

/// Restarts with the launcher's token as an argument. GTK 3 never reads `XDG_ACTIVATION_TOKEN`,
/// and the single-instance hand-over drops the environment.
#[cfg(target_os = "linux")]
pub fn forward_activation_token(args: &[String]) {
    use std::os::unix::process::CommandExt;

    if activation_token(args).is_some() {
        return;
    }
    let Some(token) = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|token| !token.is_empty())
    else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let error = std::process::Command::new(exe)
        .args(args.iter().skip(1))
        .arg(format!("{ACTIVATION_FLAG}{token}"))
        .exec();
    eprintln!("could not restart with the activation token: {error}");
}

/// Whether this process was started only to report what Vulkan the machine supports.
pub fn is_vulkan_probe(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .any(|arg| arg == gameyfin_core::vulkan::PROBE_FLAG)
}

/// Must run before the single-instance plugin, which would hand the argument over and
/// leave this process printing nothing.
pub fn run_vulkan_probe() -> ! {
    let support = gameyfin_core::vulkan::probe();
    println!("{}", serde_json::to_string(&support).unwrap_or_default());
    std::process::exit(0);
}

/// Reveals the window either way, so a failure has somewhere to be explained.
pub async fn handle_launch(app: &AppHandle, game_id: i64, token: Option<String>) {
    tracing::info!(game_id, "launching from a shortcut");
    crate::tray::reveal_activated(app, Some("/installed"), token);

    if let Err(e) =
        crate::ipc::launch::launch_game(app.clone(), app.state::<crate::state::AppState>(), game_id)
            .await
    {
        let title = app.state::<crate::state::AppState>().title(game_id).await;
        crate::notify::failed(app, "Launch", &title, &e.to_string()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(rest: &[&str]) -> Vec<String> {
        std::iter::once("gameyfin-app".to_string())
            .chain(rest.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn the_vulkan_probe_runs_only_when_its_own_flag_is_present() {
        assert!(is_vulkan_probe(&args(&["--vulkan-probe"])));
        assert!(is_vulkan_probe(&args(&["--hidden", "--vulkan-probe"])));
        assert!(!is_vulkan_probe(&args(&[])));
        assert!(!is_vulkan_probe(&args(&["--launch", "7"])));
        // The program's own name is skipped, so a copy installed at that path is not a probe.
        assert!(!is_vulkan_probe(&["--vulkan-probe".to_string()]));
    }

    #[test]
    fn launch_target_reads_only_a_well_formed_game_id() {
        for (argv, expected) in [
            (vec!["--launch", "12"], Some(12)),
            (vec!["--launch=12"], Some(12)),
            (vec![], None),
            (vec!["--no-sandbox"], None),
            (vec!["--launch", "celeste"], None),
            (vec!["--launch"], None),
            (vec!["--launch="], None),
            (vec!["--no-sandbox", "--launch", "7"], Some(7)),
        ] {
            assert_eq!(launch_target(&args(&argv)), expected, "{argv:?}");
        }
    }

    #[test]
    fn the_activation_token_is_read_only_from_its_own_flag() {
        assert_eq!(
            activation_token(&args(&["--launch", "7", "--activation-token=kwin-42"])),
            Some("kwin-42".to_string())
        );
        assert_eq!(activation_token(&args(&["--activation-token="])), None);
        assert_eq!(activation_token(&args(&["--hidden"])), None);
        assert_eq!(
            activation_token(&["--activation-token=x".to_string()]),
            None
        );
    }

    #[test]
    fn the_programs_own_name_is_never_read_as_an_argument() {
        // argv[0] can be anything, including something that looks like a flag.
        let odd = vec![
            "--launch=99".to_string(),
            "--launch".to_string(),
            "3".to_string(),
        ];
        assert_eq!(launch_target(&odd), Some(3));
    }
}
