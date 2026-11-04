//! Reading the daemon's records without letting one odd value fail the whole `torrent_get`.
//!
//! A response that does not decode is an [`RpcError::InvalidResponse`](crate::RpcError), which
//! the engine takes for an unusable server and answers by reconnecting; a record that fails the
//! same way on every read would reconnect in a loop. The records of peers and trackers carry text
//! and numbers that other parties control, so each of their keys is read on its own: a value of
//! an unexpected type or range holds the key's default and the rest of the record is still read.
//! The live tests compare what the daemon sent with what was read, so that a key the daemon
//! really names or types otherwise is noticed there.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// `deserialize_with` for a key whose value of another type, or out of range, holds the default.
///
/// The value is read whole first, so a misplaced object or list is skipped as a unit and the
/// keys after it are still read.
///
/// # Errors
///
/// Only the errors of the input itself, such as text that stops half way; never a value of the
/// wrong type.
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

/// `deserialize_with` for an optional list of records: a value that is not a list is `None`, and
/// an item of another type than the records is a default record.
///
/// # Errors
///
/// Only the errors of the input itself.
pub(crate) fn lenient_list<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(items) => Some(
            items
                .into_iter()
                .map(|item| T::deserialize(item).unwrap_or_default())
                .collect(),
        ),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::{lenient, lenient_list};

    #[derive(Debug, Default, PartialEq, Deserialize)]
    #[serde(default)]
    struct Probe {
        #[serde(deserialize_with = "lenient")]
        count: u16,
        #[serde(deserialize_with = "lenient")]
        text: String,
        #[serde(deserialize_with = "lenient")]
        maybe: Option<i64>,
        #[serde(deserialize_with = "lenient_list")]
        items: Option<Vec<u8>>,
    }

    fn probe(value: Value) -> Probe {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn values_of_the_right_type_are_kept() {
        let read = probe(json!({ "count": 7, "text": "a", "maybe": -1, "items": [1, 2] }));

        assert_eq!(
            read,
            Probe {
                count: 7,
                text: "a".to_owned(),
                maybe: Some(-1),
                items: Some(vec![1, 2]),
            }
        );
    }

    #[test]
    fn values_of_another_type_become_the_default() {
        let read = probe(json!({ "count": "x", "text": 5, "maybe": "x", "items": "x" }));

        assert_eq!(read, Probe::default());
    }

    #[test]
    fn numbers_out_of_range_become_the_default() {
        assert_eq!(probe(json!({ "count": 70_000 })).count, 0);
        assert_eq!(probe(json!({ "count": -1 })).count, 0);
        assert_eq!(probe(json!({ "count": 1.5 })).count, 0);
    }

    #[test]
    fn null_is_the_default_too() {
        let read = probe(json!({ "count": null, "text": null, "maybe": null, "items": null }));

        assert_eq!(read, Probe::default());
    }

    #[test]
    fn a_misplaced_nested_value_does_not_derail_the_keys_after_it() {
        // Read straight from text, as the client does: a reader that gave up half way through
        // the object would lose the text behind it.
        let text = r#"{"count": {"a": [1, 2, {"b": 3}]}, "text": "after", "items": [[1], 2]}"#;

        let read: Probe = serde_json::from_str(text).unwrap();

        assert_eq!(read.count, 0);
        assert_eq!(read.text, "after");
        assert_eq!(read.items, Some(vec![0, 2]));
    }

    #[test]
    fn list_items_of_another_type_become_the_default_each() {
        let read = probe(json!({ "items": [1, "x", null, 300, 4] }));

        assert_eq!(read.items, Some(vec![1, 0, 0, 0, 4]));
    }

    #[test]
    fn an_absent_key_is_the_default() {
        assert_eq!(probe(json!({})), Probe::default());
    }
}
