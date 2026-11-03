//! The HTTP layer under the JSON-RPC client: a [`Transport`] trait and its `ureq` implementation.
//!
//! A transport only moves bytes and the `X-Transmission-Session-Id` header; status codes and the
//! session-id handshake are interpreted by [`Client`](crate::Client).

use std::io;
use std::time::Duration;

use ureq::config::Config;
use ureq::http::Uri;
// ureq's `unversioned` API may change in a minor release; a break shows up as a build error
// on `just update`.
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// The CSRF header Transmission requires on every request (rpc-spec §2.2.1).
const SESSION_ID_HEADER: &str = "X-Transmission-Session-Id";

/// One JSON-RPC POST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpRequest<'a> {
    /// The encoded JSON-RPC request.
    pub body: &'a [u8],
    /// The `X-Transmission-Session-Id` to send, if one is known.
    pub session_id: Option<&'a str>,
    /// How long this request may take in place of the transport's own timeout, from connecting
    /// to the last byte of the response; `None` keeps the transport's.
    pub timeout: Option<Duration>,
}

/// The server's answer to an [`HttpRequest`], whatever its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The `X-Transmission-Session-Id` response header, if present.
    pub session_id: Option<String>,
    /// The response body.
    pub body: Vec<u8>,
}

/// A request that produced no HTTP response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The request did not complete within the configured timeout.
    #[error("the request timed out")]
    Timeout,
    /// No connection to the server: refused, unreachable, unknown host or invalid URL.
    #[error("cannot connect to the server: {0}")]
    Connect(String),
    /// The connection failed while sending the request or reading the response.
    #[error("I/O error talking to the server: {0}")]
    Io(String),
}

/// Sends JSON-RPC requests to one Transmission RPC endpoint.
///
/// `Send` so a client can live on a worker thread.
pub trait Transport: Send {
    /// POSTs `request` and returns the response for any HTTP status.
    ///
    /// # Errors
    ///
    /// [`TransportError`] when no HTTP response was received.
    fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, TransportError>;
}

/// A blocking HTTP transport over `ureq`, plain HTTP only.
///
/// It always connects directly to the daemon: the proxy environment variables (`HTTP_PROXY`,
/// `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`) are ignored. `NO_PROXY` cannot exempt a LAN range, so
/// honouring them would send requests for a LAN daemon to the proxy. A proxy option is future work.
#[derive(Debug, Clone)]
pub struct UreqTransport {
    agent: ureq::Agent,
    url: String,
}

impl UreqTransport {
    /// A transport for the RPC endpoint `url` (for example
    /// `http://host:9091/transmission/rpc`) whose requests fail after `timeout`.
    ///
    /// The URL is checked on the first request: an invalid one fails every request with
    /// [`TransportError::Connect`].
    #[must_use]
    pub fn new(url: &str, timeout: Duration) -> Self {
        Self::with_resolver(url, timeout, DefaultResolver::default())
    }

    /// [`UreqTransport::new`] with a bespoke name resolver, so tests can fail lookups offline.
    fn with_resolver(url: &str, timeout: Duration, resolver: impl Resolver) -> Self {
        let config = ureq::Agent::config_builder()
            // 401, 403 and 409 are part of the protocol; the client interprets every status.
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .proxy(None)
            .build();
        let agent =
            ureq::Agent::with_parts(config, DefaultConnector::new(), LookupResolver(resolver));
        Self {
            agent,
            url: url.to_owned(),
        }
    }
}

/// Marks name-resolution failures, which std reports as uncategorised I/O errors that would
/// otherwise look like a failure on an open connection.
#[derive(Debug)]
struct LookupResolver<R>(R);

impl<R: Resolver> Resolver for LookupResolver<R> {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        self.0
            .resolve(uri, config, timeout)
            .map_err(|error| match error {
                ureq::Error::Io(io) => ureq::Error::Other(Box::new(HostLookupError(io))),
                other => other,
            })
    }
}

