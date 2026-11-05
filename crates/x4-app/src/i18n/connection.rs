//! The texts of the Connection dialog that carry values or word x4-core's decisions: the address
//! the fields make, what a test of the server found, why the settings could not be saved, and what
//! is wrong with each field.
//!
//! A failed test says the reason in the words of the status bar's tooltip
//! ([`connection_reason_detail_text`]), and a daemon 4x4 cannot use is named as the status bar
//! names it, so the two never disagree about the same server.

use gpui_kit::SharedString;
use x4_core::{CheckOutcome, HostProblem, PathProblem, PortProblem, UsernameProblem};

use super::{connection_reason_detail_text, t, unsupported_string};

/// The line under the fields: the address they make, such as "Address:
/// `http://nas:9091/transmission/rpc`". `url` is the settings'
/// [`url`](x4_core::ConnectionSettings::url), which never holds the credentials.
pub(crate) fn connection_address_text(url: &str) -> SharedString {
    t!("connection-address", url = url).into()
}

/// What a test of the server found: the daemon's version, or why 4x4 cannot use it.
pub(crate) fn connection_test_text(outcome: &CheckOutcome) -> SharedString {
    match outcome {
        CheckOutcome::Connected { version } => {
            t!("connection-test-ok", version = version.as_str()).into()
        }
        CheckOutcome::Failed(error) => connection_reason_detail_text(error),
        CheckOutcome::Unsupported(reason) => unsupported_string(reason).into(),
        CheckOutcome::NotRun => t!("connection-test-not-run").into(),
    }
}

/// What the dialog says when Save could not write the settings, for `reason`, the system's words.
pub(crate) fn connection_save_failed_text(reason: &str) -> SharedString {
    t!("connection-save-failed", reason = reason).into()
}

/// What is wrong with the host field.
pub(crate) fn host_problem_text(problem: HostProblem) -> SharedString {
    match problem {
        HostProblem::Empty => t!("connection-host-empty"),
        HostProblem::Invalid => t!("connection-host-invalid"),
        HostProblem::Https => t!("connection-host-https"),
    }
    .into()
}

/// What is wrong with the port field. A port that is not a number and one out of range get the
/// same words: both are told what a port is.
pub(crate) fn port_problem_text(problem: PortProblem) -> SharedString {
    match problem {
        PortProblem::Empty => t!("connection-port-empty"),
        PortProblem::NotANumber | PortProblem::OutOfRange => t!("connection-port-invalid"),
    }
    .into()
}

/// What is wrong with the path field.
pub(crate) fn path_problem_text(problem: PathProblem) -> SharedString {
    match problem {
        PathProblem::Empty => t!("connection-path-empty"),
        PathProblem::Invalid => t!("connection-path-invalid"),
    }
    .into()
}

/// What is wrong with the user name field.
pub(crate) fn username_problem_text(problem: UsernameProblem) -> SharedString {
    match problem {
        UsernameProblem::Colon => t!("connection-username-colon"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use x4_core::{ConnectionError, UnsupportedReason};

    use super::*;

    #[test]
    fn the_address_line_names_the_address() {
        assert_eq!(
            connection_address_text("http://nas:9091/transmission/rpc"),
            "Address: http://nas:9091/transmission/rpc"
        );
        assert_eq!(
            connection_address_text("http://[::1]:9091/rpc"),
            "Address: http://[::1]:9091/rpc"
        );
    }

    #[test]
    fn a_test_says_the_version_or_why_the_server_cannot_be_used() {
        assert_eq!(
            connection_test_text(&CheckOutcome::Connected {
                version: "4.1.0 (abc123)".to_owned()
            }),
            "Connected: Transmission 4.1.0 (abc123)"
        );
        assert_eq!(
            connection_test_text(&CheckOutcome::Failed(ConnectionError::Unauthorized)),
            "The server rejected the credentials"
        );
        assert_eq!(
            connection_test_text(&CheckOutcome::Failed(ConnectionError::Unreachable)),
            "The server cannot be reached"
        );
        assert_eq!(
            connection_test_text(&CheckOutcome::Failed(ConnectionError::Server(
                "the server's own words".to_owned()
            ))),
            "The server reported an error",
            "as the status bar's tooltip, without the server's details"
        );
        assert_eq!(
            connection_test_text(&CheckOutcome::Unsupported(UnsupportedReason::Legacy)),
            "Transmission 4.1 or newer required"
        );
        assert_eq!(
            connection_test_text(&CheckOutcome::Unsupported(UnsupportedReason::NoVersion)),
            "Transmission 4.1 or newer required (server did not report a version)"
        );
        assert_eq!(
            connection_test_text(&CheckOutcome::Unsupported(UnsupportedReason::Version(
                "5.0.1".to_owned()
            ))),
            "Transmission 4.1 or newer required (server RPC 5.0.1)"
        );
        assert_eq!(
            connection_test_text(&CheckOutcome::NotRun),
            "The test could not run"
        );
    }

    #[test]
    fn a_failed_save_says_why() {
        assert_eq!(
            connection_save_failed_text("Permission denied (os error 13)"),
            "The settings could not be saved: Permission denied (os error 13)"
        );
    }

    #[test]
    fn each_problem_of_a_field_has_its_words() {
        assert_eq!(
            host_problem_text(HostProblem::Empty),
            "Enter the server's name or IP address"
        );
        assert_eq!(
            host_problem_text(HostProblem::Invalid),
            "Not a valid name or IP address"
        );
        assert_eq!(
            host_problem_text(HostProblem::Https),
            "HTTPS is not supported yet"
        );
        assert_eq!(port_problem_text(PortProblem::Empty), "Enter the port");
        for problem in [PortProblem::NotANumber, PortProblem::OutOfRange] {
            assert_eq!(
                port_problem_text(problem),
                "The port is a number from 1 to 65535",
                "{problem:?}"
            );
        }
        assert_eq!(
            path_problem_text(PathProblem::Empty),
            "Enter the path, usually /transmission/rpc"
        );
        assert_eq!(
            path_problem_text(PathProblem::Invalid),
            "The path cannot hold ?, # or spaces"
        );
        assert_eq!(
            username_problem_text(UsernameProblem::Colon),
            "A user name cannot contain a colon"
        );
    }
}
