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
