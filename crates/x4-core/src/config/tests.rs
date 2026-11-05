use super::*;

fn credentials(username: &str, password: &str) -> Credentials {
    Credentials {
        username: username.to_owned(),
        password: password.to_owned(),
    }
}

/// The settings of `url`, with the user name and password that go with it.
fn settings(url: &str, credentials: Option<Credentials>) -> ConnectionSettings {
    ConnectionSettings::from_url(url, credentials)
        .unwrap_or_else(|problem| panic!("{url:?}: {problem}"))
}

/// The server `config.toml` names in `text`, which must be valid and name one.
fn server(text: &str) -> ConnectionSettings {
    Config::parse(text)
        .unwrap_or_else(|error| panic!("{error}"))
        .connection
        .unwrap_or_else(|| panic!("no server in {text:?}"))
}

/// `text` with `new` written into it.
fn written(text: &str, new: &ConnectionSettings) -> String {
    config_with_connection(text, new).unwrap_or_else(|error| panic!("{text:?}: {error}"))
}

/// `text` as plain TOML values, to look at keys and tables whatever their layout.
fn table(text: &str) -> toml::Table {
    toml::from_str(text).unwrap_or_else(|error| panic!("{text:?}: {error}"))
}

// Reading.

#[test]
fn config_without_a_url_sets_no_server() {
    for text in ["", "[other]\nkey = 1\n", "[connection]\n"] {
        let config = Config::parse(text).expect("a valid file");

        assert_eq!(config.connection, None, "{text:?}");
        assert_eq!(config, Config::default(), "{text:?}");
    }
}

#[test]
fn credentials_without_a_url_set_no_server() {
    let config = Config::parse("[connection]\nusername = \"bob\"\npassword = \"pw\"\n")
        .expect("a valid file");

    assert_eq!(config.connection, None);
}

#[test]
fn config_rejects_what_is_not_a_plain_http_address() {
    // (what is written, what is wrong with it)
    let rows = [
        ("ftp://x", UrlProblem::NotHttp),
        ("https://pi.local/transmission/rpc", UrlProblem::Https),
        ("HTTPS://pi.local/transmission/rpc", UrlProblem::Https),
        ("192.168.1.5:9091", UrlProblem::NotHttp),
        ("pi.local/rpc", UrlProblem::NotHttp),
        ("", UrlProblem::NotHttp),
        ("httpx://x", UrlProblem::NotHttp),
        ("xhttp://x", UrlProblem::NotHttp),
        ("http//x", UrlProblem::NotHttp),
        ("http:/x", UrlProblem::Invalid),
        // The `url` crate reports an empty host before it looks at the scheme.
        ("http:", UrlProblem::NoHost),
        ("https://", UrlProblem::NoHost),
        ("ftp://", UrlProblem::NoHost),
        ("http://:9091/rpc", UrlProblem::NoHost),
        ("http://pi/rpc?token=1", UrlProblem::QueryOrFragment),
        ("http://pi/rpc#top", UrlProblem::QueryOrFragment),
        ("http://pi:0/rpc", UrlProblem::Invalid),
        ("http://pi/a b", UrlProblem::Invalid),
        ("http://bob@pi/rpc", UrlProblem::Credentials),
        ("http://bob:hunter2@pi/rpc", UrlProblem::Credentials),
        ("http://:hunter2@pi/rpc", UrlProblem::Credentials),
    ];
    for (url, problem) in rows {
        let text = format!("[connection]\nurl = {url:?}\n");

        assert_eq!(
            Config::parse(&text),
            Err(ConfigError::InvalidUrl(problem)),
            "{url:?}"
        );
    }
}

#[test]
fn config_takes_the_url_it_is_given() {
    let new = server("[connection]\nurl = \"http://pi.local:9091/transmission/rpc\"\n");

    assert_eq!(new.host(), "pi.local");
    assert_eq!(new.port(), 9091);
    assert_eq!(new.path(), "/transmission/rpc");
    assert_eq!(new.url(), "http://pi.local:9091/transmission/rpc");
    assert_eq!(new.credentials(), None);
}

#[test]
fn config_tolerates_unknown_keys_and_comments() {
    let text = "# my daemon\n[connection]\nurl = \"http://x/rpc\" # here\ntimeout = 5\n\n[future]\nkey = true\n";

    assert_eq!(server(text).url(), "http://x:80/rpc");
}

