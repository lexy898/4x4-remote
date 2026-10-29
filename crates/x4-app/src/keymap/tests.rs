//! The README's table of keys says what [`BINDINGS`] binds, and where; and an open menu keeps
//! the keys that work outside it.

use std::collections::BTreeSet;

use gpui_kit::{Action as _, KeyBindingContextPredicate, KeyContext, NoAction};

use super::{
    ANY_MENU, BINDINGS, Binding, MAIN_CONTEXT, MAIN_OUTSIDE_TEXT_CONTEXT, SEARCH_CONTEXT,
    TABLE_CONTEXT, TABLE_MENU, TABLE_NO_MENU, TORRENT_LIST_CONTEXT, TORRENTS,
};

const README: &str = include_str!("../../../../README.md");

/// Keys bound here in the table only, which would take them for itself; everywhere else the
/// toolkit's own binding does the same, so the README gives them for anywhere.
const ALSO_ELSEWHERE: &[&str] = &["tab", "shift-tab"];

/// The places of the README's table of keys where [`BINDINGS`] binds keys, as `place` names them.
const ANYWHERE: &str = "anywhere";
const IN_THE_WINDOW: &str = "anywhere in the window";
const OUTSIDE_TEXT: &str = "anywhere but a text field";
const IN_THE_SEARCH_BOX: &str = "in the search box";
const IN_THE_TABLE: &str = "in the table";
const BOUND_PLACES: &[&str] = &[
    ANYWHERE,
    IN_THE_WINDOW,
    OUTSIDE_TEXT,
    IN_THE_SEARCH_BOX,
    IN_THE_TABLE,
];
/// The places of the README's table of keys where the keys are the toolkit's own.
const TOOLKIT_PLACES: &[&str] = &["in a menu", "in a removal dialog", "in the speed limits"];

/// Where the README says that `keys`, bound in `context`, work.
fn place(keys: &str, context: Option<&str>) -> &'static str {
    if ALSO_ELSEWHERE.contains(&keys) {
        return ANYWHERE;
    }
    match context {
        None => ANYWHERE,
        Some(MAIN_CONTEXT) => IN_THE_WINDOW,
        Some(MAIN_OUTSIDE_TEXT_CONTEXT) => OUTSIDE_TEXT,
        Some(SEARCH_CONTEXT) => IN_THE_SEARCH_BOX,
        Some(TABLE_CONTEXT | TABLE_NO_MENU | TORRENTS) => IN_THE_TABLE,
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
    key_table(section).unwrap_or_else(|problem| panic!("the README's table of keys: {problem}"))
}

