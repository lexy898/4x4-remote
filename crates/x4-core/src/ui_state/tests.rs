use super::*;

/// Parses `text` and normalizes the result, the way the app reads the file.
fn load(text: &str) -> UiState {
    UiState::parse(text)
        .expect("the text is valid")
        .state
        .normalized()
}

fn column(id: ColumnId, visible: bool, width: f32) -> ColumnState {
    ColumnState { id, visible, width }
}

fn ids(columns: &[ColumnState]) -> Vec<ColumnId> {
    columns.iter().map(|column| column.id).collect()
}

/// A state where nothing is the default, with a width that has no exact decimal form.
fn customized() -> UiState {
    let mut columns = default_columns();
    columns.reverse();
    if let Some(first) = columns.first_mut() {
        first.visible = false;
        first.width = 123.456;
    }
    UiState {
        window: WindowState {
            width: 1234.5,
            height: 801.0,
            maximized: true,
        },
        columns,
        sort: SortState {
            key: SortKey::DownloadSpeed,
            direction: SortDirection::Descending,
        },
        filter: Filter::Downloading,
        theme: ThemeChoice::Dark,
        ..UiState::default()
    }
}

#[test]
fn ui_state_default_is_the_first_run() {
    let state = UiState::default();

    assert_eq!(state.version, UI_STATE_VERSION);
    assert_eq!(UI_STATE_VERSION, 1);
    assert_eq!(
        state.window,
        WindowState {
            width: 1100.0,
            height: 700.0,
            maximized: false
        }
    );
    assert_eq!(state.columns, default_columns());
    assert_eq!(state.sort.key, SortKey::Name);
    assert_eq!(state.sort.direction, SortDirection::Ascending);
    assert_eq!(state.filter, Filter::All);
    assert_eq!(state.theme, ThemeChoice::System);
}

#[test]
fn default_columns_show_everything_at_their_widths() {
    let expected = [
        (ColumnId::Name, 360.0),
        (ColumnId::Size, 90.0),
        (ColumnId::Progress, 160.0),
        (ColumnId::State, 110.0),
        (ColumnId::Down, 90.0),
        (ColumnId::Up, 90.0),
        (ColumnId::Eta, 80.0),
        (ColumnId::Ratio, 70.0),
        (ColumnId::Peers, 60.0),
    ]
    .map(|(id, width)| column(id, true, width));

    assert_eq!(default_columns(), expected);
}

#[test]
fn ui_state_roundtrips() {
    for state in [UiState::default(), customized()] {
        let text = state.to_toml().expect("the state can be written");

        let parsed = UiState::parse(&text).expect("what was written can be read");

        assert_eq!(parsed.state, state, "{text}");
        assert!(!parsed.newer);
    }
}

#[test]
fn ui_state_is_written_with_stable_names() {
    let text = UiState::default()
        .to_toml()
        .expect("the state can be written");

    assert_eq!(
        text,
        r#"version = 1
filter = "all"
theme = "system"

[window]
width = 1100.0
height = 700.0
maximized = false

[[columns]]
id = "name"
visible = true
width = 360.0

[[columns]]
id = "size"
visible = true
width = 90.0

[[columns]]
id = "progress"
visible = true
width = 160.0

[[columns]]
id = "state"
visible = true
width = 110.0

[[columns]]
id = "down"
visible = true
width = 90.0

[[columns]]
id = "up"
visible = true
width = 90.0

[[columns]]
id = "eta"
visible = true
width = 80.0

[[columns]]
id = "ratio"
visible = true
width = 70.0

[[columns]]
id = "peers"
visible = true
width = 60.0

[sort]
key = "name"
direction = "ascending"
"#
    );
}

#[test]
fn ui_state_writes_every_enum_value_by_its_snake_case_name() {
    let state = UiState {
        sort: SortState {
            key: SortKey::UploadSpeed,
            direction: SortDirection::Descending,
        },
        filter: Filter::Active,
        theme: ThemeChoice::Light,
        ..UiState::default()
    };

    let text = state.to_toml().expect("the state can be written");

    assert!(text.contains("key = \"upload_speed\""), "{text}");
    assert!(text.contains("direction = \"descending\""), "{text}");
    assert!(text.contains("filter = \"active\""), "{text}");
    assert!(text.contains("theme = \"light\""), "{text}");
}

