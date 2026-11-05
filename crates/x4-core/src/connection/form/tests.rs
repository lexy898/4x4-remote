use super::*;

fn form(host: &str, port: &str, path: &str, username: &str, password: &str) -> ConnectionForm {
    ConnectionForm {
        host: host.to_owned(),
        port: port.to_owned(),
        path: path.to_owned(),
        username: username.to_owned(),
        password: password.to_owned(),
    }
}

/// A form with nothing wrong in it, to change one field at a time.
fn filled() -> ConnectionForm {
    form("pi.local", "9091", "/transmission/rpc", "", "")
}

fn credentials(username: &str, password: &str) -> Credentials {
    Credentials {
        username: username.to_owned(),
        password: password.to_owned(),
    }
}

fn spread(host: &str) -> Option<ConnectionForm> {
    ConnectionForm {
        host: host.to_owned(),
        ..ConnectionForm::of(None)
    }
    .spread()
}

#[test]
fn form_of_none_has_the_transmission_defaults() {
    assert_eq!(
        ConnectionForm::of(None),
        form("", "9091", "/transmission/rpc", "", "")
    );
    assert_eq!(DEFAULT_PORT, 9091);
    assert_eq!(DEFAULT_PATH, "/transmission/rpc");
}

#[test]
fn form_of_settings_shows_every_part_of_them() {
    let ipv6 =
        ConnectionSettings::from_url("http://[::1]:8080/rpc", Some(credentials(" bob", "p w")))
            .unwrap();
    let plain = ConnectionSettings::from_url("http://nas/", None).unwrap();

    assert_eq!(
        ConnectionForm::of(Some(&ipv6)),
        form("::1", "8080", "/rpc", " bob", "p w")
    );
    assert_eq!(
        ConnectionForm::of(Some(&plain)),
        form("nas", "80", "/", "", "")
    );
}

#[test]
fn form_check_accepts_ipv6_both_ways() {
    let bare = ConnectionForm {
        host: "::1".to_owned(),
        ..filled()
    }
    .check()
    .unwrap();
    let bracketed = ConnectionForm {
        host: "[::1]".to_owned(),
        ..filled()
    }
    .check()
    .unwrap();

    assert_eq!(bare, bracketed);
    assert_eq!(bare.host(), "::1");
    assert_eq!(bare.url(), "http://[::1]:9091/transmission/rpc");
    assert_eq!(bare.server_label(), "[::1]:9091");
    // The form shows the host without brackets, and what it shows checks again to the same.
    assert_eq!(ConnectionForm::of(Some(&bare)).check().unwrap(), bare);
}

#[test]
fn form_check_round_trips_every_kind_of_host() {
    for host in [
        "::1",
        "[::1]",
        "fe80::1",
        "[FE80::1]",
        "192.168.1.5",
        "pi.local",
        "NAS",
    ] {
        let checked = ConnectionForm {
            host: host.to_owned(),
            ..filled()
        }
        .check()
        .unwrap_or_else(|problems| panic!("{host:?}: {problems:?}"));

        assert_eq!(
            ConnectionSettings::from_url(&checked.url(), None),
            Ok(checked.clone()),
            "{host:?}"
        );
        assert_eq!(
            ConnectionForm::of(Some(&checked)).check(),
            Ok(checked),
            "{host:?}"
        );
    }
}

#[test]
fn form_check_reports_every_problem_at_once() {
    let problems = form("", "0", "", "a:b", "pw").check().unwrap_err();

    assert_eq!(
        problems,
        FormProblems {
            host: Some(HostProblem::Empty),
            port: Some(PortProblem::OutOfRange),
            path: Some(PathProblem::Empty),
            username: Some(UsernameProblem::Colon),
        }
    );
}

#[test]
fn form_check_reports_only_the_fields_that_are_wrong() {
    let problems = ConnectionForm {
        port: "x".to_owned(),
        ..filled()
    }
    .check()
    .unwrap_err();

    assert_eq!(
        problems,
        FormProblems {
            port: Some(PortProblem::NotANumber),
            ..FormProblems::default()
        }
    );
}

#[test]
fn form_host_rules() {
    let invalid = [
        "na s",
        "nas/rpc",
        "user@nas",
        "nas?x",
        "nas#x",
        "nas\\rpc",
        "nas:9091",
        "a<b",
        "a\u{7}b",
        // A full address is for `spread`; unspread, it is not a host.
        "http://nas",
        // Brackets belong to an IPv6 literal only, and both of them.
        "[nas]",
        "[::1",
        "::1]",
        "[[::1]]",
    ];

    for host in invalid {
        let problems = ConnectionForm {
            host: host.to_owned(),
            ..filled()
        }
        .check();

        assert_eq!(
            problems.map_err(|problems| problems.host),
            Err(Some(HostProblem::Invalid)),
            "{host:?}"
        );
    }
    for (host, problem) in [
        ("", HostProblem::Empty),
        (" \t ", HostProblem::Empty),
        ("https://nas", HostProblem::Https),
        ("HTTPS://nas:9091/rpc", HostProblem::Https),
    ] {
        let problems = ConnectionForm {
            host: host.to_owned(),
            ..filled()
        }
        .check();

        assert_eq!(
            problems.map_err(|problems| problems.host),
            Err(Some(problem)),
            "{host:?}"
        );
    }
}

