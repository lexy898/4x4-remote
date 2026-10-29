//! The safety net for the message files: every language directory under `i18n/` must parse, and
//! must define the same messages, using the same `$variables`, as the English fallback.
//!
//! `fl!` checks only the fallback language at compile time, so a translation that lacks a
//! message, or that forgot a variable, would go unnoticed until a user saw the wrong text. The
//! checks look at every directory, so adding a language needs no change here.
//!
//! Only the variables written in a message itself count: the ones it would get by referring to
//! another message are not followed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use fluent_syntax::ast::{
    CallArguments, Entry, Expression, InlineExpression, Pattern, PatternElement, Resource,
};
use fluent_syntax::parser;

/// The language every other one is compared with.
const FALLBACK: &str = "en";
/// The message file of a language; the domain in `i18n.toml`.
const MESSAGE_FILE: &str = "x4.ftl";

/// What a message file defines: each message (`id`), message attribute (`id.attribute`) and term
/// (`-id`), with the variables its text uses.
type Catalog = BTreeMap<String, BTreeSet<String>>;

fn i18n_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n")
}

/// The language directories under `i18n/`, by language code.
fn languages() -> BTreeMap<String, PathBuf> {
    let entries = fs::read_dir(i18n_dir()).expect("the i18n directory is readable");
    entries
        .map(|entry| entry.expect("a directory entry is readable").path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let code = path.file_name()?.to_str()?.to_owned();
            Some((code, path))
        })
        .collect()
}

fn read_messages(language: &str, directory: &Path) -> String {
    let file = directory.join(MESSAGE_FILE);
    fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("language {language}: cannot read {file:?}: {error}"))
}

/// Parses `source`; on failure, one line per syntax error.
fn parse(source: &str) -> Result<Resource<&str>, Vec<String>> {
    parser::parse(source).map_err(|(_, errors)| {
        errors
            .iter()
            .map(|error| format!("{error} (bytes {}..{})", error.pos.start, error.pos.end))
            .collect()
    })
}

fn catalog(source: &str) -> Result<Catalog, Vec<String>> {
    let resource = parse(source)?;
    let mut catalog = Catalog::new();
    for entry in &resource.body {
        match entry {
            Entry::Message(message) => {
                let id = message.id.name;
                if let Some(value) = &message.value {
                    add_pattern(&mut catalog, id.to_owned(), value);
                }
                for attribute in &message.attributes {
                    let key = format!("{id}.{}", attribute.id.name);
                    add_pattern(&mut catalog, key, &attribute.value);
                }
            }
            Entry::Term(term) => {
                let id = format!("-{}", term.id.name);
                add_pattern(&mut catalog, id.clone(), &term.value);
                for attribute in &term.attributes {
                    let key = format!("{id}.{}", attribute.id.name);
                    add_pattern(&mut catalog, key, &attribute.value);
                }
            }
            Entry::Comment(_)
            | Entry::GroupComment(_)
            | Entry::ResourceComment(_)
            | Entry::Junk { .. } => {}
        }
    }
    Ok(catalog)
}

fn add_pattern(catalog: &mut Catalog, key: String, pattern: &Pattern<&str>) {
    let mut variables = BTreeSet::new();
    pattern_variables(pattern, &mut variables);
    catalog.insert(key, variables);
}

fn pattern_variables(pattern: &Pattern<&str>, into: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        match element {
            PatternElement::TextElement { .. } => {}
            PatternElement::Placeable { expression } => expression_variables(expression, into),
        }
    }
}

fn expression_variables(expression: &Expression<&str>, into: &mut BTreeSet<String>) {
    match expression {
        Expression::Inline(inline) => inline_variables(inline, into),
        Expression::Select { selector, variants } => {
            inline_variables(selector, into);
            for variant in variants {
                pattern_variables(&variant.value, into);
            }
        }
    }
}

fn inline_variables(inline: &InlineExpression<&str>, into: &mut BTreeSet<String>) {
    match inline {
        InlineExpression::VariableReference { id } => {
            into.insert(id.name.to_owned());
        }
        InlineExpression::FunctionReference { arguments, .. }
        | InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => call_variables(arguments, into),
        InlineExpression::Placeable { expression } => expression_variables(expression, into),
        InlineExpression::StringLiteral { .. }
        | InlineExpression::NumberLiteral { .. }
        | InlineExpression::MessageReference { .. }
        | InlineExpression::TermReference {
            arguments: None, ..
        } => {}
    }
}

fn call_variables(arguments: &CallArguments<&str>, into: &mut BTreeSet<String>) {
    for positional in &arguments.positional {
        inline_variables(positional, into);
    }
    for named in &arguments.named {
        inline_variables(&named.value, into);
    }
}

