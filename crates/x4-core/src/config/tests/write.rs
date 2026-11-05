//! Writing the connection into `config.toml`.

use super::*;

#[test]
fn writing_keeps_comments_and_other_tables() {
    let text = "# my daemon\n[connection]\n# where it is\nurl = \"http://old.local:9091/rpc\" # here\ntimeout = 5\n\n[future]\nkey = true # kept\n";
    let new = settings("http://nas.local:9091/transmission/rpc", None);

    let out = written(text, &new);

    assert_eq!(
        out,
        "# my daemon\n[connection]\n# where it is\nurl = \"http://nas.local:9091/transmission/rpc\" # here\ntimeout = 5\n\n[future]\nkey = true # kept\n"
    );
    assert_eq!(server(&out), new);
    let values = table(&out);
    assert_eq!(values["connection"]["timeout"].as_integer(), Some(5));
    assert_eq!(values["future"]["key"].as_bool(), Some(true));
}

#[test]
fn writing_adds_the_credentials_inside_the_connection_table() {
    let text = "[connection]\nurl = \"http://old/rpc\"\n\n[future]\nkey = true\n";
    let new = settings("http://nas:9091/rpc", Some(credentials("bob", "hunter2")));

    let out = written(text, &new);

    assert_eq!(
        out,
        "[connection]\nurl = \"http://nas:9091/rpc\"\nusername = \"bob\"\npassword = \"hunter2\"\n\n[future]\nkey = true\n"
    );
    assert_eq!(server(&out), new);
}

#[test]
fn writing_changes_the_credentials_in_place() {
    let text =
        "[connection]\nusername = \"old\" # who\nurl = \"http://old/rpc\"\npassword = \"old\"\n";
    let new = settings("http://nas:9091/rpc", Some(credentials("bob", "hunter2")));

    let out = written(text, &new);

    assert_eq!(
        out,
        "[connection]\nusername = \"bob\" # who\nurl = \"http://nas:9091/rpc\"\npassword = \"hunter2\"\n"
    );
}

#[test]
fn writing_removes_empty_credentials() {
    let text = "[connection]\nurl = \"http://old/rpc\"\nusername = \"bob\"\npassword = \"hunter2\"\ntimeout = 5\n";
    let new = settings("http://nas:9091/rpc", None);

    let out = written(text, &new);

    assert_eq!(
        out,
        "[connection]\nurl = \"http://nas:9091/rpc\"\ntimeout = 5\n"
    );
    assert!(!out.contains("hunter2"), "{out}");
    assert_eq!(server(&out), new);
}

#[test]
fn writing_a_password_alone_drops_the_old_user_name() {
    let text = "[connection]\nurl = \"http://old/rpc\"\nusername = \"bob\"\n";
    let new = settings("http://nas:9091/rpc", Some(credentials("", "hunter2")));

    let out = written(text, &new);

    assert!(!out.contains("bob"), "{out}");
    assert!(out.contains("password = \"hunter2\""), "{out}");
    assert_eq!(server(&out), new);
}

#[test]
fn writing_into_an_empty_text_makes_a_connection_table() {
    let new = settings("http://nas:9091/rpc", Some(credentials("bob", "hunter2")));

    let out = written("", &new);

    assert_eq!(
        out,
        "[connection]\nurl = \"http://nas:9091/rpc\"\nusername = \"bob\"\npassword = \"hunter2\"\n"
    );
    assert_eq!(server(&out), new);
}

#[test]
fn writing_adds_the_table_after_what_is_already_there() {
    let text = "# mine\ntheme = \"dark\"\n\n[future]\nkey = true\n";
    let new = settings("http://nas:9091/rpc", None);

    let out = written(text, &new);

    assert!(out.starts_with(text), "{out}");
    assert_eq!(server(&out), new);
    let values = table(&out);
    assert_eq!(values["theme"].as_str(), Some("dark"));
    assert_eq!(values["future"]["key"].as_bool(), Some(true));
}

#[test]
fn writing_replaces_a_connection_that_is_not_a_table() {
    let new = settings("http://nas:9091/rpc", None);

    for text in [
        "connection = \"http://old/rpc\"\n",
        "connection = 5\n",
        "connection = [1, 2]\n",
        "# mine\nconnection = \"x\"\nother = 1\n",
        "[[connection]]\nurl = \"http://old/rpc\"\n",
    ] {
        let out = written(text, &new);

        assert_eq!(server(&out), new, "{text:?} -> {out:?}");
        let values = table(&out);
        assert!(values["connection"].is_table(), "{text:?} -> {out:?}");
        if text.contains("other") {
            assert_eq!(values["other"].as_integer(), Some(1), "{out:?}");
            assert!(out.contains("# mine"), "{out:?}");
        }
    }
}