#[test]
fn form_host_is_trimmed_and_normalized() {
    for (typed, stored) in [
        (" nas ", "nas"),
        ("\tPi.Local\n", "pi.local"),
        ("[::1]", "::1"),
        (" [FE80::1] ", "fe80::1"),
        ("127.1", "127.0.0.1"),
        ("bücher.example", "xn--bcher-kva.example"),
    ] {
        let checked = ConnectionForm {
            host: typed.to_owned(),
            ..filled()
        }
        .check()
        .unwrap_or_else(|problems| panic!("{typed:?}: {problems:?}"));

        assert_eq!(checked.host(), stored, "{typed:?}");
    }
}

#[test]
fn form_port_rules() {
    for (port, problem) in [
        ("", PortProblem::Empty),
        ("   ", PortProblem::Empty),
        ("9o91", PortProblem::NotANumber),
        ("+80", PortProblem::NotANumber),
        ("-1", PortProblem::NotANumber),
        ("90 91", PortProblem::NotANumber),
        ("80.0", PortProblem::NotANumber),
        // Digits other than ASCII ones.
        ("\u{663}", PortProblem::NotANumber),
        ("0", PortProblem::OutOfRange),
        ("65536", PortProblem::OutOfRange),
        ("99999999999999999999999", PortProblem::OutOfRange),
    ] {
        let problems = ConnectionForm {
            port: port.to_owned(),
            ..filled()
        }
        .check();

        assert_eq!(
            problems.map_err(|problems| problems.port),
            Err(Some(problem)),
            "{port:?}"
        );
    }
    for (port, number) in [(" 9091 ", 9091), ("1", 1), ("65535", 65535), ("0080", 80)] {
        let checked = ConnectionForm {
            port: port.to_owned(),
            ..filled()
        }
        .check()
        .unwrap();

        assert_eq!(checked.port(), number, "{port:?}");
    }
}

#[test]
fn form_path_gets_its_leading_slash() {
    for (typed, stored) in [
        ("rpc", "/rpc"),
        ("transmission/rpc", "/transmission/rpc"),
        (" /rpc ", "/rpc"),
        ("/", "/"),
        // The `url` crate has the last word, so the form's path is the settings' path.
        ("/a/../b", "/b"),
        ("/ä", "/%C3%A4"),
    ] {
        let checked = ConnectionForm {
            path: typed.to_owned(),
            ..filled()
        }
        .check()
        .unwrap();

        assert_eq!(checked.path(), stored, "{typed:?}");
    }
}

#[test]
fn form_path_rules() {
    for (path, problem) in [
        ("", PathProblem::Empty),
        ("  ", PathProblem::Empty),
        ("/a?b", PathProblem::Invalid),
        ("/a#b", PathProblem::Invalid),
        ("?", PathProblem::Invalid),
        ("/a b", PathProblem::Invalid),
        ("a\tb", PathProblem::Invalid),
        ("/a\u{7}b", PathProblem::Invalid),
    ] {
        let problems = ConnectionForm {
            path: path.to_owned(),
            ..filled()
        }
        .check();

        assert_eq!(
            problems.map_err(|problems| problems.path),
            Err(Some(problem)),
            "{path:?}"
        );
    }
}

#[test]
fn form_keeps_spaces_in_the_password() {
    let checked = ConnectionForm {
        username: " bob ".to_owned(),
        password: "  pass word  ".to_owned(),
        ..filled()
    }
    .check()
    .unwrap();

    assert_eq!(
        checked.credentials(),
        Some(&credentials(" bob ", "  pass word  "))
    );
}

#[test]
fn form_carries_a_password_with_every_special_character_unchanged() {
    let password = "p\"a\\ss#=\n\u{e4}@:/? %";
    let checked = ConnectionForm {
        username: "b\u{f6}b".to_owned(),
        password: password.to_owned(),
        ..filled()
    }
    .check()
    .unwrap();

    assert_eq!(
        checked.credentials(),
        Some(&credentials("b\u{f6}b", password))
    );
    assert_eq!(checked.url(), "http://pi.local:9091/transmission/rpc");
    assert_eq!(ConnectionForm::of(Some(&checked)).password, password);
}

#[test]
fn form_without_credentials_has_none() {
    assert_eq!(filled().check().unwrap().credentials(), None);
    // A user name alone, or a password alone, is a credential.
    let alone = ConnectionForm {
        username: "bob".to_owned(),
        ..filled()
    };

    assert_eq!(
        alone.check().unwrap().credentials(),
        Some(&credentials("bob", ""))
    );
    let alone = ConnectionForm {
        password: "pw".to_owned(),
        ..filled()
    };

    assert_eq!(
        alone.check().unwrap().credentials(),
        Some(&credentials("", "pw"))
    );
}