#[test]
fn ui_state_tolerates_unknown_keys() {
    let text = r#"
        version = 1
        future_setting = "x"

        [window]
        width = 900.0
        height = 650.0
        maximized = true
        dock = "left"

        [foo]
        bar = 1

        [sort]
        key = "size"
        direction = "descending"
        stable = true

        [[columns]]
        id = "name"
        visible = true
        width = 300.0
        color = "red"
    "#;

    let parsed = UiState::parse(text).expect("unknown keys are ignored");

    assert!(!parsed.newer);
    assert_eq!(
        parsed.state.window,
        WindowState {
            width: 900.0,
            height: 650.0,
            maximized: true
        }
    );
    assert_eq!(
        parsed.state.sort,
        SortState {
            key: SortKey::Size,
            direction: SortDirection::Descending
        }
    );
    assert_eq!(parsed.state.columns, [column(ColumnId::Name, true, 300.0)]);
}

#[test]
fn ui_state_unknown_enum_values_fall_back() {
    let text = r#"
        filter = "laser"
        theme = "neon"

        [sort]
        key = "weight"
        direction = "sideways"

        [[columns]]
        id = "name"
        visible = true
        width = 300.0

        [[columns]]
        id = "bogus"
        visible = true
        width = 100.0

        [[columns]]
        id = "size"
        visible = false
        width = 95.0
    "#;

    let state = UiState::parse(text)
        .expect("unknown values do not fail the file")
        .state;

    assert_eq!(state.filter, Filter::All);
    assert_eq!(state.theme, ThemeChoice::System);
    assert_eq!(state.sort, SortState::default());
    assert_eq!(
        state.columns,
        [
            column(ColumnId::Name, true, 300.0),
            column(ColumnId::Size, false, 95.0)
        ]
    );
}

#[test]
fn ui_state_unknown_values_do_not_reset_their_neighbors() {
    let text = "filter = \"laser\"\ntheme = \"dark\"\n[sort]\nkey = \"weight\"\ndirection = \"descending\"\n";

    let state = UiState::parse(text).expect("a valid file").state;

    assert_eq!(state.theme, ThemeChoice::Dark);
    assert_eq!(
        state.sort,
        SortState {
            key: SortKey::Name,
            direction: SortDirection::Descending
        }
    );
}

#[test]
fn ui_state_reads_every_known_enum_name() {
    for filter in [
        Filter::All,
        Filter::Downloading,
        Filter::Seeding,
        Filter::Paused,
        Filter::Active,
        Filter::Error,
    ] {
        let text = UiState {
            filter,
            ..UiState::default()
        }
        .to_toml()
        .expect("the state can be written");
        assert_eq!(UiState::parse(&text).expect("valid").state.filter, filter);
    }
    for key in [
        SortKey::Name,
        SortKey::Size,
        SortKey::Progress,
        SortKey::State,
        SortKey::DownloadSpeed,
        SortKey::UploadSpeed,
        SortKey::Eta,
        SortKey::Ratio,
        SortKey::Peers,
        SortKey::Added,
    ] {
        let text = UiState {
            sort: SortState {
                key,
                direction: SortDirection::Descending,
            },
            ..UiState::default()
        }
        .to_toml()
        .expect("the state can be written");
        assert_eq!(UiState::parse(&text).expect("valid").state.sort.key, key);
    }
    for theme in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
        let text = UiState {
            theme,
            ..UiState::default()
        }
        .to_toml()
        .expect("the state can be written");
        assert_eq!(UiState::parse(&text).expect("valid").state.theme, theme);
    }
}

