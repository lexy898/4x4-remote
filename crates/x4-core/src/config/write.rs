//! Putting the settings of the Connection dialog into the text of `config.toml`.

use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use super::ConfigError;
use crate::connection::ConnectionSettings;
use crate::toml_error::locate_edit;

/// `text`, the contents of `config.toml`, with `settings` written into its `[connection]` table:
/// `url`, and `username` and `password` when there are credentials (they are removed when there
/// are none).
///
/// Everything else stays as it is: the other keys and tables, the comments, the order and the
/// layout. A `connection` that is not a table (`connection = "x"`) is replaced by one, and a key
/// of the wrong type (`url = 5`) is overwritten. An empty text gives a fresh file. Nothing is read
/// from or written to a file here.
///
/// # Errors
///
/// [`ConfigError::Parse`] if `text` is not valid TOML. The caller decides what happens to such a
/// file, for instance moving it aside.
pub fn config_with_connection(
    text: &str,
    settings: &ConnectionSettings,
) -> Result<String, ConfigError> {
    let mut document: DocumentMut = text.parse().map_err(|error| {
        let (position, message) = locate_edit(&error, text);
        ConfigError::Parse { position, message }
    })?;
    let root = document.as_table_mut();
    // An inline table or dotted keys are tables too, and their other keys are the owner's.
    if let Some(connection) = root.get_mut("connection").and_then(Item::as_table_like_mut) {
        fill(connection, settings);
    } else {
        let mut table = Table::new();
        // The comment lines above the old `connection = "x"` (a header comment of the file, say)
        // are not the old value's to take along.
        if let Some(prefix) = root
            .key("connection")
            .and_then(|key| key.leaf_decor().prefix().cloned())
        {
            table.decor_mut().set_prefix(prefix);
        }
        fill(&mut table, settings);
        root.insert("connection", Item::Table(table));
    }
    Ok(document.to_string())
}

/// Writes `settings` into the `[connection]` table: `url`, and the credentials or their absence.
fn fill(connection: &mut dyn TableLike, settings: &ConnectionSettings) {
    set_string(connection, "url", &settings.url());
    if let Some(credentials) = settings.credentials() {
        set_string(connection, "username", &credentials.username);
        set_string(connection, "password", &credentials.password);
    } else {
        connection.remove("username");
        connection.remove("password");
    }
}

/// Sets `key` to the string `text`. A key that is there keeps its place, its comments and the
/// comment after its value; whatever type it was.
fn set_string(table: &mut dyn TableLike, key: &str, text: &str) {
    let mut value = Value::from(text);
    match table.get_mut(key) {
        Some(item) => {
            // Replacing the value alone leaves the key's own decor alone: `insert` would reset it,
            // and with it the comment lines above the key.
            if let Some(old) = item.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}
