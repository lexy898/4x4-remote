//! The JSON-RPC 2.0 client: request envelope, `X-Transmission-Session-Id` handshake and response
//! decoding (rpc-spec §2 and §2.2.1).

use std::fmt::Display;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{RpcError, UnsupportedReason};
use crate::transport::{HttpRequest, HttpResponse, Transport};

/// The HTTP timeout per request; [`Client::torrent_add`] waits [`ADD_TIMEOUT`] instead.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long [`Client::torrent_add`] waits for its answer, in place of the transport's timeout.
///
/// An add sends the whole `.torrent` file: up to 16 MiB become about 21 MiB of base64, which
/// take several seconds over Wi-Fi to a Raspberry Pi, and the daemon decodes, parses, hashes and
/// stores them before it answers, with every other request waiting. Running out of this time
/// does not tell whether the torrent was added.
pub const ADD_TIMEOUT: Duration = Duration::from_secs(60);

const JSONRPC_VERSION: &str = "2.0";
const HTTP_OK: u16 = 200;
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_CONFLICT: u16 = 409;

/// A blocking Transmission RPC client over a [`Transport`].
///
/// Calls take `&mut self`: each one may replace the stored session id. The first call (and any
/// call after the daemon rotates its session id) costs one extra round trip: the server answers
/// 409 with a new `X-Transmission-Session-Id`, and the client resends the request once with it.
#[derive(Debug)]
pub struct Client<T: Transport> {
    transport: T,
    session_id: Option<String>,
    next_id: u64,
}

impl<T: Transport> Client<T> {
    /// A client with no session id yet; the first call performs the handshake.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            session_id: None,
            next_id: 1,
        }
    }

    /// Calls `method` with untyped `params` (omitted when `None`) and returns the untyped `result`
    /// (feature `test-support`).
    ///
    /// An untyped escape hatch for tooling such as the fixture recorder; app code must use the
    /// typed methods, which guarantee non-empty ids and never send empty file lists.
    ///
    /// # Errors
    ///
    /// Any [`RpcError`]: transport failure, HTTP status, a failed session-id handshake, an invalid
    /// response, a JSON-RPC error from the server, or [`RpcError::UnsupportedServer`] when the
    /// server answers in the legacy protocol of Transmission 4.0 and older.
    #[cfg(feature = "test-support")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "owned params let callers pass `Some(json!(...))` directly"
    )]
    pub fn call_raw(&mut self, method: &str, params: Option<Value>) -> Result<Value, RpcError> {
        self.call(method, params.as_ref())
    }

    /// Calls `method` with `params` (omitted when `None`) and decodes the `result` as `R`.
    ///
    /// # Errors
    ///
    /// As [`Client::call_raw`]; also [`RpcError::InvalidRequest`] when `params` cannot be encoded
    /// and [`RpcError::InvalidResponse`] when the `result` does not decode as `R`.
    pub(crate) fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Option<&P>,
    ) -> Result<R, RpcError> {
        self.call_with_timeout(method, params, None)
    }

    /// [`Client::call`] that may take `timeout` in place of the transport's own timeout, for
    /// each POST of it: the first, and the one resent after a session-id conflict.
    ///
    /// # Errors
    ///
    /// As [`Client::call`].
    pub(crate) fn call_with_timeout<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Option<&P>,
        timeout: Option<Duration>,
    ) -> Result<R, RpcError> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let request = RequestEnvelope {
            jsonrpc: JSONRPC_VERSION,
            method,
            params,
            id,
        };
        let body = serde_json::to_vec(&request)
            .map_err(|error| RpcError::InvalidRequest(format!("{method}: {error}")))?;
        let response = self.post_with_session(&body, timeout)?;
        match response.status {
            HTTP_OK => decode_response(method, id, &response.body),
            HTTP_UNAUTHORIZED => Err(RpcError::Unauthorized),
            HTTP_FORBIDDEN => Err(RpcError::Forbidden),
            status => Err(RpcError::HttpStatus(status)),
        }
    }

    /// POSTs `body`; on 409 adopts the session id the server sent and resends once.
    fn post_with_session(
        &mut self,
        body: &[u8],
        timeout: Option<Duration>,
    ) -> Result<HttpResponse, RpcError> {
        let response = self.post(body, timeout)?;
        if response.status != HTTP_CONFLICT {
            return Ok(response);
        }
        self.session_id = Some(response.session_id.ok_or(RpcError::SessionIdMissing)?);
        let retry = self.post(body, timeout)?;
        if retry.status == HTTP_CONFLICT {
            // Keep the newest id anyway, so the next call starts from it.
            if let Some(session_id) = retry.session_id {
                self.session_id = Some(session_id);
            }
            return Err(RpcError::SessionIdRejected);
        }
        Ok(retry)
    }

    fn post(&self, body: &[u8], timeout: Option<Duration>) -> Result<HttpResponse, RpcError> {
        self.transport
            .post(&HttpRequest {
                body,
                session_id: self.session_id.as_deref(),
                timeout,
            })
            .map_err(RpcError::Transport)
    }
}

