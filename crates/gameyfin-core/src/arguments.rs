//! Turning a typed-in argument string into an argument list.
//!
//! Per-game launch and installer options are one text box, because that is how everyone
//! writes them down and how every guide and forum post presents them: `-windowed -dx11`,
//! `/VERYSILENT /DIR="C:\Games\Thing"`. They have to become a real argument vector, since
//! the process is spawned directly rather than through a shell.
//!
//! Deliberately **not** a shell. No variable expansion, no globbing, no command
//! substitution, no operators: a `$HOME` or a `;` in this box is a literal, because the
//! box is for arguments and treating it as a command line is how a text field becomes an
//! execution vector. Quoting is the only syntax, and only because paths contain spaces.

/// Split a typed argument string into individual arguments.
///
/// Single and double quotes both group. A backslash escapes the following character
/// **outside** quotes, and inside double quotes only when it precedes a quote or another
/// backslash.
///
/// That last rule is the important one, and it differs from a POSIX shell on purpose. The
/// arguments people paste here are overwhelmingly Windows installer flags, and
/// `/DIR="C:\Program Files\Thing"` has to survive with its separators intact. Treating
/// every backslash in a quoted string as an escape would silently turn that into
/// `C:Program FilesThing`, which fails at install time with an error that points nowhere
/// near this text box.
pub fn split(input: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    // Whether anything has been contributed to `current`, so an explicitly empty argument
    // (`""`) survives while ordinary whitespace does not produce a blank one.
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = input.chars();

    while let Some(c) = chars.next() {
        match c {
            // Outside quotes a backslash escapes whatever follows; inside double quotes
            // it only does so for a quote or another backslash, leaving Windows path
            // separators alone. Inside single quotes nothing is special at all.
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
                    // A separator, not an escape.
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

/// Render an argument list back into something a person can edit.
///
/// The inverse of [`split`] for every list it produces, so a value can be shown, edited
/// and stored without drifting.
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
        // The rule that differs from a shell, and the reason for it: this is what people
        // paste, and eating the backslashes fails far from where the mistake was made.
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
        // Long-standing shell convention, and what a Windows path pasted between single
        // quotes needs in order to survive.
        assert_eq!(split(r"'C:\Games\Thing'"), vec![r"C:\Games\Thing"]);
    }

    #[test]
    fn an_explicitly_empty_argument_survives() {
        // Some installers are given an empty value on purpose.
        assert_eq!(split(r#"-a "" -b"#), vec!["-a", "", "-b"]);
    }

    #[test]
    fn shell_syntax_is_not_interpreted() {
        // The box is for arguments. Treating it as a command line would make a text field
        // an execution vector, and none of this has any business running.
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
