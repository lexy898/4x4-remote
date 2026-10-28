//! How the state file is read and written beyond what `derive` does: a value or a column that a
//! newer version added must not make the whole file unreadable.

use std::any::type_name;

use serde::de::value::Error as ValueError;
use serde::de::{DeserializeOwned, IntoDeserializer};
use serde::{Deserialize, Deserializer, Serializer};

use super::{ColumnState, default_width};
use crate::view::ColumnId;

/// The longest piece of a value from the file that is logged.
const LOGGED_CHARS: usize = 32;

/// `value` cut to [`LOGGED_CHARS`] characters (with an ellipsis if it was longer), so that a huge
/// or odd entry in the file cannot flood the log.
pub(super) fn shortened(value: &str) -> String {
    let mut chars = value.chars();
    let mut shortened: String = chars.by_ref().take(LOGGED_CHARS).collect();
    if chars.next().is_some() {
        shortened.push('…');
    }
    shortened
}

/// Reads an enum from its name, and takes the default for a name the enum does not have.
///
/// A file from a newer version may name a filter, a sort key or a theme this one lacks. Losing
/// that one setting is better than refusing to start. A value that is not a string at all is
/// still an error, like any other wrong type.
pub(super) fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let name = String::deserialize(deserializer)?;
    // The error type of the string deserializer does not matter, only whether the name is known.
    let parsed: Result<T, ValueError> = T::deserialize(name.as_str().into_deserializer());
    Ok(parsed.unwrap_or_else(|_| {
        tracing::warn!(
            value = %shortened(&name),
            setting = type_name::<T>(),
            "unknown value in the state file, using the default"
        );
        T::default()
    }))
}

/// A column as the file has it, before it is checked against the columns this version knows.
#[derive(Deserialize)]
struct RawColumn {
    id: Option<String>,
    visible: Option<bool>,
    width: Option<f32>,
}

impl RawColumn {
    /// The column, or `None` for an entry that names no column this version has.
    fn into_state(self) -> Option<ColumnState> {
        let Some(key) = self.id else {
            tracing::warn!("a column in the state file has no id, dropped");
            return None;
        };
        let Some(id) = ColumnId::from_key(&key) else {
            tracing::warn!(column = %shortened(&key), "unknown column in the state file, dropped");
            return None;
        };
        Some(ColumnState {
            id,
            visible: self.visible.unwrap_or(true),
            width: self.width.unwrap_or_else(|| default_width(id)),
        })
    }
}

/// Reads the list of columns and drops the entries no column of this version matches. A missing
/// `visible` or `width` takes the default of that column.
pub(super) fn columns<'de, D>(deserializer: D) -> Result<Vec<ColumnState>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Vec::<RawColumn>::deserialize(deserializer)?;
    Ok(raw.into_iter().filter_map(RawColumn::into_state).collect())
}

/// Writes a column by its stable key.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `serialize_with` hands the field over by reference"
)]
pub(super) fn column_id<S>(id: &ColumnId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(id.key())
}