#[test]
fn writing_keeps_the_other_keys_of_an_inline_connection() {
    let text = "connection = { url = \"http://old/rpc\", timeout = 5 }\n";
    let new = settings("http://nas:9091/rpc", Some(credentials("bob", "hunter2")));

    let out = written(text, &new);

    assert_eq!(server(&out), new, "{out:?}");
    assert_eq!(table(&out)["connection"]["timeout"].as_integer(), Some(5));
}

#[test]
fn writing_keeps_the_other_keys_of_dotted_connection_keys() {
    let text = "connection.url = \"http://old/rpc\"\nconnection.timeout = 5\n";
    let new = settings("http://nas:9091/rpc", None);

    let out = written(text, &new);

    assert_eq!(server(&out), new, "{out:?}");
    assert_eq!(table(&out)["connection"]["timeout"].as_integer(), Some(5));
}

#[test]
fn writing_into_a_connection_that_only_holds_a_subtable() {
    let text = "[connection.extra]\nkey = 1\n";
    let new = settings("http://nas:9091/rpc", None);

    let out = written(text, &new);

    assert_eq!(server(&out), new, "{out:?}");
    assert_eq!(
        table(&out)["connection"]["extra"]["key"].as_integer(),
        Some(1)
    );
}

#[test]
fn writing_fixes_a_wrong_type() {
    let new = settings("http://nas:9091/rpc", None);

    let out = written("[connection]\nurl = 5\n", &new);
    assert_eq!(out, "[connection]\nurl = \"http://nas:9091/rpc\"\n");

    let with_password = settings("http://nas:9091/rpc", Some(credentials("bob", "hunter2")));
    let out = written(
        "[connection]\nurl = [1]\nusername = 7\npassword = true\n",
        &with_password,
    );
    assert_eq!(server(&out), with_password, "{out:?}");

    // A wrong type of a key that is to go is simply gone.
    let out = written(
        "[connection]\nurl = \"http://old/rpc\"\nusername = 7\npassword = [1]\n",
        &new,
    );
    assert_eq!(out, "[connection]\nurl = \"http://nas:9091/rpc\"\n");
}

#[test]
fn writing_broken_toml_is_a_parse_error() {
    let new = settings("http://nas:9091/rpc", None);

    let error = config_with_connection("[connection\nurl = 1", &new).expect_err("not TOML");

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
fn writing_a_parse_error_never_shows_the_file() {
    let secret = "hunter2";
    let new = settings("http://nas:9091/rpc", None);
    for text in [
        format!("[connection]\npassword = {secret}\n"),
        format!("[connection]\npassword = \"{secret}\n"),
        format!("[connection]\nurl = \"http://bob:{secret}@pi/rpc\"\nurl = 1\n"),
        format!("{secret} = = 1\n"),
    ] {
        let error = config_with_connection(&text, &new).expect_err("not TOML");

        assert!(matches!(error, ConfigError::Parse { .. }), "{error:?}");
        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(!shown.contains(secret), "{text}: {shown}");
            assert!(!shown.contains('\n'), "{text}: {shown}");
        }
    }
}

#[test]
fn writing_the_same_settings_again_changes_nothing() {
    let new = settings(
        "http://[::1]:9091/rpc",
        Some(credentials("bob", "#hash = x")),
    );
    let once = written("# mine\n[other]\nkey = 1\n", &new);

    let twice = written(&once, &new);

    assert_eq!(twice, once);
}

#[test]
fn written_text_reads_back_to_the_same_settings() {
    let hosts = [
        "pi.local",
        "192.168.1.5",
        "[::1]",
        "[fe80::1]",
        "[2001:db8::2]",
    ];
    let passwords = [
        "hunter2",
        "a\"b",
        "back\\slash",
        "#hash = x",
        "line\nbreak",
        "ä@:",
    ];
    let users = ["bob", "", "ünï", " pad "];
    let before = "# mine\n[connection]\nurl = \"http://old/rpc\"\n\n[other]\nkey = 1\n";

    for host in hosts {
        for password in passwords {
            for user in users {
                let new = settings(
                    &format!("http://{host}:9091/transmission/rpc"),
                    Some(credentials(user, password)),
                );

                for text in ["", before] {
                    let out = written(text, &new);

                    assert_eq!(server(&out), new, "{host} {user:?} {password:?}\n{out}");
                    assert_eq!(
                        Config::parse(&out),
                        Ok(Config {
                            connection: Some(new.clone())
                        })
                    );
                }
            }
        }
    }
}
