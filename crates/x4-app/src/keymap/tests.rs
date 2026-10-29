//! The README's table of keys says what [`BINDINGS`] binds, and where.

use std::collections::BTreeSet;

use gpui_kit::{Action as _, NoAction};

use super::{
    BINDINGS, MAIN_CONTEXT, MAIN_OUTSIDE_TEXT_CONTEXT, SEARCH_CONTEXT, TABLE_CONTEXT,
    TABLE_NO_MENU, TORRENTS,
};

const README: &str = include_str!("../../../../README.md");

/// Keys bound here in the table only, which would take them for itself; everywhere else the
/// toolkit's own binding does the same, so the README gives them for anywhere.
const ALSO_ELSEWHERE: &[&str] = &["tab", "shift-tab"];

/// Where the README says that `keys`, bound in `context`, work.
fn place(keys: &str, context: Option<&str>) -> &'static str {
    if ALSO_ELSEWHERE.contains(&keys) {
        return "anywhere";
    }
    match context {
        None => "anywhere",
        Some(MAIN_CONTEXT) => "anywhere in the window",
        Some(MAIN_OUTSIDE_TEXT_CONTEXT) => "anywhere but a text field",
        Some(SEARCH_CONTEXT) => "in the search box",
        Some(TABLE_CONTEXT | TABLE_NO_MENU | TORRENTS) => "in the table",
        Some(other) => panic!("the README has no place for keys bound in {other:?}"),
    }
}

/// `keys` in gpui's spelling, such as `ctrl-shift-a`, as the README writes them: `Ctrl+Shift+A`.
fn spelled(keys: &str) -> String {
    keys.split('-')
        .map(|part| match part {
            "pageup" => "Page Up".to_owned(),
            "pagedown" => "Page Down".to_owned(),
            _ => {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Each key of the README's table of keys with the place of its row, such as
/// `("Ctrl+S", "in the table")`.
fn readme_keys() -> BTreeSet<(String, String)> {
    let section = README
        .split("\n### Keys\n")
        .nth(1)
        .expect("the README has a section of keys");
    section
        .lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        // The header and the line under it.
        .skip(2)
        .flat_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let (keys, place) = (cells[1], cells[2]);
            keys.split('`')
                .skip(1)
                .step_by(2)
                .map(move |key| (key.to_owned(), place.to_owned()))
        })
        .collect()
}

#[test]
fn the_readme_lists_every_key_binding_where_it_works() {
    let readme = readme_keys();
    let bound: BTreeSet<(String, String)> = BINDINGS
        .iter()
        .filter(|binding| (binding.action)().name() != NoAction.name())
        .map(|binding| {
            let place = place(binding.keys, binding.context);
            (spelled(binding.keys), place.to_owned())
        })
        .collect();

    let missing: Vec<_> = bound.difference(&readme).collect();
    assert!(
        missing.is_empty(),
        "bound, but not in the README's table of keys: {missing:?}"
    );
    // The other places of the table (a menu, a dialog, the speed limits) are the toolkit's keys.
    let places: BTreeSet<&str> = bound.iter().map(|(_, place)| place.as_str()).collect();
    let unbound: Vec<_> = readme
        .iter()
        .filter(|pair| places.contains(pair.1.as_str()) && !bound.contains(*pair))
        .collect();
    assert!(
        unbound.is_empty(),
        "in the README's table of keys, but not bound there: {unbound:?}"
    );
}

#[test]
fn keys_are_spelled_as_the_readme_writes_them() {
    assert_eq!(spelled("ctrl-shift-a"), "Ctrl+Shift+A");
    assert_eq!(spelled("pagedown"), "Page Down");
    assert_eq!(spelled("shift-delete"), "Shift+Delete");
    assert_eq!(spelled("/"), "/");
}
