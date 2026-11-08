//! The legacy answer envelope: its tag, its errors, and the renaming of its keys back into
//! JSON-RPC's names.

use std::fmt::Display;

use serde_json::{Map, Value};

use super::names::snake_name;
use crate::error::{LEGACY_ERROR_CODE, RpcError};

/// The `result` of a legacy answer that is no error.
const SUCCESS: &str = "success";

/// The code and message of a failed `free_space` in JSON-RPC (Transmission 4.1), which also
/// carries the system's text as its detail. A legacy daemon sends only that text; giving it the
/// same shape lets a caller tell a folder that is not there the same way for both protocols.
const SYSTEM_ERROR_CODE: i64 = 5;
const SYSTEM_ERROR_MESSAGE: &str = "system error";

/// The `arguments` of the answer `body` to the legacy request for `method` tagged `tag`, with
/// every key renamed to its JSON-RPC name (a key 4x4 does not know stays as it is); an answer
/// with no `arguments` has an empty object.
///
/// # Errors
///
/// [`RpcError::InvalidResponse`] when `body` is not a JSON object with a string `result`, or
/// carries another tag; [`RpcError::Server`] when `result` is not `"success"`.
pub(super) fn arguments(method: &str, tag: u64, body: &[u8]) -> Result<Value, RpcError> {
    let Ok(Value::Object(mut envelope)) = serde_json::from_slice::<Value>(body) else {
        return Err(not_legacy(method));
    };
    let Some(Value::String(result)) = envelope.remove("result") else {
        return Err(not_legacy(method));
    };
    if let Some(answer_tag) = envelope.get("tag")
        && answer_tag.as_u64() != Some(tag)
    {
        return Err(invalid(
            method,
            format_args!("response tag {answer_tag} does not match request tag {tag}"),
        ));
    }
    if result != SUCCESS {
        return Err(server_error(method, result));
    }
    let arguments = envelope
        .remove("arguments")
        .unwrap_or_else(|| Value::Object(Map::new()));
    Ok(snake_keys(arguments))
}

/// The error of a legacy answer whose `result` is `text`.
fn server_error(method: &str, text: String) -> RpcError {
    if method == "free_space" {
        RpcError::Server {
            code: SYSTEM_ERROR_CODE,
            message: SYSTEM_ERROR_MESSAGE.to_owned(),
            detail: Some(text),
        }
    } else {
        RpcError::Server {
            code: LEGACY_ERROR_CODE,
            message: text,
            detail: None,
        }
    }
}

/// `value` with the keys of every object in it, however deep, in their JSON-RPC names.
fn snake_keys(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    let key = snake_name(&key).map_or(key, str::to_owned);
                    (key, snake_keys(value))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(snake_keys).collect()),
        other => other,
    }
}

fn not_legacy(method: &str) -> RpcError {
    invalid(method, "not a legacy Transmission response")
}

fn invalid(method: &str, what: impl Display) -> RpcError {
    RpcError::InvalidResponse(format!("{method}: {what}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read(method: &str, body: &Value) -> Result<Value, RpcError> {
        arguments(method, 7, body.to_string().as_bytes())
    }

    #[test]
    fn keys_are_renamed_at_every_depth_and_values_stay() {
        let body = json!({
            "result": "success",
            "tag": 7,
            "arguments": {
                "torrents": [{
                    "hashString": "isUTP",
                    "peers": [{ "isUTP": true, "clientName": "x" }],
                    "files": [{ "bytesCompleted": 1, "begin_piece": 0 }],
                }],
            },
        });

        assert_eq!(
            read("torrent_get", &body),
            Ok(json!({
                "torrents": [{
                    "hash_string": "isUTP",
                    "peers": [{ "is_utp": true, "client_name": "x" }],
                    "files": [{ "bytes_completed": 1, "begin_piece": 0 }],
                }],
            }))
        );
    }

    #[test]
    fn an_answer_without_a_tag_or_arguments_is_read() {
        assert_eq!(
            read("torrent_start", &json!({ "result": "success" })),
            Ok(json!({}))
        );
    }

    #[test]
    fn another_tag_is_invalid() {
        let result = read(
            "session_stats",
            &json!({ "result": "success", "arguments": {}, "tag": 8 }),
        );

        assert_eq!(
            result,
            Err(RpcError::InvalidResponse(
                "session_stats: response tag 8 does not match request tag 7".to_owned()
            ))
        );
    }

    #[test]
    fn what_is_no_legacy_answer_is_invalid() {
        let not_legacy = Err(RpcError::InvalidResponse(
            "session_get: not a legacy Transmission response".to_owned(),
        ));

        assert_eq!(arguments("session_get", 7, b"<html>"), not_legacy);
        assert_eq!(read("session_get", &json!([1])), not_legacy);
        assert_eq!(
            read(
                "session_get",
                &json!({ "jsonrpc": "2.0", "id": 7, "result": {} })
            ),
            not_legacy
        );
        assert_eq!(read("session_get", &json!({ "arguments": {} })), not_legacy);
    }

    #[test]
    fn an_error_carries_the_daemon_text() {
        assert_eq!(
            read(
                "torrent_add",
                &json!({ "result": "invalid or corrupt torrent file", "arguments": {} })
            ),
            Err(RpcError::Server {
                code: LEGACY_ERROR_CODE,
                message: "invalid or corrupt torrent file".to_owned(),
                detail: None,
            })
        );
    }

    #[test]
    fn a_failed_free_space_is_a_system_error() {
        // What Transmission 3.00 answers for a folder that is not there.
        let body = json!({
            "result": "No such file or directory",
            "arguments": { "path": "/nowhere", "size-bytes": -1 },
            "tag": 7,
        });

        assert_eq!(
            read("free_space", &body),
            Err(RpcError::Server {
                code: SYSTEM_ERROR_CODE,
                message: "system error".to_owned(),
                detail: Some("No such file or directory".to_owned()),
            })
        );
    }
}