/// The messages `reference` lacks and the ones it has that `fallback` does not.
fn id_differences(fallback: &Catalog, reference: &Catalog) -> Vec<String> {
    let missing = fallback
        .keys()
        .filter(|id| !reference.contains_key(*id))
        .map(|id| format!("missing: {id}"));
    let extra = reference
        .keys()
        .filter(|id| !fallback.contains_key(*id))
        .map(|id| format!("not in {FALLBACK}: {id}"));
    missing.chain(extra).collect()
}

/// The messages both define whose `$variables` differ.
fn variable_differences(fallback: &Catalog, reference: &Catalog) -> Vec<String> {
    fallback
        .iter()
        .filter_map(|(id, expected)| {
            let actual = reference.get(id)?;
            (actual != expected)
                .then(|| format!("{id}: uses {actual:?}, but {FALLBACK} uses {expected:?}"))
        })
        .collect()
}

/// Every language's catalog, panicking with the syntax errors of a file that does not parse.
fn all_catalogs() -> BTreeMap<String, Catalog> {
    languages()
        .into_iter()
        .map(|(code, directory)| {
            let source = read_messages(&code, &directory);
            let catalog = catalog(&source).unwrap_or_else(|errors| {
                panic!(
                    "language {code}: {MESSAGE_FILE} has errors:\n{}",
                    errors.join("\n")
                )
            });
            (code, catalog)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `check` on every language but the fallback and fails with all the differences.
    fn assert_every_language_matches_the_fallback(check: fn(&Catalog, &Catalog) -> Vec<String>) {
        let catalogs = all_catalogs();
        let fallback = catalogs
            .get(FALLBACK)
            .expect("the fallback language has a directory");
        let problems: Vec<String> = catalogs
            .iter()
            .filter(|(code, _)| code.as_str() != FALLBACK)
            .flat_map(|(code, catalog)| {
                check(fallback, catalog)
                    .into_iter()
                    .map(move |problem| format!("{code}: {problem}"))
            })
            .collect();
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn ftl_files_parse_without_errors() {
        let languages = languages();
        assert!(
            languages.contains_key(FALLBACK),
            "there is no i18n/{FALLBACK} directory"
        );
        for (code, directory) in &languages {
            let source = read_messages(code, directory);
            if let Err(errors) = parse(&source) {
                panic!(
                    "language {code}: {MESSAGE_FILE} has errors:\n{}",
                    errors.join("\n")
                );
            }
        }
    }

    #[test]
    fn locales_have_the_same_messages_as_en() {
        assert_every_language_matches_the_fallback(id_differences);
    }

    #[test]
    fn locales_use_the_same_variables_as_en() {
        assert_every_language_matches_the_fallback(variable_differences);
    }

    fn catalog_of(source: &str) -> Catalog {
        catalog(source).expect("the test source parses")
    }

    #[test]
    fn a_syntax_error_is_reported() {
        let errors = catalog("good = Fine\nbad = { $x\n").expect_err("the source is broken");
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    #[test]
    fn a_missing_message_is_reported() {
        let fallback = catalog_of("a = One\nb = Two\n");
        let translation = catalog_of("a = Uno\n");
        assert_eq!(id_differences(&fallback, &translation), ["missing: b"]);
    }

    #[test]
    fn an_extra_message_is_reported() {
        let fallback = catalog_of("a = One\n");
        let translation = catalog_of("a = Uno\nc = Tres\n");
        assert_eq!(id_differences(&fallback, &translation), ["not in en: c"]);
    }

    #[test]
    fn an_attribute_counts_as_a_message() {
        let fallback = catalog_of("a = One\n    .title = Hint\n");
        let translation = catalog_of("a = Uno\n");
        assert_eq!(
            id_differences(&fallback, &translation),
            ["missing: a.title"]
        );
    }

    #[test]
    fn a_missing_variable_is_reported() {
        let fallback = catalog_of("a = { $size } of { $total }\n");
        let translation = catalog_of("a = { $size }\n");
        let differences = variable_differences(&fallback, &translation);
        assert_eq!(differences.len(), 1, "{differences:?}");
        assert!(differences[0].starts_with("a: "), "{differences:?}");
    }

    #[test]
    fn a_renamed_variable_is_reported() {
        let fallback = catalog_of("a = { $count }\n");
        let translation = catalog_of("a = { $number }\n");
        assert_eq!(variable_differences(&fallback, &translation).len(), 1);
    }

    #[test]
    fn variables_in_selectors_variants_and_functions_count() {
        let source = "a = { $n ->\n    [one] { $n } file of { NUMBER($total, minimumFractionDigits: 1) }\n   *[other] { $n } files\n}\n";
        let found = catalog_of(source);
        let expected: BTreeSet<String> = ["n", "total"].map(str::to_owned).into();
        assert_eq!(found.get("a"), Some(&expected));
    }

    #[test]
    fn the_same_variables_in_another_order_are_no_difference() {
        let fallback = catalog_of("a = { $x } { $y }\n");
        let translation = catalog_of("a = { $y } { $x }\n");
        assert_eq!(
            variable_differences(&fallback, &translation),
            Vec::<String>::new()
        );
    }
}