#[test]
fn config_reads_a_connection_written_as_an_inline_table_or_dotted_keys() {
    for text in [
        "connection = { url = \"http://x/rpc\" }\n",
        "connection.url = \"http://x/rpc\"\n",
    ] {
        assert_eq!(server(text).url(), "http://x:80/rpc", "{text:?}");
    }
}

#[test]
fn config_accepts_the_http_scheme_in_any_case() {
    for url in [
        "http://x/rpc",
        "HTTP://x/rpc",
        "Http://x/rpc",
        "hTTp://x/rpc",
    ] {
        let text = format!("[connection]\nurl = {url:?}\n");

        assert_eq!(server(&text).url(), "http://x:80/rpc", "{url:?}");
    }
}

#[test]
fn credentials_come_from_their_keys() {
    let creds = |text: &str| server(text).credentials().cloned();
    let url = "url = \"http://pi/rpc\"\n";

    assert_eq!(
        creds(&format!(
            "[connection]\n{url}username = \"bob\"\npassword = \"hunter2\"\n"
        )),
        Some(credentials("bob", "hunter2"))
    );
    // One key alone: the other counts as empty.
    assert_eq!(
        creds(&format!("[connection]\n{url}username = \"bob\"\n")),
        Some(credentials("bob", ""))
    );
    assert_eq!(
        creds(&format!("[connection]\n{url}password = \"hunter2\"\n")),
        Some(credentials("", "hunter2"))
    );
    // Neither, or both empty: no credentials.
    assert_eq!(creds(&format!("[connection]\n{url}")), None);
    assert_eq!(
        creds(&format!(
            "[connection]\n{url}username = \"\"\npassword = \"\"\n"
        )),
        None
    );
    // Never trimmed.
    assert_eq!(
        creds(&format!(
            "[connection]\n{url}username = \" bob \"\npassword = \" pw \"\n"
        )),
        Some(credentials(" bob ", " pw "))
    );
}

#[test]
fn credentials_in_the_url_are_refused_with_a_pointer_to_the_keys() {
    let error = Config::parse("[connection]\nurl = \"http://bob:hunter2@pi/rpc\"\n")
        .expect_err("the credentials belong in their own keys");

    assert_eq!(error, ConfigError::InvalidUrl(UrlProblem::Credentials));
    let shown = error.to_string();
    assert!(shown.contains("connection.username"), "{shown}");
    assert!(shown.contains("connection.password"), "{shown}");
    for shown in [shown, format!("{error:?}")] {
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(!shown.contains("bob"), "{shown}");
    }
}

#[test]
fn a_user_name_with_a_colon_is_refused_and_named() {
    let error = Config::parse(
        "[connection]\nurl = \"http://pi/rpc\"\nusername = \"a:b\"\npassword = \"hunter2\"\n",
    )
    .expect_err("a user name cannot hold a colon");

    assert_eq!(error, ConfigError::InvalidUrl(UrlProblem::UsernameColon));
    let shown = error.to_string();
    assert_eq!(shown, "connection.username cannot contain a colon");
    assert!(!shown.contains("connection.url"), "{shown}");
}

#[test]
fn config_wrong_type_is_a_parse_error_with_a_position() {
    let error = Config::parse("[connection]\nurl = 5\n").expect_err("a number is not a URL");
    let ConfigError::Parse { position, message } = &error else {
        panic!("a wrong type is a parse error: {error:?}");
    };

    assert_eq!(*position, Some(TextPosition { line: 2, column: 7 }));
    assert_eq!(message, "invalid type: integer, expected a string");
    assert_eq!(
        error.to_string(),
        "cannot parse config.toml: line 2, column 7: invalid type: integer, expected a string"
    );
    assert!(!error.to_string().contains('\n'), "{error}");
}

#[test]
fn config_wrong_type_of_a_credential_is_a_parse_error_with_a_position() {
    for (text, line) in [
        ("[connection]\nurl = \"http://x/rpc\"\nusername = 5\n", 3),
        ("[connection]\nurl = \"http://x/rpc\"\npassword = true\n", 3),
    ] {
        let error = Config::parse(text).expect_err("not a string");

        assert!(
            matches!(
                error,
                ConfigError::Parse {
                    position: Some(TextPosition { line: found, .. }),
                    ..
                } if found == line
            ),
            "{text:?}: {error:?}"
        );
    }
}

