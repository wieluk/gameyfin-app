//! The command line the desktop and Steam shortcuts use: `gameyfin-app --launch <id>` so a
//! shortcut goes through the same prepare/supervise path as pressing Play. A second process
//! hands its arguments to the running instance rather than fighting over the library file.

use tauri::{AppHandle, Manager};

/// The game id in a `--launch` argument, if there is one. Tolerant of both spellings; silent
/// about anything else since unknown arguments belong to the webview runtime.
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

/// Act on a `--launch` argument. Reveals the window either way, so a launch failure has
/// somewhere to be explained.
pub async fn handle_launch(app: &AppHandle, game_id: i64) {
    tracing::info!(game_id, "launching from a shortcut");
    crate::tray::reveal(app, Some("/installed"));

    let state = app.state::<crate::state::AppState>();
    if let Err(e) = crate::ipc::launch_game(app.clone(), state, game_id).await {
        tracing::error!(game_id, "the shortcut could not launch the game: {e}");
        let title = crate::ipc::title_of_game(app, game_id).await;
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
    fn reads_the_game_id_in_either_spelling() {
        assert_eq!(launch_target(&args(&["--launch", "12"])), Some(12));
        assert_eq!(launch_target(&args(&["--launch=12"])), Some(12));
    }

    #[test]
    fn ignores_a_plain_start() {
        assert_eq!(launch_target(&args(&[])), None);
        assert_eq!(launch_target(&args(&["--no-sandbox"])), None);
    }

    #[test]
    fn a_malformed_id_is_not_a_launch() {
        assert_eq!(launch_target(&args(&["--launch", "celeste"])), None);
        assert_eq!(launch_target(&args(&["--launch"])), None);
        assert_eq!(launch_target(&args(&["--launch="])), None);
    }

    #[test]
    fn finds_the_argument_after_others() {
        assert_eq!(
            launch_target(&args(&["--no-sandbox", "--launch", "7"])),
            Some(7)
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
