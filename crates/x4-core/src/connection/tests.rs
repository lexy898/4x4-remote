use super::*;

fn credentials(username: &str, password: &str) -> Credentials {
    Credentials {
        username: username.to_owned(),
        password: password.to_owned(),
    }
}

fn settings(url: &str) -> ConnectionSettings {
    ConnectionSettings::from_url(url, None).unwrap_or_else(|problem| panic!("{url:?}: {problem}"))
}

#[test]
fn url_round_trips() {
    // (what is written, host, port, path, the URL 4x4 writes back)
    let rows = [
        (
            "http://pi.local:9091/transmission/rpc",
            "pi.local",
            9091,
            "/transmission/rpc",
            "http://pi.local:9091/transmission/rpc",
        ),
        // The scheme and the host are case-insensitive; no port means 80, which 4x4 spells out.
        (
            "HTTP://Pi.Local/rpc",
            "pi.local",
            80,
            "/rpc",
            "http://pi.local:80/rpc",
        ),
        (
            "http://[::1]:9091/x",
            "::1",
            9091,
            "/x",
            "http://[::1]:9091/x",
        ),
        (
            "http://[FE80::1]/",
            "fe80::1",
            80,
            "/",
            "http://[fe80::1]:80/",
        ),
        // No path at all is the root path.
        (
            "http://192.168.1.5:9091",
            "192.168.1.5",
            9091,
            "/",
            "http://192.168.1.5:9091/",
        ),
        // The `url` crate's own readings of an IPv4 address and of dot segments.
        (
            "http://127.1:9091/",
            "127.0.0.1",
            9091,
            "/",
            "http://127.0.0.1:9091/",
        ),
        (
            "http://x:9091/a/../b/./c",
            "x",
            9091,
            "/b/c",
            "http://x:9091/b/c",
        ),
        // Spaces around the text are not part of it.
        (
            " http://x:9091/rpc\n",
            "x",
            9091,
            "/rpc",
            "http://x:9091/rpc",
        ),
        // A path is kept percent-encoded, and encoding it twice changes nothing.
        (
            "http://x:9091/ä%20b",
            "x",
            9091,
            "/%C3%A4%20b",
            "http://x:9091/%C3%A4%20b",
        ),
    ];

    for (input, host, port, path, url) in rows {
        let parsed = settings(input);

        assert_eq!(parsed.host(), host, "{input:?}");
        assert_eq!(parsed.port(), port, "{input:?}");
        assert_eq!(parsed.path(), path, "{input:?}");
        assert_eq!(parsed.credentials(), None, "{input:?}");
        assert_eq!(parsed.url(), url, "{input:?}");
        assert_eq!(
            ConnectionSettings::from_url(&parsed.url(), None),
            Ok(parsed),
            "{input:?}"
        );
    }
}

#[test]
fn from_url_rejects() {
    let rows = [
        ("https://x/rpc", UrlProblem::Https),
        ("HTTPS://x/rpc", UrlProblem::Https),
        ("ftp://x", UrlProblem::NotHttp),
        ("x:9091", UrlProblem::NotHttp),
        // No scheme at all.
        ("192.168.1.5:9091", UrlProblem::NotHttp),
        ("pi.local/transmission/rpc", UrlProblem::NotHttp),
        ("", UrlProblem::NotHttp),
        // The `url` crate reads `http:///rpc` as the host `rpc`, so the empty host it does refuse
        // is the one before a port.
        ("http://:9091/rpc", UrlProblem::NoHost),
        ("http://u:p@x/rpc", UrlProblem::Credentials),
        ("http://u@x/rpc", UrlProblem::Credentials),
        ("http://:p@x/rpc", UrlProblem::Credentials),
        ("http://x/rpc?a=1", UrlProblem::QueryOrFragment),
        ("http://x/rpc?", UrlProblem::QueryOrFragment),
        ("http://x/rpc#f", UrlProblem::QueryOrFragment),
        ("http://x/rpc#", UrlProblem::QueryOrFragment),
        // `http:nas` is a URL to the standard, but it is not what the owner means by `http://`.
        ("http:nas/rpc", UrlProblem::Invalid),
        ("http:/nas/rpc", UrlProblem::Invalid),
        ("http://x:0/rpc", UrlProblem::Invalid),
        ("http://x:65536/rpc", UrlProblem::Invalid),
        ("http://x:port/rpc", UrlProblem::Invalid),
        ("http://a b/rpc", UrlProblem::Invalid),
        ("http://[::1/rpc", UrlProblem::Invalid),
        ("http://[nas]/rpc", UrlProblem::Invalid),
        // The `url` crate drops tabs and line breaks inside a URL; 4x4 does not.
        ("http://x/r\npc", UrlProblem::Invalid),
        ("http://x/r\tpc", UrlProblem::Invalid),
        ("http://x/r pc", UrlProblem::Invalid),
    ];

    for (input, problem) in rows {
        assert_eq!(
            ConnectionSettings::from_url(input, None),
            Err(problem),
            "{input:?}"
        );
    }
}