#[test]
fn ui_state_missing_fields_use_defaults() {
    assert_eq!(
        UiState::parse("").expect("an empty file is valid"),
        ParsedUiState {
            state: UiState::default(),
            newer: false
        }
    );

    let state = UiState::parse("[window]\nwidth = 900.0\n")
        .expect("a partial file is valid")
        .state;
    assert_eq!(
        state.window,
        WindowState {
            width: 900.0,
            ..WindowState::default()
        }
    );
    assert_eq!(state.columns, default_columns());
    assert_eq!(state.sort, SortState::default());
}

#[test]
fn ui_state_a_column_without_visible_or_width_takes_the_defaults() {
    let state = UiState::parse("[[columns]]\nid = \"ratio\"\n\n[[columns]]\nvisible = false\n")
        .expect("a valid file")
        .state;

    // The second entry names no column, so it goes.
    assert_eq!(state.columns, [column(ColumnId::Ratio, true, 70.0)]);
}

#[test]
fn ui_state_reads_integer_sizes() {
    let state = UiState::parse(
        "[window]\nwidth = 900\nheight = 600\n[[columns]]\nid = \"name\"\nwidth = 250\n",
    )
    .expect("TOML integers are fine for sizes")
    .state;

    assert_eq!(state.window.width, 900.0);
    assert_eq!(state.window.height, 600.0);
    assert_eq!(state.columns, [column(ColumnId::Name, true, 250.0)]);
}

#[test]
fn ui_state_normalizes_columns() {
    let text = r#"
        [[columns]]
        id = "peers"
        visible = true
        width = 5.0

        [[columns]]
        id = "name"
        visible = false
        width = 5000.0

        [[columns]]
        id = "peers"
        visible = false
        width = 300.0
    "#;

    let state = load(text);

    assert_eq!(
        state.columns,
        [
            column(ColumnId::Peers, true, 40.0),
            column(ColumnId::Name, true, 1200.0),
            column(ColumnId::Size, true, 90.0),
            column(ColumnId::Progress, true, 160.0),
            column(ColumnId::State, true, 110.0),
            column(ColumnId::Down, true, 90.0),
            column(ColumnId::Up, true, 90.0),
            column(ColumnId::Eta, true, 80.0),
            column(ColumnId::Ratio, true, 70.0),
        ]
    );
}

#[test]
fn ui_state_normalization_keeps_what_is_fine() {
    let state = customized();

    assert_eq!(state.clone().normalized(), state);
    assert_eq!(UiState::default().normalized(), UiState::default());
}

#[test]
fn ui_state_normalization_keeps_hidden_columns_hidden() {
    let state = load("[[columns]]\nid = \"eta\"\nvisible = false\nwidth = 100.0\n");

    let eta = state.columns.first().expect("the eta column is kept first");
    assert_eq!(*eta, column(ColumnId::Eta, false, 100.0));
    assert_eq!(state.columns.len(), ColumnId::ALL.len());
}

#[test]
fn ui_state_an_empty_column_list_becomes_the_default_columns() {
    let state = load("columns = []\n");

    assert_eq!(state.columns, default_columns());
}

#[test]
fn ui_state_normalization_repairs_widths_that_are_not_numbers() {
    let mut state = UiState::default();
    for (position, width) in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY]
        .into_iter()
        .enumerate()
    {
        if let Some(column) = state.columns.get_mut(position) {
            column.width = width;
        }
    }

    let state = state.normalized();

    assert_eq!(state.columns, default_columns());
}

#[test]
fn ui_state_normalizes_the_window_size() {
    let too_small = [
        (399.0, 700.0),
        (1100.0, 299.0),
        (10.0, 10.0),
        (-5.0, -5.0),
        (f32::NAN, 700.0),
        (1100.0, f32::INFINITY),
    ];
    for (width, height) in too_small {
        let state = UiState {
            window: WindowState {
                width,
                height,
                maximized: true,
            },
            ..UiState::default()
        }
        .normalized();

        assert_eq!(
            state.window,
            WindowState {
                maximized: true,
                ..WindowState::default()
            },
            "{width} x {height}"
        );
    }

    let minimum = UiState {
        window: WindowState {
            width: 400.0,
            height: 300.0,
            maximized: false,
        },
        ..UiState::default()
    };
    assert_eq!(minimum.clone().normalized(), minimum);
}