/// The host name of the RPC URL could not be resolved.
#[derive(Debug, thiserror::Error)]
#[error("cannot resolve the host name: {0}")]
struct HostLookupError(io::Error);

impl Transport for UreqTransport {
    fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, TransportError> {
        let uri = Uri::try_from(self.url.as_str()).map_err(|error| {
            TransportError::Connect(format!("invalid URL {:?}: {error}", self.url))
        })?;
        let mut builder = self.agent.post(uri).content_type("application/json");
        if let Some(session_id) = request.session_id {
            builder = builder.header(SESSION_ID_HEADER, session_id);
        }
        if let Some(timeout) = request.timeout {
            // A copy of the agent's configuration for this request only.
            builder = builder.config().timeout_global(Some(timeout)).build();
        }
        let mut response = builder.send(request.body).map_err(transport_error)?;
        let session_id = response
            .headers()
            .get(SESSION_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_vec().map_err(transport_error)?;
        Ok(HttpResponse {
            status,
            session_id,
            body,
        })
    }
}

fn transport_error(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        ureq::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut => TransportError::Timeout,
        ureq::Error::Io(io) if is_connect_error(io.kind()) => {
            TransportError::Connect(io.to_string())
        }
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::BadUri(_) => {
            TransportError::Connect(error.to_string())
        }
        ureq::Error::Other(inner) if inner.is::<HostLookupError>() => {
            TransportError::Connect(inner.to_string())
        }
        other => TransportError::Io(other.to_string()),
    }
}

/// I/O errors that mean the server could not be reached at all.
fn is_connect_error(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::NetworkDown
            | io::ErrorKind::AddrNotAvailable
    )
}

