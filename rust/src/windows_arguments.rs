//! Windows CRT argument quoting, preserving literal backslashes in paths.
//! Arguments are passed to `Command::args`; no command shell is involved.

pub fn split(input: &str) -> Result<Vec<String>, &'static str> {
    let mut chars = input.chars().peekable();
    let mut result = Vec::new();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| matches!(c, ' ' | '\t')) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut argument = String::new();
        let mut quoted = false;
        while let Some(c) = chars.next() {
            if matches!(c, ' ' | '\t') && !quoted {
                break;
            }
            if c == '\\' {
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    count += 1;
                    chars.next();
                }
                if chars.peek() == Some(&'"') {
                    chars.next();
                    argument.extend(std::iter::repeat_n('\\', count / 2));
                    if count % 2 == 1 {
                        argument.push('"');
                    } else if quoted && chars.peek() == Some(&'"') {
                        argument.push('"');
                        chars.next();
                    } else {
                        quoted = !quoted;
                    }
                } else {
                    argument.extend(std::iter::repeat_n('\\', count));
                }
            } else if c == '"' {
                if quoted && chars.peek() == Some(&'"') {
                    argument.push('"');
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            } else {
                argument.push(c);
            }
        }
        if quoted {
            return Err("Unclosed quote in Windows arguments");
        }
        result.push(argument);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn windows_paths_keep_backslashes_and_unicode() {
        assert_eq!(
            split(r#"--file C:\Games\file.dat "C:\遊戯 Games\file.dat""#).unwrap(),
            ["--file", r"C:\Games\file.dat", r"C:\遊戯 Games\file.dat"]
        );
    }

    #[test]
    fn empty_arguments_and_literal_quotes_follow_crt_rules() {
        assert_eq!(
            split(r#""" "a\"b" "C:\Games\\" 'literal'"#).unwrap(),
            ["", "a\"b", r"C:\Games\", "'literal'"]
        );
        assert_eq!(split(r#""a""b""#).unwrap(), ["a\"b"]);
    }

    #[test]
    fn malformed_quotes_are_actionable() {
        assert!(split("--file \"unfinished").is_err());
        assert!(split(" \t ").unwrap().is_empty());
    }
}

/// Quote one argument using Windows CRT rules, including trailing backslashes.
pub fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            '\\',
            if c == '"' { slashes * 2 + 1 } else { slashes },
        ));
        result.push(c);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}

#[cfg(test)]
mod quote_tests {
    #[test]
    fn shortcut_paths_round_trip_without_becoming_extra_arguments() {
        for value in [
            r"C:\",
            r"C:\Game library\",
            "C:\\café 遊戯\\data",
            "a\"b",
            "",
            "a b --launch injected",
        ] {
            assert_eq!(super::split(&super::quote(value)).unwrap(), [value]);
        }
    }
}
