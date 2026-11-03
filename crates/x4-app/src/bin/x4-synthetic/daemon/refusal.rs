//! Why the daemon refuses a request: the JSON-RPC error it answers with, in the shape Transmission
//! 4.1 builds it (`JsonRpc::Error::build`, T/rpcimpl.cc): a code, the text of that code, and under
//! `data` an `error_string` and the `result` of the call, for those errors that have them.

use serde_json::{Map, Value};

use super::INVALID_PARAMS;

/// The code of a path that is not absolute (`PATH_NOT_ABSOLUTE`).
pub(super) const PATH_NOT_ABSOLUTE: i64 = 3;
/// The code of metainfo or a magnet link the daemon cannot read (`UNRECOGNIZED_INFO`).
pub(super) const UNRECOGNIZED_INFO: i64 = 4;
/// The code of a failed system call (`SYSTEM_ERROR`).
pub(super) const SYSTEM_ERROR: i64 = 5;
/// The code of a file index outside the torrent's files (`FILE_IDX_OOR`).
pub(super) const FILE_INDEX_OUT_OF_RANGE: i64 = 6;

/// Why the daemon refused a request: the JSON-RPC error it answers with.
#[derive(Debug)]
pub(super) struct Refusal {
    pub(super) code: i64,
    pub(super) message: String,
    /// The error's `data`: what Transmission puts there, when the call has something to say.
    pub(super) data: Option<Value>,
}

impl Refusal {
    /// Parameters the daemon cannot use, with `message` to say why.
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
            data: None,
        }
    }

    /// A refusal as Transmission words it: the text of `code`, then the `error_string` and the
    /// `result` of the call if there are any. `code` must be one of the codes this daemon uses.
    pub(super) fn daemon(code: i64, error_string: Option<&str>, result: Option<Value>) -> Self {
        let mut data = Map::new();
        if let Some(text) = error_string {
            data.insert("error_string".to_owned(), text.into());
        }
        if let Some(result) = result {
            data.insert("result".to_owned(), result);
        }
        Self {
            code,
            message: code_text(code).to_owned(),
            data: (!data.is_empty()).then_some(Value::Object(data)),
        }
    }

    /// The JSON-RPC `error` member.
    pub(super) fn to_json(&self) -> Value {
        let mut error = Map::new();
        error.insert("code".to_owned(), self.code.into());
        error.insert("message".to_owned(), self.message.clone().into());
        if let Some(data) = &self.data {
            error.insert("data".to_owned(), data.clone());
        }
        Value::Object(error)
    }
}

/// The text Transmission gives a code (`JsonRpc::Error::to_string`).
fn code_text(code: i64) -> &'static str {
    match code {
        INVALID_PARAMS => "Invalid params",
        PATH_NOT_ABSOLUTE => "path is not absolute",
        UNRECOGNIZED_INFO => "unrecognized info",
        SYSTEM_ERROR => "system error",
        FILE_INDEX_OUT_OF_RANGE => "file index out of range",
        _ => "",
    }
}