#[test]
fn from_url_rejects_a_user_name_with_a_colon() {
    for username in ["a:b", ":", "bob:"] {
        assert_eq!(
            ConnectionSettings::from_url("http://x:9091/rpc", Some(credentials(username, "pw"))),
            Err(UrlProblem::UsernameColon),
            "{username:?}"
        );
    }
    // The password may hold one.
    assert!(
        ConnectionSettings::from_url("http://x:9091/rpc", Some(credentials("bob", "a:b"))).is_ok()
    );
}

#[test]
fn from_url_keeps_credentials_apart_from_the_url() {
    let parsed =
        ConnectionSettings::from_url("http://x:9091/rpc", Some(credentials("bob", "s@cret")))
            .unwrap();

    assert_eq!(parsed.credentials(), Some(&credentials("bob", "s@cret")));
    assert_eq!(parsed.url(), "http://x:9091/rpc");
    assert!(!parsed.url().contains("bob"));
    assert!(!parsed.url().contains("cret"));
}

#[test]
fn empty_credentials_are_none() {
    let parsed =
        ConnectionSettings::from_url("http://x:9091/rpc", Some(credentials("", ""))).unwrap();

    assert_eq!(parsed.credentials(), None);
    assert_eq!(parsed, settings("http://x:9091/rpc"));
    // A user name alone, or a password alone, is a credential.
    for (username, password) in [("bob", ""), ("", "pw")] {
        let parsed = ConnectionSettings::from_url(
            "http://x:9091/rpc",
            Some(credentials(username, password)),
        )
        .unwrap();

        assert_eq!(parsed.credentials(), Some(&credentials(username, password)));
    }
}

#[test]
fn server_label_brackets_ipv6() {
    assert_eq!(
        settings("http://pi.local:9091/rpc").server_label(),
        "pi.local:9091"
    );
    assert_eq!(
        settings("http://192.168.1.5/").server_label(),
        "192.168.1.5:80"
    );
    assert_eq!(settings("http://[::1]:9091/x").server_label(), "[::1]:9091");
    assert_eq!(
        settings("http://[fe80::1]/x").server_label(),
        "[fe80::1]:80"
    );
}

#[test]
fn debug_hides_the_password() {
    let parsed =
        ConnectionSettings::from_url("http://x:9091/rpc", Some(credentials("bob", "hunter2")))
            .unwrap();

    for text in [format!("{parsed:?}"), format!("{parsed:#?}")] {
        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("bob"), "{text}");
    }
}

#[test]
fn url_problems_never_repeat_the_url() {
    let url = "http://bob:hunter2@secret.example:9091/private";
    let problems = [
        UrlProblem::NotHttp,
        UrlProblem::Https,
        UrlProblem::NoHost,
        UrlProblem::Credentials,
        UrlProblem::QueryOrFragment,
        UrlProblem::Invalid,
        UrlProblem::UsernameColon,
    ];

    assert_eq!(
        ConnectionSettings::from_url(url, None),
        Err(UrlProblem::Credentials)
    );
    // The query is a problem of its own, and the one found first.
    assert_eq!(
        ConnectionSettings::from_url("http://secret.example/private?token=hunter2", None),
        Err(UrlProblem::QueryOrFragment)
    );
    for problem in problems {
        let text = format!("{problem} {problem:?}");

        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("secret.example"), "{text}");
    }
    assert_eq!(
        UrlProblem::UsernameColon.to_string(),
        "the user name cannot contain a colon"
    );
}