/// Each key of the first table in `section` with the place of its row. A row must have three
/// cells: the keys, each in backticks, a known place, and what they do.
fn key_table(section: &str) -> Result<BTreeSet<(String, String)>, String> {
    let mut keys = BTreeSet::new();
    let rows = section
        .lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        // The header and the line under it.
        .skip(2);
    for row in rows {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let ["", row_keys, place, _what, ""] = cells.as_slice() else {
            return Err(format!(
                "a row without its three cells (keys, where, what): {row:?}"
            ));
        };
        if !BOUND_PLACES.contains(place) && !TOOLKIT_PLACES.contains(place) {
            return Err(format!("a row with an unknown place {place:?}: {row:?}"));
        }
        let row_keys: Vec<&str> = row_keys.split('`').skip(1).step_by(2).collect();
        if row_keys.is_empty() {
            return Err(format!("a row that names no key in backticks: {row:?}"));
        }
        keys.extend(
            row_keys
                .into_iter()
                .map(|key| (key.to_owned(), (*place).to_owned())),
        );
    }
    Ok(keys)
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
fn a_malformed_row_of_the_key_table_is_reported_clearly() {
    const HEADER: &str = "| Key | Where | What it does |\n| --- | --- | --- |\n";
    let table = |rows: &str| key_table(&format!("{HEADER}{rows}"));

    assert_eq!(
        table("| `Ctrl+S` | in the table | starts them |\n| `Ctrl+P` | in the table | pauses |\n"),
        Ok([
            ("Ctrl+P".to_owned(), IN_THE_TABLE.to_owned()),
            ("Ctrl+S".to_owned(), IN_THE_TABLE.to_owned()),
        ]
        .into())
    );
    let short = table("| `Ctrl+S` | in the table |\n");
    assert!(
        short
            .as_ref()
            .is_err_and(|problem| problem.contains("three cells")),
        "{short:?}"
    );
    let no_key = table("| Ctrl+S | in the table | starts them |\n");
    assert!(
        no_key
            .as_ref()
            .is_err_and(|problem| problem.contains("names no key")),
        "{no_key:?}"
    );
}

#[test]
fn a_row_of_the_key_table_with_an_unknown_place_is_flagged() {
    let table = key_table(
        "| Key | Where | What it does |\n| --- | --- | --- |\n| `Up` | in a menuu | moves |\n",
    );
    assert!(
        table
            .as_ref()
            .is_err_and(|problem| problem.contains("unknown place \"in a menuu\"")),
        "{table:?}"
    );
}

#[test]
fn keys_are_spelled_as_the_readme_writes_them() {
    assert_eq!(spelled("ctrl-shift-a"), "Ctrl+Shift+A");
    assert_eq!(spelled("pagedown"), "Page Down");
    assert_eq!(spelled("shift-delete"), "Shift+Delete");
    assert_eq!(spelled("/"), "/");
}

/// The key contexts around an open menu, from the window down to the menu: the context menu,
/// which is drawn inside the table, and a menu of the toolbar (Remove, Columns).
const OPEN_MENUS: &[&[&str]] = &[
    &[
        MAIN_CONTEXT,
        TORRENT_LIST_CONTEXT,
        TABLE_CONTEXT,
        "PopupMenu",
    ],
    &[MAIN_CONTEXT, "PopupMenu"],
];

/// The keys that may act while a menu is open: Ctrl+Q quits from anywhere, and the menu's own
/// keys, which gpui-component's menu binds deeper than ours, choose an item or close the menu.
const ACT_IN_A_MENU: &[&str] = &["ctrl-q", "enter", "escape", "up", "down", "left", "right"];

fn is_no_action(binding: &Binding) -> bool {
    (binding.action)().name() == NoAction.name()
}

fn predicate(context: &str) -> KeyBindingContextPredicate {
    KeyBindingContextPredicate::parse(context).expect("a context of the key table parses")
}

/// A menu is modal: every key bound outside it, whose context does not leave an open menu out
/// (`!PopupMenu`), is bound to nothing in the menu too, deeper than where it works, so that it
/// does nothing there. Otherwise the key would act behind the menu, or take the focus out of it
/// and leave it open.
#[test]
fn every_key_bound_outside_a_menu_does_nothing_while_one_is_open() {
    let mut acting = Vec::new();
    for stack in OPEN_MENUS {
        let contexts: Vec<KeyContext> = stack
            .iter()
            .map(|context| KeyContext::parse(context).expect("a key context parses"))
            .collect();
        for binding in BINDINGS {
            if is_no_action(binding) || ACT_IN_A_MENU.contains(&binding.keys) {
                continue;
            }
            let Some(context) = binding.context else {
                acting.push((binding.keys, "no context", stack));
                continue;
            };
            let Some(depth) = predicate(context).depth_of(&contexts) else {
                // It does not work here: it leaves an open menu out, or it is bound elsewhere.
                continue;
            };
            let kept = BINDINGS.iter().any(|other| {
                other.keys == binding.keys
                    && is_no_action(other)
                    && other
                        .context
                        .filter(|context| [ANY_MENU, TABLE_MENU].contains(context))
                        .and_then(|context| predicate(context).depth_of(&contexts))
                        .is_some_and(|menu_depth| menu_depth > depth)
            });
            if !kept {
                acting.push((binding.keys, context, stack));
            }
        }
    }
    assert!(
        acting.is_empty(),
        "keys that act while a menu is open, (keys, where bound, around the menu): {acting:?}; \
         bind them to `NoAction` in `ANY_MENU` or `TABLE_MENU`"
    );
}

#[test]
fn the_menu_rule_knows_which_contexts_leave_an_open_menu_out() {
    let context_menu: Vec<KeyContext> = OPEN_MENUS[0]
        .iter()
        .map(|context| KeyContext::parse(context).expect("a key context parses"))
        .collect();
    assert_eq!(predicate(TORRENTS).depth_of(&context_menu), None);
    assert_eq!(predicate(TABLE_NO_MENU).depth_of(&context_menu), None);
    assert_eq!(predicate(TABLE_CONTEXT).depth_of(&context_menu), Some(3));
    assert_eq!(predicate(ANY_MENU).depth_of(&context_menu), Some(4));
}