#[test]
fn ui_state_newer_version_keeps_defaults() {
    let text = r#"
        version = 99
        filter = "laser"

        [window]
        width = 900.0
        height = 650.0
    "#;

    let parsed = UiState::parse(text).expect("a newer file is not an error");

    assert_eq!(
        parsed,
        ParsedUiState {
            state: UiState::default(),
            newer: true
        }
    );
}

#[test]
fn ui_state_newer_version_is_recognized_whatever_else_it_holds() {
    // A newer format may change a type this version knows; that must not read as corruption.
    let text = "version = 2\n[window]\nwidth = \"wide\"\n[[columns]]\nid = 7\n";

    let parsed = UiState::parse(text).expect("a newer file is not an error");

    assert!(parsed.newer);
    assert_eq!(parsed.state, UiState::default());

    let huge = UiState::parse("version = 5000000000\n").expect("not an error");
    assert!(huge.newer);
}

#[test]
fn ui_state_current_and_older_versions_are_read() {
    let current = UiState::parse("version = 1\n[window]\nmaximized = true\n").expect("valid");
    assert!(!current.newer);
    assert!(current.state.window.maximized);

    // Nothing was ever written below 1, and nothing differs yet, so it is read as the current one.
    let older = UiState::parse("version = 0\n[window]\nmaximized = true\n").expect("valid");
    assert!(!older.newer);
    assert!(older.state.window.maximized);
    assert_eq!(older.state.version, UI_STATE_VERSION);
}

#[test]
fn ui_state_wrong_type_is_an_error() {
    let wrong = [
        "[window]\nwidth = \"wide\"\n",
        "[window]\nmaximized = 1\n",
        "version = \"one\"\n",
        "version = -1\n",
        "filter = 5\n",
        "theme = true\n",
        "[sort]\nkey = 3\n",
        "columns = \"all\"\n",
        "[[columns]]\nid = 7\n",
        "[[columns]]\nid = \"name\"\nwidth = \"wide\"\n",
        "window = 4\n",
    ];

    for text in wrong {
        assert!(
            matches!(UiState::parse(text), Err(UiStateError::Parse(_))),
            "{text:?}"
        );
    }
}

#[test]
fn ui_state_broken_toml_is_an_error() {
    for text in ["[window\n", "width = = 3", "\"unterminated"] {
        assert!(
            matches!(UiState::parse(text), Err(UiStateError::Parse(_))),
            "{text:?}"
        );
    }
}

#[test]
fn ui_state_parse_error_names_the_position_but_not_the_file() {
    let text = "version = 1\n\n[window]\nwidth = \"wide\"\n";

    let Err(UiStateError::Parse(message)) = UiState::parse(text) else {
        panic!("a string is not a width");
    };

    assert!(message.starts_with("line 4, column 9: "), "{message}");
    assert!(!message.contains("[window]"), "{message}");
    assert!(!message.contains('\n'), "{message}");
    let shown = UiStateError::Parse(message).to_string();
    assert!(
        shown.starts_with("cannot parse the UI state: line 4"),
        "{shown}"
    );
}

#[test]
fn ui_state_serialize_error_says_so() {
    let error = UiStateError::Serialize("boom".to_owned());

    assert_eq!(error.to_string(), "cannot serialize the UI state: boom");
}

#[test]
fn ui_state_view_state_holds_the_filter_and_the_sort_with_an_empty_search() {
    let view = customized().view_state();

    assert_eq!(
        view,
        ViewState {
            filter: Filter::Downloading,
            search: String::new(),
            sort_key: SortKey::DownloadSpeed,
            sort_direction: SortDirection::Descending,
        }
    );
    assert_eq!(UiState::default().view_state(), ViewState::default());
}

#[test]
fn ui_state_keeps_the_order_of_the_columns_the_user_chose() {
    let state = load("");
    let mut reordered = state.clone();
    reordered.columns.reverse();

    let text = reordered.to_toml().expect("the state can be written");

    assert_eq!(ids(&load(&text).columns), ids(&reordered.columns));
}