#[cfg(test)]
mod tests {
    //! Loopback-only: a one-shot HTTP server on 127.0.0.1 stands in for the daemon.

    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread::{self, JoinHandle};

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Accepts one connection, reads one request and answers with `response`.
    /// Joining returns the raw request (head and body).
    fn serve_once(response: &'static str) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transmission/rpc", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (url, server)
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut raw = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            assert_ne!(read, 0, "client closed before sending a full request");
            raw.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&raw).into_owned();
            if let Some(head_end) = text.find("\r\n\r\n") {
                let length = text[..head_end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if raw.len() >= head_end + 4 + length {
                    return text;
                }
            }
        }
    }

    #[test]
    fn posts_json_with_the_session_id_and_reads_a_conflict() {
        let (url, server) = serve_once(
            "HTTP/1.1 409 Conflict\r\nX-Transmission-Session-Id: fresh\r\n\
             Content-Length: 22\r\nConnection: close\r\n\r\n<h1>409: Conflict</h1>",
        );
        let transport = UreqTransport::new(&url, TIMEOUT);

        let response = transport.post(&HttpRequest {
            body: br#"{"jsonrpc":"2.0","method":"session_stats","id":1}"#,
            session_id: Some("stale"),
            timeout: None,
        });
        let request = server.join().unwrap().to_ascii_lowercase();

        assert_eq!(
            response,
            Ok(HttpResponse {
                status: 409,
                session_id: Some("fresh".to_owned()),
                body: b"<h1>409: Conflict</h1>".to_vec(),
            })
        );
        assert!(request.starts_with("post /transmission/rpc "), "{request}");
        assert!(
            request.contains("\r\ncontent-type: application/json\r\n"),
            "{request}"
        );
        assert!(
            request.contains("\r\nx-transmission-session-id: stale\r\n"),
            "{request}"
        );
        assert!(request.ends_with(r#"{"jsonrpc":"2.0","method":"session_stats","id":1}"#));
    }

    #[test]
    fn sends_no_session_id_until_one_is_known() {
        let (url, server) =
            serve_once("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        let transport = UreqTransport::new(&url, TIMEOUT);

        let response = transport.post(&HttpRequest {
            body: b"{}",
            session_id: None,
            timeout: None,
        });
        let request = server.join().unwrap().to_ascii_lowercase();

        assert_eq!(
            response,
            Ok(HttpResponse {
                status: 200,
                session_id: None,
                body: b"{}".to_vec(),
            })
        );
        assert!(!request.contains("x-transmission-session-id"), "{request}");
    }

    #[test]
    fn a_silent_server_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transmission/rpc", listener.local_addr().unwrap());
        // Accept and hold the connection without ever answering.
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut sink = Vec::new();
            // Returns once the client gives up and closes the connection.
            let _read = stream.read_to_end(&mut sink);
        });
        let transport = UreqTransport::new(&url, Duration::from_millis(200));

        let response = transport.post(&HttpRequest {
            body: b"{}",
            session_id: None,
            timeout: None,
        });

        assert_eq!(response, Err(TransportError::Timeout));
        drop(transport);
        server.join().unwrap();
    }

    #[test]
    fn a_request_can_wait_longer_than_the_transport_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transmission/rpc", listener.local_addr().unwrap());
        // Answers well after the transport's own timeout.
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            thread::sleep(Duration::from_millis(300));
            // A client that gave up has closed the connection; the response below tells.
            let _written = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        });
        let transport = UreqTransport::new(&url, Duration::from_millis(100));

        let response = transport.post(&HttpRequest {
            body: b"{}",
            session_id: None,
            timeout: Some(Duration::from_secs(5)),
        });

        assert_eq!(response.map(|response| response.status), Ok(200));
        server.join().unwrap();
    }

    #[test]
    fn a_closed_port_is_a_connect_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transmission/rpc", listener.local_addr().unwrap());
        drop(listener);
        let transport = UreqTransport::new(&url, TIMEOUT);

        let response = transport.post(&HttpRequest {
            body: b"{}",
            session_id: None,
            timeout: None,
        });

        assert!(
            matches!(response, Err(TransportError::Connect(_))),
            "got {response:?}"
        );
    }

    #[test]
    fn an_invalid_url_is_a_connect_error() {
        let transport = UreqTransport::new("not a url", TIMEOUT);

        let response = transport.post(&HttpRequest {
            body: b"{}",
            session_id: None,
            timeout: None,
        });

        assert!(
            matches!(response, Err(TransportError::Connect(_))),
            "got {response:?}"
        );
    }

    #[test]
    fn proxy_environment_variables_are_ignored() {
        // Environment variables cannot be set here (`set_var` is unsafe), so check the agent's
        // configuration: ureq's default would pick up HTTP_PROXY and friends.
        let transport = UreqTransport::new("http://127.0.0.1:9091/transmission/rpc", TIMEOUT);

        assert!(
            transport.agent.config().proxy().is_none(),
            "{:?}",
            transport.agent.config().proxy()
        );
    }

    /// Fails every lookup the way std does: an uncategorised I/O error.
    #[derive(Debug)]
    struct FailingLookup;

    impl Resolver for FailingLookup {
        fn resolve(
            &self,
            _uri: &Uri,
            _config: &Config,
            _timeout: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, ureq::Error> {
            Err(ureq::Error::Io(io::Error::other(
                "failed to lookup address information: Temporary failure in name resolution",
            )))
        }
    }

    // The real resolver is not tested: a lookup through it may send a DNS query, and default
    // tests never touch the network. The mapping of a failed lookup is the same for any resolver.
    #[test]
    fn a_failed_lookup_is_a_connect_error() {
        let transport = UreqTransport::with_resolver(
            "http://daemon.example:9091/transmission/rpc",
            TIMEOUT,
            FailingLookup,
        );

        let response = transport.post(&HttpRequest {
            body: b"{}",
            session_id: None,
            timeout: None,
        });

        assert_eq!(
            response,
            Err(TransportError::Connect(
                "cannot resolve the host name: failed to lookup address information: \
                 Temporary failure in name resolution"
                    .to_owned()
            ))
        );
    }
}