#[derive(Serialize)]
struct RequestEnvelope<'a, P> {
    jsonrpc: &'static str,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a P>,
    id: u64,
}

#[derive(Deserialize)]
struct ErrorObject {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

/// Decodes the body of a 200 response to request `id`.
fn decode_response<R: DeserializeOwned>(method: &str, id: u64, body: &[u8]) -> Result<R, RpcError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| invalid(method, format_args!("not JSON: {error}")))?;
    let Value::Object(mut envelope) = value else {
        return Err(invalid(method, "not a JSON object"));
    };
    // Transmission 4.0 and older do not know JSON-RPC: they answer 200 with
    // `{"arguments":{},"result":"method name not recognized"}`. A string `result` and no
    // `jsonrpc` key is that shape and nothing else, unlike HTML or other garbage.
    if !envelope.contains_key("jsonrpc") && envelope.get("result").is_some_and(Value::is_string) {
        return Err(RpcError::UnsupportedServer {
            reason: UnsupportedReason::Legacy,
        });
    }
    if envelope.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Err(invalid(method, "not a JSON-RPC 2.0 response"));
    }
    let response_id = envelope.remove("id").unwrap_or(Value::Null);
    let id_matches = response_id.as_u64() == Some(id);
    match (envelope.remove("result"), envelope.remove("error")) {
        (Some(result), None) if id_matches => serde_json::from_value(result)
            .map_err(|error| invalid(method, format_args!("unexpected result: {error}"))),
        // JSON-RPC 2.0 answers with a null id when it could not read the request's id.
        (None, Some(error)) if id_matches || response_id.is_null() => {
            Err(server_error(method, error))
        }
        (None, None) => Err(invalid(method, "neither result nor error")),
        (Some(_), Some(_)) => Err(invalid(method, "both result and error")),
        _ => Err(invalid(
            method,
            format_args!("response id {response_id} does not match request id {id}"),
        )),
    }
}

fn server_error(method: &str, error: Value) -> RpcError {
    match serde_json::from_value::<ErrorObject>(error) {
        Ok(ErrorObject {
            code,
            message,
            data,
        }) => {
            let detail = data
                .as_ref()
                .and_then(|data| data.get("error_string"))
                .and_then(Value::as_str)
                .filter(|detail| !detail.is_empty())
                .map(str::to_owned);
            RpcError::Server {
                code,
                message,
                detail,
            }
        }
        Err(error) => invalid(method, format_args!("malformed error object: {error}")),
    }
}

fn invalid(method: &str, what: impl Display) -> RpcError {
    RpcError::InvalidResponse(format!("{method}: {what}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn decode(body: &Value) -> Result<Value, RpcError> {
        decode_response("session_get", 7, body.to_string().as_bytes())
    }

    #[test]
    fn error_with_null_id_is_still_a_server_error() {
        let body = json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32700, "message": "Parse error" }
        });

        assert_eq!(
            decode(&body),
            Err(RpcError::Server {
                code: -32700,
                message: "Parse error".to_owned(),
                detail: None,
            })
        );
    }

    #[test]
    fn error_with_another_id_is_invalid() {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 8,
            "error": { "code": 5, "message": "system error" }
        });

        assert!(matches!(decode(&body), Err(RpcError::InvalidResponse(_))));
    }

    #[test]
    fn empty_error_string_is_no_detail() {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "error": { "code": 5, "message": "system error", "data": { "error_string": "" } }
        });

        assert_eq!(
            decode(&body),
            Err(RpcError::Server {
                code: 5,
                message: "system error".to_owned(),
                detail: None,
            })
        );
    }

    #[test]
    fn invalid_response_names_the_method() {
        let error = decode(&json!({ "jsonrpc": "2.0", "id": 7 }));

        assert_eq!(
            error,
            Err(RpcError::InvalidResponse(
                "session_get: neither result nor error".to_owned()
            ))
        );
    }

    #[test]
    fn result_of_the_wrong_shape_is_invalid() {
        let result = decode_response::<String>(
            "session_get",
            7,
            br#"{"jsonrpc":"2.0","id":7,"result":{"version":"4.1.3"}}"#,
        );

        assert!(
            matches!(result, Err(RpcError::InvalidResponse(_))),
            "got {result:?}"
        );
    }
}
