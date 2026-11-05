//! Telling the user where a TOML file is wrong, without echoing the file.

use std::fmt;

/// A place in a text: the 1-based line and column, with columns counted in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextPosition {
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1, in characters.
    pub column: u32,
}

impl TextPosition {
    /// The place of the byte `offset` in `text`; `None` if it is past the end or inside a
    /// character. A line or column beyond `u32::MAX` reads as `u32::MAX`.
    #[must_use]
    pub fn of(text: &str, offset: usize) -> Option<Self> {
        let before = text.get(..offset)?;
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
        let column = before.get(line_start..)?.chars().count() + 1;
        Some(Self {
            line: u32::try_from(line).unwrap_or(u32::MAX),
            column: u32::try_from(column).unwrap_or(u32::MAX),
        })
    }
}

impl fmt::Display for TextPosition {
    /// `line L, column C`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

/// Where `error`, found while parsing `text`, is (when the parser knows) and what it says.
///
/// The error's own display repeats the offending line of the file, and serde's "invalid type"
/// messages quote the value they found. Files may hold things that must not end up in a log (a
/// URL with a password), so neither is passed on: the position, the kind of value found and what
/// was expected say enough to fix the file. An error about the document as a whole has no
/// position.
pub(crate) fn locate(error: &toml::de::Error, text: &str) -> (Option<TextPosition>, String) {
    let position = error
        .span()
        .and_then(|span| TextPosition::of(text, span.start));
    (position, without_value(error.message()))
}

/// Where `error`, found while editing `text`, is (when the parser knows) and what it says.
///
/// Its own display repeats the offending line of the file, so only the message and the position
/// are passed on, as [`locate`] does for serde's errors.
pub(crate) fn locate_edit(
    error: &toml_edit::TomlError,
    text: &str,
) -> (Option<TextPosition>, String) {
    let position = error
        .span()
        .and_then(|span| TextPosition::of(text, span.start));
    (position, error.message().to_owned())
}

/// `message`, after its position if there is one: "line L, column C: message".
pub(crate) fn with_position(position: Option<TextPosition>, message: &str) -> String {
    match position {
        Some(position) => format!("{position}: {message}"),
        None => message.to_owned(),
    }
}

/// Describes `error`, found while parsing `text`, as "line L, column C: message" (see
/// [`locate`]); without a position it is the message alone.
pub(crate) fn describe(error: &toml::de::Error, text: &str) -> String {
    let (position, message) = locate(error, text);
    with_position(position, &message)
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

    fn at(line: u32, column: u32) -> TextPosition {
        TextPosition { line, column }
    }

    #[test]
    fn a_position_past_the_end_is_none() {
        assert_eq!(TextPosition::of("abc", 99), None);
        assert_eq!(TextPosition::of("ab\ncd", 4), Some(at(2, 2)));
        assert_eq!(TextPosition::of("", 0), Some(at(1, 1)));
        assert_eq!(TextPosition::of("é", 1), None, "inside a character");
    }

    #[test]
    fn a_position_reads_as_line_and_column() {
        assert_eq!(
            TextPosition { line: 2, column: 7 }.to_string(),
            "line 2, column 7"
        );
        assert_eq!(
            with_position(Some(at(2, 7)), "boom"),
            "line 2, column 7: boom"
        );
        assert_eq!(with_position(None, "boom"), "boom");
    }

    #[test]
    fn locate_gives_the_position_as_numbers() {
        let text = "# comment\nnumber = \"seven\"\n";
        let error = toml::from_str::<Probe>(text).expect_err("the probe text is invalid");

        let (position, message) = locate(&error, text);

        assert_eq!(position, Some(at(2, 10)));
        assert_eq!(message, "invalid type: string, expected u32");
    }
}
