//! Windows' per-program compatibility settings, which installers use to tick "Run this program
//! as an administrator" on games that run fine without it.

use std::path::Path;

/// Set on a program's environment, it runs without the elevation its settings ask for.
pub const RUN_AS_INVOKER: (&str, &str) = ("__COMPAT_LAYER", "RunAsInvoker");

#[cfg(windows)]
const LAYERS_KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers";

/// Unticks "Run this program as an administrator" for the current user, the way the
/// Compatibility tab does. False when it was not ticked there or could not be changed.
pub fn clear_run_as_admin(program: &Path) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;

        let name = value_name(program);
        let Some(layers) = crate::registry::string(HKEY_CURRENT_USER, LAYERS_KEY, &name) else {
            return false;
        };
        match without_run_as_admin(&layers) {
            None => false,
            Some(None) => crate::registry::delete_value(HKEY_CURRENT_USER, LAYERS_KEY, &name),
            Some(Some(rest)) => {
                crate::registry::set_string(HKEY_CURRENT_USER, LAYERS_KEY, &name, &rest)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = program;
        false
    }
}

/// The value is the program's full path, spelled the way Explorer writes it.
#[cfg(any(windows, test))]
fn value_name(program: &Path) -> String {
    let path = program.to_string_lossy().replace('/', "\\");
    path.strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or(path)
}

/// The layers without `RUNASADMIN`: `None` when it was not there, `Some(None)` when nothing
/// else is left. Tokens like `~` are markers, not settings.
#[cfg(any(windows, test))]
fn without_run_as_admin(layers: &str) -> Option<Option<String>> {
    let tokens: Vec<&str> = layers.split_whitespace().collect();
    let rest: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|t| !t.eq_ignore_ascii_case("RUNASADMIN"))
        .collect();
    if rest.len() == tokens.len() {
        return None;
    }
    let settings = rest.iter().any(|t| t.chars().any(char::is_alphanumeric));
    Some(settings.then(|| rest.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_administrator_box_is_unticked() {
        assert_eq!(without_run_as_admin("~ RUNASADMIN"), Some(None));
        assert_eq!(without_run_as_admin("RunAsAdmin"), Some(None));
        assert_eq!(
            without_run_as_admin("~ RUNASADMIN WIN7RTM HIGHDPIAWARE"),
            Some(Some("~ WIN7RTM HIGHDPIAWARE".to_string()))
        );
        assert_eq!(without_run_as_admin("~ WIN7RTM"), None);
        assert_eq!(without_run_as_admin(""), None);
    }

    #[test]
    fn the_value_is_named_after_the_plain_windows_path() {
        assert_eq!(
            value_name(Path::new(
                r"\\?\C:\Games\(59) Risk of Rain 2/Risk of Rain 2.exe"
            )),
            r"C:\Games\(59) Risk of Rain 2\Risk of Rain 2.exe"
        );
    }
}
