//! Split a typed argument string into an argv, deliberately not a shell: no expansion,
//! globbing, substitution or operators, so a text field cannot become an execution vector.

/// Split a typed argument string into individual arguments.
///
/// Backslash-in-double-quotes is kept literal (unlike POSIX) so pasted Windows paths like
/// `/DIR="C:\Program Files\Thing"` survive with their separators intact.
pub fn split(input: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    // Tracks whether `current` got any input, so an explicit `""` survives but whitespace
    // does not produce a blank argument.
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = input.chars();

    while let Some(c) = chars.next() {
        match c {
            '\\' if quote.is_none() => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    started = true;
                }
            }
            '\\' if quote == Some('"') => {
                match chars.clone().next() {
                    Some(next @ ('"' | '\\')) => {
                        chars.next();
                        current.push(next);
                    }
                    // Windows path separator, not an escape.
                    _ => current.push('\\'),
                }
                started = true;
            }
            '"' | '\'' if quote.is_none() => {
                quote = Some(c);
                started = true;
            }
            c if Some(c) == quote => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }

    if started {
        arguments.push(current);
    }
    arguments
}

/// Render an argument list back into an editable string; the inverse of [`split`].
pub fn join(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if argument.is_empty() {
                "\"\"".to_string()
            } else if argument.contains([' ', '\t', '"', '\'', '\\']) {
                let escaped = argument.replace('\\', r"\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            } else {
                argument.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(split("-windowed -dx11"), vec!["-windowed", "-dx11"]);
        assert_eq!(split("  -a   -b  "), vec!["-a", "-b"]);
        assert_eq!(split(""), Vec::<String>::new());
        assert_eq!(split("   "), Vec::<String>::new());
    }

    #[test]
    fn quotes_group_a_path_with_spaces() {
        // The reason quoting is supported at all.
        assert_eq!(
            split(r#"/DIR="C:\Program Files\Thing""#),
            vec![r"/DIR=C:\Program Files\Thing"]
        );
        assert_eq!(split("'one two' three"), vec!["one two", "three"]);
    }

    #[test]
    fn a_backslash_escapes_the_next_character_outside_quotes() {
        assert_eq!(split(r#"a\ b"#), vec!["a b"]);
    }

    #[test]
    fn a_windows_path_keeps_its_separators_inside_quotes() {
        // The rule that differs from a shell: this is what people paste.
        assert_eq!(
            split(r#""C:\Program Files\Thing\setup.exe""#),
            vec![r"C:\Program Files\Thing\setup.exe"]
        );
        // A quote can still be escaped, which is the only thing that needs to be.
        assert_eq!(split(r#""say \"hi\"""#), vec![r#"say "hi""#]);
        // A doubled backslash is one literal separator, so a path may end in one.
        assert_eq!(split(r#""C:\Games\\""#), vec!["C:\\Games\\"]);
    }

    #[test]
    fn single_quotes_take_a_backslash_literally() {
        assert_eq!(split(r"'C:\Games\Thing'"), vec![r"C:\Games\Thing"]);
    }

    #[test]
    fn an_explicitly_empty_argument_survives() {
        // Some installers are given an empty value on purpose.
        assert_eq!(split(r#"-a "" -b"#), vec!["-a", "", "-b"]);
    }

    #[test]
    fn shell_syntax_is_not_interpreted() {
        assert_eq!(split("-a; rm -rf /"), vec!["-a;", "rm", "-rf", "/"]);
        assert_eq!(split("$HOME"), vec!["$HOME"]);
        assert_eq!(split("`id`"), vec!["`id`"]);
        assert_eq!(split("a && b"), vec!["a", "&&", "b"]);
        assert_eq!(split("*.exe"), vec!["*.exe"]);
        assert_eq!(split("a|b"), vec!["a|b"]);
    }

    #[test]
    fn an_unterminated_quote_still_yields_the_argument() {
        // Someone is mid-edit; refusing to parse would mean losing what they typed.
        assert_eq!(split(r#"-a "unfinished"#), vec!["-a", "unfinished"]);
    }

    #[test]
    fn joining_is_the_inverse_of_splitting() {
        for original in [
            "-windowed -dx11",
            r#"/DIR="C:\Program Files\Thing""#,
            r#"-a "" -b"#,
            "'one two' three",
        ] {
            let parts = split(original);
            assert_eq!(split(&join(&parts)), parts, "round trip of {original:?}");
        }
    }

    #[test]
    fn joining_quotes_only_what_needs_it() {
        assert_eq!(join(&["-a".into(), "-b".into()]), "-a -b");
        assert_eq!(join(&["one two".into()]), "\"one two\"");
        assert_eq!(join(&[String::new()]), "\"\"");
    }
}