#[test]
fn form_and_url_agree_about_the_user_name() {
    let colon = ConnectionForm {
        username: "a:b".to_owned(),
        ..filled()
    };

    assert_eq!(
        colon.check().map_err(|problems| problems.username),
        Err(Some(UsernameProblem::Colon))
    );
    assert_eq!(
        ConnectionSettings::from_url(
            "http://pi.local:9091/transmission/rpc",
            Some(credentials("a:b", ""))
        ),
        Err(UrlProblem::UsernameColon)
    );
}

#[test]
fn debug_hides_the_password() {
    let form = ConnectionForm {
        username: "bob".to_owned(),
        password: "hunter2".to_owned(),
        ..filled()
    };

    for text in [format!("{form:?}"), format!("{form:#?}")] {
        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("bob"), "{text}");
        assert!(text.contains("pi.local"), "{text}");
    }
}

#[test]
fn spread_splits_a_full_address() {
    let spread = spread("http://bob:s%40cret@nas:8080/rpc").unwrap();

    assert_eq!(spread, form("nas", "8080", "/rpc", "bob", "s@cret"));
    assert_eq!(spread.check().unwrap().url(), "http://nas:8080/rpc");
}

#[test]
fn spread_keeps_the_path_for_a_bare_root() {
    for address in ["http://nas:9091/", "http://nas:9091", " HTTP://Nas:9091 "] {
        assert_eq!(
            spread(address).unwrap(),
            form("nas", "9091", "/transmission/rpc", "", ""),
            "{address:?}"
        );
    }
}

#[test]
fn spread_defaults_the_port_to_80() {
    assert_eq!(
        spread("http://nas/rpc").unwrap(),
        form("nas", "80", "/rpc", "", "")
    );
}

#[test]
fn spread_takes_host_colon_port() {
    assert_eq!(
        spread("nas:8080").unwrap(),
        form("nas", "8080", "/transmission/rpc", "", "")
    );
    assert_eq!(
        spread(" 192.168.1.5:9091 ").unwrap(),
        form("192.168.1.5", "9091", "/transmission/rpc", "", "")
    );
    assert_eq!(
        spread("[::1]:8080").unwrap(),
        form("::1", "8080", "/transmission/rpc", "", "")
    );
}

#[test]
fn spread_ignores_plain_hosts_and_ipv6() {
    for host in [
        "",
        "nas",
        "::1",
        "[::1]",
        "fe80::1",
        "nas:",
        ":9091",
        "nas:port",
        "nas:80:90",
        "[nas]:9091",
        "nas/rpc",
    ] {
        assert_eq!(spread(host), None, "{host:?}");
    }
}

#[test]
fn spread_leaves_what_it_cannot_split_to_the_check() {
    for host in [
        "https://nas:9091/rpc",
        "ftp://nas",
        "http://nas/rpc?x=1",
        "http://nas/rpc#f",
        "http://nas:0/rpc",
        "http://",
        "http://na s/rpc",
        "http://x/r\npc",
        // A user name that is not UTF-8 once decoded.
        "http://%ff:pw@nas/rpc",
        "http://bob:%ff@nas/rpc",
    ] {
        assert_eq!(spread(host), None, "{host:?}");
    }
}

#[test]
fn spread_keeps_the_credentials_the_address_does_not_have() {
    let before = ConnectionForm {
        host: "http://nas:8080/rpc".to_owned(),
        username: "old".to_owned(),
        password: "typed".to_owned(),
        ..ConnectionForm::of(None)
    };

    assert_eq!(
        before.spread().unwrap(),
        form("nas", "8080", "/rpc", "old", "typed")
    );
    let with_user = ConnectionForm {
        host: "http://new@nas:8080/rpc".to_owned(),
        ..before.clone()
    };

    assert_eq!(
        with_user.spread().unwrap(),
        form("nas", "8080", "/rpc", "new", "typed")
    );
}

#[test]
fn spread_is_pure() {
    let before = ConnectionForm {
        host: "http://bob:pw@nas:8080/rpc".to_owned(),
        ..ConnectionForm::of(None)
    };
    let copy = before.clone();

    let spread = before.spread();

    assert_eq!(before, copy);
    assert_ne!(spread, Some(copy));
}

#[test]
fn spread_decodes_the_user_name_and_password_only() {
    let spread = spread("http://a%3Ab:p%25w%20d@nas:80/a%20b").unwrap();

    assert_eq!(spread.username, "a:b");
    assert_eq!(spread.password, "p%w d");
    // A path stays as the `url` crate writes it.
    assert_eq!(spread.path, "/a%20b");
    // The colon in the user name is for the check to report, on its own field.
    assert_eq!(
        spread.check().map_err(|problems| problems.username),
        Err(Some(UsernameProblem::Colon))
    );
}
