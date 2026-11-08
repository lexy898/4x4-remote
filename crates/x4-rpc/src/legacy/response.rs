//! The legacy answer envelope: its tag, its errors, and the renaming of its keys back into
//! JSON-RPC's names.

use std::fmt::Display;

use serde_json::{Map, Value};

use super::names::snake_name;
use crate::error::{LEGACY_ERROR_CODE, RpcError};

/// The `result` of a legacy answer that is no error.
const SUCCESS: &str = "success";

/// The method whose errors take the shape Transmission 4.1 gives them.
const FREE_SPACE: &str = "free_space";

/// The code and message of a failed `free_space` in JSON-RPC (Transmission 4.1), which also
/// carries the system's text as its detail. A legacy daemon sends only that text; giving it the
/// same shape lets a caller tell a folder that is not there the same way for both protocols.
const SYSTEM_ERROR_CODE: i64 = 5;
const SYSTEM_ERROR_MESSAGE: &str = "system error";

/// The text of a file index past a torrent's last file, the same in the legacy daemons and in
/// Transmission 4.1, and the code 4.1 gives it (`FILE_IDX_OOR`), so that a caller tells it the
/// same way for both protocols.
const FILE_INDEX_OUT_OF_RANGE: &str = "file index out of range";
const FILE_INDEX_OUT_OF_RANGE_CODE: i64 = 6;

/// The `arguments` of the answer `body` to the legacy request for `method` tagged `tag`, with
/// every key renamed to its JSON-RPC name (a key 4x4 does not know stays as it is); an answer
/// with no `arguments` has an empty object.
///
/// # Errors
///
/// [`RpcError::InvalidResponse`] when `body` is not a JSON object with a string `result`, or
/// carries another tag; [`RpcError::Server`] when `result` is not `"success"`, and for a
/// `free_space` whose free space is negative: Transmission 4.1.0-beta answers so for a folder
/// it cannot read, where the others report an error.
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
    let arguments = snake_keys(
        envelope
            .remove("arguments")
            .unwrap_or_else(|| Value::Object(Map::new())),
    );
    let no_free_space = arguments
        .get("size_bytes")
        .and_then(Value::as_i64)
        .is_some_and(|size| size < 0);
    if method == FREE_SPACE && no_free_space {
        return Err(system_error(None));
    }
    Ok(arguments)
}

/// The error of a legacy answer whose `result` is `text`.
fn server_error(method: &str, text: String) -> RpcError {
    if method == FREE_SPACE {
        return system_error(Some(text));
    }
    let code = if text == FILE_INDEX_OUT_OF_RANGE {
        FILE_INDEX_OUT_OF_RANGE_CODE
    } else {
        LEGACY_ERROR_CODE
    };
    RpcError::Server {
        code,
        message: text,
        detail: None,
    }
}

/// The error of a failed `free_space`, in the shape Transmission 4.1 gives it.
fn system_error(detail: Option<String>) -> RpcError {
    RpcError::Server {
        code: SYSTEM_ERROR_CODE,
        message: SYSTEM_ERROR_MESSAGE.to_owned(),
        detail,
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
    fn an_out_of_range_file_index_gets_the_code_4_1_gives_it() {
        // What Transmission 3.00 to 4.1.0-beta answer for a file index past the last file.
        let body = json!({ "result": "file index out of range", "arguments": {}, "tag": 7 });

        assert_eq!(
            read("torrent_set", &body),
            Err(RpcError::Server {
                code: 6,
                message: "file index out of range".to_owned(),
                detail: None,
            })
        );
    }

    #[test]
    fn a_free_space_without_room_is_read_and_a_negative_one_is_a_system_error() {
        let answer = |size: i64| {
            read(
                "free_space",
                &json!({ "result": "success", "arguments": { "path": "/p", "size-bytes": size } }),
            )
        };

        assert_eq!(answer(0), Ok(json!({ "path": "/p", "size_bytes": 0 })));
        // What Transmission 4.1.0-beta answers for a folder that is not there.
        assert_eq!(
            answer(-1),
            Err(RpcError::Server {
                code: SYSTEM_ERROR_CODE,
                message: "system error".to_owned(),
                detail: None,
            })
        );
    }

    #[test]
    fn a_negative_size_bytes_means_nothing_to_another_method() {
        let body = json!({ "result": "success", "arguments": { "size-bytes": -1 } });

        assert_eq!(read("session_get", &body), Ok(json!({ "size_bytes": -1 })));
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
