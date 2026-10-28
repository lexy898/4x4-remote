//! Telling the user where a TOML file is wrong, without echoing the file.

/// Describes `error`, found while parsing `text`, as "line L, column C: message".
///
/// The error's own display repeats the offending line of the file, and the files may hold things
/// the user does not want in a log. The message and the position say enough to fix the file.
/// Without a position (the error is about the document as a whole) it is the message alone.
pub(crate) fn describe(error: &toml::de::Error, text: &str) -> String {
    match error.span().and_then(|span| position(text, span.start)) {
        Some((line, column)) => format!("line {line}, column {column}: {}", error.message()),
        None => error.message().to_owned(),
    }
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
    fn a_position_past_the_end_is_none() {
        assert_eq!(position("abc", 99), None);
        assert_eq!(position("ab\ncd", 4), Some((2, 2)));
        assert_eq!(position("", 0), Some((1, 1)));
    }
}
