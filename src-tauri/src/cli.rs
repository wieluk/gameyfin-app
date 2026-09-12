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
pub async fn handle_launch(app: &AppHandle, game_id: i64) {
    tracing::info!(game_id, "launching from a shortcut");
    crate::tray::reveal(app, Some("/installed"));

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
