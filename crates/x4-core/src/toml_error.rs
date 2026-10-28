//! Telling the user where a TOML file is wrong, without echoing the file.

/// Describes `error`, found while parsing `text`, as "line L, column C: message".
///
/// The error's own display repeats the offending line of the file, and serde's "invalid type"
/// messages quote the value they found. Files may hold things that must not end up in a log (a
/// URL with a password), so neither is passed on: the position, the kind of value found and what
/// was expected say enough to fix the file. Without a position (the error is about the document
/// as a whole) it is the message alone.
pub(crate) fn describe(error: &toml::de::Error, text: &str) -> String {
    let message = without_value(error.message());
    match error.span().and_then(|span| position(text, span.start)) {
        Some((line, column)) => format!("line {line}, column {column}: {message}"),
        None => message,
    }
}

/// `message` without the value that serde quotes in `invalid type: string "x", expected y` and
/// `invalid value: integer `5`, expected y`: it becomes `invalid type: string, expected y`.
/// Any other message is returned as it is.
fn without_value(message: &str) -> String {
    for prefix in ["invalid type: ", "invalid value: "] {
        let Some(found) = message.strip_prefix(prefix) else {
            continue;
        };
        // The quoted value starts at the first quote. What was expected follows the last comma
        // of that kind, as the value itself may hold text like it.
        let Some(quote) = found.find(['"', '`', '\'']) else {
            return message.to_owned();
        };
        let kind = found.get(..quote).unwrap_or_default().trim_end();
        return match found.rfind(", expected ") {
            Some(expected) if expected >= quote => {
                format!(
                    "{prefix}{kind}{}",
                    found.get(expected..).unwrap_or_default()
                )
            }
            _ => format!("{prefix}{kind}"),
        };
    }
    message.to_owned()
}

/// The 1-based line and column (in characters) of the byte `offset` in `text`.
fn position(text: &str, offset: usize) -> Option<(usize, usize)> {
    let before = text.get(..offset)?;
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let column = before.get(line_start..)?.chars().count() + 1;
    Some((line, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize)]
    struct Probe {
        #[expect(dead_code, reason = "only parsed, to provoke a type error")]
        number: u32,
    }

    fn describe_probe(text: &str) -> String {
        let error = toml::from_str::<Probe>(text).expect_err("the probe text is invalid");
        describe(&error, text)
    }

    #[test]
    fn names_the_line_and_column_of_the_value() {
        let described = describe_probe("# comment\nnumber = \"seven\"\n");

        assert!(described.starts_with("line 2, column 10: "), "{described}");
    }

    #[test]
    fn counts_columns_in_characters_not_bytes() {
        // The bad value is the 7th character, and the 8th byte, after a 2-byte `é`.
        let described = describe_probe("\"é\" = 1 x");

        assert!(described.starts_with("line 1, column 7: "), "{described}");
    }

    #[test]
    fn does_not_repeat_the_line_of_the_file() {
        let described = describe_probe("number = \"seven\"\n");

        assert!(!described.contains('\n'), "{described}");
        assert!(!described.contains("number ="), "{described}");
    }

    #[test]
    fn does_not_repeat_the_value_that_has_the_wrong_type() {
        for text in [
            "number = \"hunter2\"\n",
            "number = \"a, expected b\"\n",
            "number = 1.5\n",
            "number = true\n",
            "number = -4\n",
            "number = 5000000000\n",
        ] {
            let described = describe_probe(text);

            assert!(described.contains(", expected "), "{described}");
            for secret in [
                "hunter2",
                "a, expected b",
                "1.5",
                "true",
                "-4",
                "5000000000",
            ] {
                if text.contains(secret) {
                    assert!(!described.contains(secret), "{described}");
                }
            }
        }
        assert!(
            describe_probe("number = \"hunter2\"\n")
                .ends_with("invalid type: string, expected u32")
        );
        assert!(describe_probe("number = -4\n").ends_with("invalid value: integer, expected u32"));
    }

    #[test]
    fn leaves_other_messages_alone() {
        assert_eq!(without_value("missing field `a`"), "missing field `a`");
        assert_eq!(
            without_value("invalid type: map, expected u32"),
            "invalid type: map, expected u32"
        );
        assert_eq!(
            without_value("invalid type: string \"x\""),
            "invalid type: string"
        );
    }

    #[test]
    fn a_position_past_the_end_is_none() {
        assert_eq!(position("abc", 99), None);
        assert_eq!(position("ab\ncd", 4), Some((2, 2)));
        assert_eq!(position("", 0), Some((1, 1)));
    }
}
