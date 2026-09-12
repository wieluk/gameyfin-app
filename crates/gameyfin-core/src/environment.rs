//! Per-game environment variables as `KEY=value` lines, split with [`crate::arguments::split`]
//! so quoting works but a pasted value can never reach a shell.

use std::collections::BTreeMap;

/// Read a typed block into variables, ignoring blank lines and `#` comments.
pub fn parse(input: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // One line can hold several assignments, which is how a ProtonDB line is written.
        for token in crate::arguments::split(line) {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            let key = key.trim();
            if !key.is_empty() {
                out.insert(key.to_string(), value.to_string());
            }
        }
    }
    out
}

/// Render variables back into the block a user sees.
pub fn format(vars: &BTreeMap<String, String>) -> String {
    vars.iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(input: &str) -> Vec<(String, String)> {
        parse(input).into_iter().collect()
    }

    #[test]
    fn one_assignment_per_line_is_the_common_case() {
        assert_eq!(
            parsed("DXVK_HUD=fps\nVKD3D_CONFIG=dxr11"),
            vec![
                ("DXVK_HUD".to_string(), "fps".to_string()),
                ("VKD3D_CONFIG".to_string(), "dxr11".to_string()),
            ]
        );
    }

    #[test]
    fn a_pasted_protondb_line_holding_several_assignments_is_understood() {
        assert_eq!(
            parsed("PROTON_USE_WINED3D=1 DXVK_HUD=fps"),
            vec![
                ("DXVK_HUD".to_string(), "fps".to_string()),
                ("PROTON_USE_WINED3D".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn a_quoted_value_keeps_its_spaces() {
        assert_eq!(
            parsed(r#"WINEDLLOVERRIDES="d3d11=n,b;dxgi=n,b""#),
            vec![(
                "WINEDLLOVERRIDES".to_string(),
                "d3d11=n,b;dxgi=n,b".to_string()
            )]
        );
    }

    #[test]
    fn blank_lines_comments_and_lines_without_an_equals_sign_are_dropped() {
        assert_eq!(parsed("\n  \n# DXVK_HUD=fps\njust-a-word\n"), vec![]);
    }

    #[test]
    fn an_empty_value_is_kept_because_unsetting_is_a_real_intent() {
        assert_eq!(
            parsed("DXVK_HUD="),
            vec![("DXVK_HUD".to_string(), String::new())]
        );
    }

    #[test]
    fn shell_syntax_is_not_interpreted() {
        // The value is taken as written, so nothing here reaches a shell.
        assert_eq!(
            parsed("EVIL=$(rm -rf /)"),
            vec![("EVIL".to_string(), "$(rm".to_string())]
        );
    }

    #[test]
    fn the_block_round_trips_through_parse_and_format() {
        let text = "DXVK_HUD=fps\nVKD3D_CONFIG=dxr11";
        assert_eq!(format(&parse(text)), text);
    }
}
