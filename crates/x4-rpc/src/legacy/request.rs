//! The legacy request envelope, and the renaming of what a typed call asks for into legacy names.

use serde_json::{Map, Value};

use super::names::{NameContext, legacy_method, legacy_name};
use crate::error::RpcError;

/// The `ids` keyword for recently changed torrents, in JSON-RPC.
const RECENTLY_ACTIVE: &str = "recently_active";
/// The same keyword in the legacy protocol.
const LEGACY_RECENTLY_ACTIVE: &str = "recently-active";

/// The body `{"method", "arguments", "tag"}` of the legacy request for the JSON-RPC call `method`
/// with `params`; `arguments` is left out when `params` is `None`.
///
/// # Errors
///
/// [`RpcError::InvalidRequest`] when `params` is not an object, or holds a key or a field with
/// no legacy name.
pub(super) fn body(method: &str, params: Option<Value>, tag: u64) -> Result<Value, RpcError> {
    let mut envelope = Map::new();
    envelope.insert("method".to_owned(), Value::from(legacy_method(method)));
    if let Some(params) = params {
        envelope.insert("arguments".to_owned(), arguments(method, params)?);
    }
    envelope.insert("tag".to_owned(), Value::from(tag));
    Ok(Value::Object(envelope))
}

/// `params` with every key in its legacy name, and the values that are names too: the `fields`
/// (a torrent's for `torrent_get`, else the session's) and the `ids` keyword.
fn arguments(method: &str, params: Value) -> Result<Value, RpcError> {
    let Value::Object(params) = params else {
        return Err(RpcError::InvalidRequest(format!(
            "legacy: the arguments of {method} are not an object"
        )));
    };
    let fields_context = if method == "torrent_get" {
        NameContext::TorrentField
    } else {
        NameContext::Argument
    };
    params
        .into_iter()
        .map(|(key, value)| {
            let value = match key.as_str() {
                "fields" => fields(value, fields_context)?,
                "ids" if value == RECENTLY_ACTIVE => Value::from(LEGACY_RECENTLY_ACTIVE),
                _ => value,
            };
            Ok((renamed(&key, NameContext::Argument)?.to_owned(), value))
        })
        .collect()
}

/// The list of field names `fields`, each in its legacy name in `context`.
fn fields(fields: Value, context: NameContext) -> Result<Value, RpcError> {
    let Value::Array(fields) = fields else {
        return Err(RpcError::InvalidRequest(
            "legacy: fields is not a list".to_owned(),
        ));
    };
    fields
        .iter()
        .map(|field| {
            let name = field.as_str().ok_or_else(|| {
                RpcError::InvalidRequest(format!("legacy: the field {field} is not a name"))
            })?;
            renamed(name, context).map(Value::from)
        })
        .collect()
}

fn renamed(name: &str, context: NameContext) -> Result<&'static str, RpcError> {
    legacy_name(name, context)
        .ok_or_else(|| RpcError::InvalidRequest(format!("legacy: no legacy name for {name}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_envelope_holds_the_method_the_arguments_and_the_tag() {
        assert_eq!(
            body("torrent_start_now", Some(json!({ "ids": [1] })), 7),
            Ok(json!({ "method": "torrent-start-now", "arguments": { "ids": [1] }, "tag": 7 }))
        );
    }

    #[test]
    fn no_params_send_no_arguments() {
        assert_eq!(
            body("session_stats", None, 3),
            Ok(json!({ "method": "session-stats", "tag": 3 }))
        );
    }

    #[test]
    fn session_fields_are_named_as_arguments() {
        assert_eq!(
            body(
                "session_get",
                Some(json!({ "fields": ["version", "download_dir"] })),
                1
            ),
            Ok(json!({
                "method": "session-get",
                "arguments": { "fields": ["version", "download-dir"] },
                "tag": 1,
            }))
        );
    }

    #[test]
    fn torrent_fields_are_named_as_torrent_fields() {
        assert_eq!(
            body(
                "torrent_get",
                Some(json!({ "ids": "recently_active", "fields": ["download_dir", "file_count"] })),
                1
            ),
            Ok(json!({
                "method": "torrent-get",
                "arguments": {
                    "ids": "recently-active",
                    "fields": ["downloadDir", "file-count"],
                },
                "tag": 1,
            }))
        );
    }

    #[test]
    fn hashes_in_ids_stay_as_they_are() {
        let hash = "0123456789abcdef0123456789abcdef01234567";

        assert_eq!(
            body("torrent_verify", Some(json!({ "ids": [3, hash] })), 1),
            Ok(json!({ "method": "torrent-verify", "arguments": { "ids": [3, hash] }, "tag": 1 }))
        );
    }

    #[test]
    fn a_name_with_no_legacy_counterpart_is_refused() {
        assert_eq!(
            body("torrent_set", Some(json!({ "ids": [1], "labels": [] })), 1),
            Err(RpcError::InvalidRequest(
                "legacy: no legacy name for labels".to_owned()
            ))
        );
        assert_eq!(
            body(
                "torrent_get",
                Some(json!({ "fields": ["id", "bytes_completed"] })),
                1
            ),
            Err(RpcError::InvalidRequest(
                "legacy: no legacy name for bytes_completed".to_owned()
            ))
        );
    }

    #[test]
    fn arguments_that_are_not_an_object_are_refused() {
        assert!(matches!(
            body("session_get", Some(json!([1])), 1),
            Err(RpcError::InvalidRequest(_))
        ));
        assert!(matches!(
            body("session_get", Some(json!({ "fields": "version" })), 1),
            Err(RpcError::InvalidRequest(_))
        ));
    }
}