#[test]
fn config_broken_toml_is_a_parse_error_with_a_position() {
    let error = Config::parse("[connection\nurl = 1").expect_err("not TOML");

    assert!(
        matches!(
            error,
            ConfigError::Parse {
                position: Some(TextPosition { line: 1, .. }),
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn config_errors_say_what_is_wrong() {
    let message = |problem| ConfigError::InvalidUrl(problem).to_string();

    assert_eq!(
        message(UrlProblem::NotHttp),
        "connection.url the address must start with http://"
    );
    assert_eq!(
        message(UrlProblem::Https),
        "connection.url https is not supported yet, only plain http"
    );
    assert_eq!(
        message(UrlProblem::NoHost),
        "connection.url the address has no host"
    );
    assert_eq!(
        message(UrlProblem::QueryOrFragment),
        "connection.url the address has a query or a fragment"
    );
    assert_eq!(
        message(UrlProblem::Invalid),
        "connection.url the address is not valid"
    );
    // The credentials belong in their own keys, and the colon is the user name's fault.
    let credentials = message(UrlProblem::Credentials);
    assert!(credentials.starts_with("connection.url "), "{credentials}");
    assert!(credentials.contains("connection.username"), "{credentials}");
    assert!(credentials.contains("connection.password"), "{credentials}");
    assert_eq!(
        message(UrlProblem::UsernameColon),
        "connection.username cannot contain a colon"
    );
    let parse = |position| ConfigError::Parse {
        position,
        message: "boom".to_owned(),
    };
    assert_eq!(
        parse(Some(TextPosition { line: 1, column: 2 })).to_string(),
        "cannot parse config.toml: line 1, column 2: boom"
    );
    assert_eq!(parse(None).to_string(), "cannot parse config.toml: boom");
}

#[test]
fn config_errors_never_show_the_url_or_the_credentials() {
    let secret = "hunter2";
    for text in [
        format!("[connection]\nurl = \"https://user:{secret}@pi/rpc\"\n"),
        format!("[connection]\nurl = \"ftp://user:{secret}@pi/rpc\"\n"),
        format!("[connection]\nurl = \"http://user:{secret}@pi/rpc\"\n"),
        format!("[connection]\nurl = \"user:{secret}@pi/rpc\"\n"),
        format!("[connection]\nurl = \"{secret}\"\n"),
        format!("[connection]\nurl = \"http://pi/rpc?{secret}\"\n"),
        format!("[connection]\nurl = \"http://pi/rpc\"\nusername = \"user:{secret}\"\n"),
    ] {
        let error = Config::parse(&text).expect_err("the settings cannot be used");

        assert!(matches!(error, ConfigError::InvalidUrl(_)), "{error:?}");
        for shown in [
            error.to_string(),
            format!("{error:?}"),
            format!("{error:#?}"),
        ] {
            assert!(!shown.contains(secret), "{text}: {shown}");
            assert!(!shown.contains("user:"), "{text}: {shown}");
            assert!(!shown.contains("pi/rpc"), "{text}: {shown}");
        }
    }
}

#[test]
fn config_parse_errors_never_show_the_value() {
    let secret = "hunter2";
    for text in [
        format!("connection = \"https://user:{secret}@pi/rpc\"\n"),
        format!("[connection]\nurl = 5\nother = \"{secret}\"\n"),
        format!("connection = {{ url = [\"{secret}\"] }}\n"),
        format!("[connection]\nurl = \"http://pi/rpc\"\npassword = [\"{secret}\"]\n"),
        format!("[connection]\nurl = \"http://pi/rpc\"\npassword = {{ x = \"{secret}\" }}\n"),
        format!("[connection]\nurl = \"http://pi/rpc\"\nusername = 7\npassword = \"{secret}\"\n"),
        format!("[connection]\nurl = \"http://pi/rpc\"\npassword = {secret}\n"),
    ] {
        let error = Config::parse(&text).expect_err("a wrong type or broken TOML");
        assert!(
            matches!(error, ConfigError::Parse { .. }),
            "{text}: a wrong type is a parse error"
        );

        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(!shown.contains(secret), "{text}: {shown}");
            assert!(!shown.contains("user"), "{text}: {shown}");
        }
    }
}

#[test]
fn a_config_never_shows_the_password() {
    let config = Config::parse(
        "[connection]\nurl = \"http://pi/rpc\"\nusername = \"bob\"\npassword = \"hunter2\"\n",
    )
    .expect("a valid file");

    for shown in [format!("{config:?}"), format!("{config:#?}")] {
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}

mod write;
