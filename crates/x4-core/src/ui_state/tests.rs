use super::*;

/// Parses `text`, which must be valid.
fn load(text: &str) -> UiState {
    UiState::parse(text).expect("the text is valid").state
}

fn column(id: ColumnId, visible: bool, width: f32) -> ColumnState {
    ColumnState { id, visible, width }
}

/// `first`, followed by the default of every column that `first` does not hold: what `parse`
/// makes of a file that lists only `first`.
fn completed(first: &[ColumnState]) -> Vec<ColumnState> {
    let mut columns = first.to_vec();
    columns.extend(
        default_columns()
            .into_iter()
            .filter(|default| first.iter().all(|given| given.id != default.id)),
    );
    columns
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
        details: DetailsState {
            open: false,
            height: 333.3,
            tab: DetailsTab::Files,
        },
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
            width: 1200.0,
            height: 720.0,
            maximized: false
        }
    );
    assert_eq!(state.columns, default_columns());
    assert_eq!(state.sort.key, SortKey::Name);
    assert_eq!(state.sort.direction, SortDirection::Ascending);
    assert_eq!(state.filter, Filter::All);
    assert_eq!(state.theme, ThemeChoice::System);
    assert_eq!(
        state.details,
        DetailsState {
            open: true,
            height: 240.0,
            tab: DetailsTab::General
        }
    );
}

#[test]
fn default_columns_show_everything_at_their_widths() {
    let expected = [
        (ColumnId::Name, 285.0),
        (ColumnId::Size, 80.0),
        (ColumnId::Progress, 120.0),
        (ColumnId::State, 105.0),
        (ColumnId::Down, 88.0),
        (ColumnId::Up, 88.0),
        (ColumnId::Eta, 82.0),
        (ColumnId::Ratio, 64.0),
        (ColumnId::Peers, 66.0),
    ]
    .map(|(id, width)| column(id, true, width));

    assert_eq!(default_columns(), expected);
    let total: f32 = expected.iter().map(|column| column.width).sum();
    assert!((total - 978.0).abs() < f32::EPSILON, "{total}");
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
width = 1200.0
height = 720.0
maximized = false

[[columns]]
id = "name"
visible = true
width = 285.0

[[columns]]
id = "size"
visible = true
width = 80.0

[[columns]]
id = "progress"
visible = true
width = 120.0

[[columns]]
id = "state"
visible = true
width = 105.0

[[columns]]
id = "down"
visible = true
width = 88.0

[[columns]]
id = "up"
visible = true
width = 88.0

[[columns]]
id = "eta"
visible = true
width = 82.0

[[columns]]
id = "ratio"
visible = true
width = 64.0

[[columns]]
id = "peers"
visible = true
width = 66.0

[sort]
key = "name"
direction = "ascending"

[details]
open = true
height = 240.0
tab = "general"
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

    let text = UiState {
        details: DetailsState {
            tab: DetailsTab::Files,
            ..DetailsState::default()
        },
        ..UiState::default()
    }
    .to_toml()
    .expect("the state can be written");

    assert!(text.contains("tab = \"files\""), "{text}");
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
    assert_eq!(
        parsed.state.columns,
        completed(&[column(ColumnId::Name, true, 300.0)])
    );
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
    // The unknown column is gone, the known ones are in the order of the file.
    assert_eq!(
        state.columns,
        completed(&[
            column(ColumnId::Name, true, 300.0),
            column(ColumnId::Size, false, 95.0)
        ])
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
    assert_eq!(
        state.columns,
        completed(&[column(ColumnId::Ratio, true, 64.0)])
    );
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
    assert_eq!(
        state.columns,
        completed(&[column(ColumnId::Name, true, 250.0)])
    );
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
            column(ColumnId::Size, true, 80.0),
            column(ColumnId::Progress, true, 120.0),
            column(ColumnId::State, true, 105.0),
            column(ColumnId::Down, true, 88.0),
            column(ColumnId::Up, true, 88.0),
            column(ColumnId::Eta, true, 82.0),
            column(ColumnId::Ratio, true, 64.0),
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
    let unusable = [
        (399.0, 700.0),
        (1100.0, 299.0),
        (10.0, 10.0),
        (-5.0, -5.0),
        (f32::NAN, 700.0),
        (1100.0, f32::NAN),
        (f32::INFINITY, 700.0),
        (1100.0, f32::INFINITY),
        (f32::NEG_INFINITY, 700.0),
        (16385.0, 700.0),
        (1100.0, 16385.0),
        (1.0e9, 1.0e9),
    ];
    for (width, height) in unusable {
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

    // The limits themselves are fine.
    for (width, height) in [(400.0, 300.0), (16384.0, 16384.0), (400.0, 16384.0)] {
        let edge = UiState {
            window: WindowState {
                width,
                height,
                maximized: false,
            },
            ..UiState::default()
        };
        assert_eq!(edge.clone().normalized(), edge, "{width} x {height}");
    }
}

#[test]
fn a_window_size_alone_is_normalized_like_a_whole_state() {
    let huge = WindowState {
        width: 20000.0,
        height: 700.0,
        maximized: true,
    };
    let usable = WindowState {
        width: 1600.0,
        height: 900.0,
        maximized: false,
    };

    assert_eq!(
        huge.normalized(),
        WindowState {
            maximized: true,
            ..WindowState::default()
        }
    );
    assert_eq!(usable.normalized(), usable);
    assert_eq!(
        huge.normalized(),
        UiState {
            window: huge,
            ..UiState::default()
        }
        .normalized()
        .window
    );
}

#[test]
fn ui_state_parse_returns_a_normalized_state() {
    let text = r#"
        [window]
        width = nan
        height = 700.0
        maximized = true

        [[columns]]
        id = "peers"
        visible = true
        width = 5000.0

        [[columns]]
        id = "name"
        visible = false
        width = 360.0

        [[columns]]
        id = "peers"
        visible = false
        width = 100.0

        [[columns]]
        id = "eta"
        width = inf
    "#;

    let parsed = UiState::parse(text).expect("a valid file");

    assert!(!parsed.newer);
    assert_eq!(
        parsed.state.window,
        WindowState {
            maximized: true,
            ..WindowState::default()
        }
    );
    assert_eq!(
        parsed.state.columns,
        [
            column(ColumnId::Peers, true, 1200.0),
            column(ColumnId::Name, true, 360.0),
            column(ColumnId::Eta, true, 82.0),
            column(ColumnId::Size, true, 80.0),
            column(ColumnId::Progress, true, 120.0),
            column(ColumnId::State, true, 105.0),
            column(ColumnId::Down, true, 88.0),
            column(ColumnId::Up, true, 88.0),
            column(ColumnId::Ratio, true, 64.0),
        ]
    );
    assert_eq!(parsed.state.clone().normalized(), parsed.state);
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

#[test]
fn ui_state_a_sort_with_only_a_key_keeps_the_default_direction() {
    let state = load("[sort]\nkey = \"ratio\"\n");

    assert_eq!(
        state.sort,
        SortState {
            key: SortKey::Ratio,
            direction: SortDirection::Ascending
        }
    );
}

#[test]
fn ui_state_a_sort_with_only_a_direction_keeps_the_default_key() {
    let state = load("[sort]\ndirection = \"descending\"\n");

    assert_eq!(
        state.sort,
        SortState {
            key: SortKey::Name,
            direction: SortDirection::Descending
        }
    );
}

#[test]
fn ui_state_writes_numbers_that_are_not_finite_and_reads_them_back_repaired() {
    let mut state = customized();
    state.window.width = f32::NAN;
    state.window.height = f32::INFINITY;
    for (column, width) in
        state
            .columns
            .iter_mut()
            .zip([f32::NAN, f32::INFINITY, f32::NEG_INFINITY])
    {
        column.width = width;
    }

    let text = state.to_toml().expect("non-finite numbers can be written");
    let parsed = UiState::parse(&text).expect("and read");

    assert_eq!(parsed.state, state.normalized(), "{text}");
}

#[test]
fn ui_state_parse_errors_never_show_the_values_of_the_file() {
    let secret = "hunter2";
    let texts = [
        format!("[window]\nwidth = \"{secret}\"\n"),
        format!("version = \"{secret}\"\n"),
        format!("[[columns]]\nid = \"name\"\nwidth = \"{secret}\"\n"),
        format!("[[columns]]\nid = \"{secret}\"\nvisible = \"{secret}\"\n"),
        format!("[sort]\nkey = \"size\"\ndirection = [\"{secret}\"]\n"),
        format!("filter = 5\nwindow = \"{secret}\"\n"),
        format!("window = \"{secret}\"\n"),
    ];

    for text in texts {
        let Err(error) = UiState::parse(&text) else {
            panic!("{text}: a wrong type is an error");
        };

        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(!shown.contains(secret), "{text}: {shown}");
            assert!(shown.contains("line "), "{text}: {shown}");
        }
    }
}

#[test]
fn logged_values_from_the_file_are_cut_short() {
    use super::wire::shortened;

    assert_eq!(shortened("laser"), "laser");
    assert_eq!(shortened(""), "");
    let exactly: String = "a".repeat(32);
    assert_eq!(shortened(&exactly), exactly);
    let long = "é".repeat(100);
    assert_eq!(shortened(&long), format!("{}…", "é".repeat(32)));
}

#[test]
fn old_state_files_get_default_details() {
    // A file written before the details panel existed has no `[details]` table.
    let old = r#"
        version = 1
        filter = "downloading"
        theme = "dark"

        [window]
        width = 900.0
        height = 650.0
        maximized = true

        [sort]
        key = "size"
        direction = "descending"
    "#;

    let parsed = UiState::parse(old).expect("an old file is valid");

    assert!(!parsed.newer);
    assert_eq!(parsed.state.details, DetailsState::default());
    assert_eq!(
        parsed.state.details,
        DetailsState {
            open: true,
            height: 240.0,
            tab: DetailsTab::General
        }
    );
    assert_eq!(parsed.state.filter, Filter::Downloading, "the rest is read");
    assert_eq!(
        UiState::parse("").expect("empty").state.details,
        DetailsState::default()
    );
}

#[test]
fn details_state_round_trips() {
    for details in [
        DetailsState::default(),
        DetailsState {
            open: false,
            height: 333.5,
            tab: DetailsTab::Files,
        },
        DetailsState {
            open: true,
            height: MIN_DETAILS_HEIGHT,
            tab: DetailsTab::General,
        },
        DetailsState {
            open: false,
            height: MAX_DETAILS_HEIGHT,
            tab: DetailsTab::Files,
        },
    ] {
        let state = UiState {
            details,
            ..UiState::default()
        };
        let text = state.to_toml().expect("the state can be written");

        let parsed = UiState::parse(&text).expect("what was written can be read");

        assert_eq!(parsed.state.details, details, "{text}");
        assert_eq!(parsed.state, state, "{text}");
    }
    // The tabs are written by their snake_case names.
    for (tab, name) in [
        (DetailsTab::General, "general"),
        (DetailsTab::Files, "files"),
    ] {
        let text = format!("[details]\ntab = \"{name}\"\n");
        assert_eq!(load(&text).details.tab, tab, "{name}");
    }
}

#[test]
fn details_height_is_clamped() {
    assert!((MIN_DETAILS_HEIGHT - 110.0).abs() < f32::EPSILON);
    assert!((MAX_DETAILS_HEIGHT - 2000.0).abs() < f32::EPSILON);
    for (given, expected) in [
        (0.0, 110.0),
        (-30.0, 110.0),
        (100.0, 110.0),
        (109.9, 110.0),
        (110.0, 110.0),
        (240.0, 240.0),
        (1999.0, 1999.0),
        (2000.0, 2000.0),
        (2000.1, 2000.0),
        (1.0e9, 2000.0),
        (f32::INFINITY, 240.0),
        (f32::NEG_INFINITY, 240.0),
        (f32::NAN, 240.0),
    ] {
        let state = UiState {
            details: DetailsState {
                height: given,
                ..DetailsState::default()
            },
            ..UiState::default()
        };

        let normalized = state.clone().normalized();

        assert!(
            (normalized.details.height - expected).abs() < f32::EPSILON,
            "{given} became {}",
            normalized.details.height
        );
        // Reading the written file repairs it the same way.
        let text = state.to_toml().expect("non-finite numbers can be written");
        let parsed = UiState::parse(&text).expect("and read").state;
        assert_eq!(parsed.details, normalized.details, "{given}: {text}");
    }
    let hand_edited = load("[details]\nheight = 5\n");
    assert!((hand_edited.details.height - 110.0).abs() < f32::EPSILON);
    // The other settings of the panel stay as they were.
    let kept = UiState {
        details: DetailsState {
            open: false,
            height: 9000.0,
            tab: DetailsTab::Files,
        },
        ..UiState::default()
    }
    .normalized();
    assert_eq!(
        kept.details,
        DetailsState {
            open: false,
            height: 2000.0,
            tab: DetailsTab::Files
        }
    );
}

#[test]
fn an_unknown_details_tab_falls_back_to_the_default() {
    let text = "[details]\nopen = false\nheight = 300.0\ntab = \"peers\"\n";

    let state = UiState::parse(text)
        .expect("an unknown tab does not fail the file")
        .state;

    assert_eq!(
        state.details,
        DetailsState {
            open: false,
            height: 300.0,
            tab: DetailsTab::General
        },
        "the neighbors of the unknown tab stay"
    );
    // Missing keys take their defaults one by one.
    assert_eq!(
        load("[details]\nopen = false\n").details,
        DetailsState {
            open: false,
            ..DetailsState::default()
        }
    );
    assert_eq!(
        load("[details]\ntab = \"files\"\nfuture = 1\n").details,
        DetailsState {
            tab: DetailsTab::Files,
            ..DetailsState::default()
        },
        "unknown keys are ignored"
    );
}

#[test]
fn details_of_the_wrong_type_are_an_error() {
    for text in [
        "[details]\nopen = 1\n",
        "[details]\nheight = \"tall\"\n",
        "[details]\ntab = 3\n",
        "details = \"open\"\n",
    ] {
        assert!(
            matches!(UiState::parse(text), Err(UiStateError::Parse(_))),
            "{text}"
        );
    }
}

#[test]
fn the_panel_fits_the_room_the_window_leaves_it() {
    let at = |height: f32| DetailsState {
        height,
        ..DetailsState::default()
    };
    let fitted = |state: DetailsState, room: f32| state.fitted_height(room);

    // It fits: the remembered height.
    assert!((fitted(at(240.0), 500.0) - 240.0).abs() < f32::EPSILON);
    assert!((fitted(at(240.0), 240.0) - 240.0).abs() < f32::EPSILON);
    // Too high for the room: as high as the room.
    assert!((fitted(at(2000.0), 500.0) - 500.0).abs() < f32::EPSILON);
    assert!((fitted(at(240.0), 150.5) - 150.5).abs() < f32::EPSILON);
    // Less room than the minimum, or none, or a room that is not a number: the minimum.
    for room in [109.0, 0.0, -50.0, f32::NAN] {
        assert!(
            (fitted(at(240.0), room) - MIN_DETAILS_HEIGHT).abs() < f32::EPSILON,
            "{room}"
        );
    }
    // Infinite room: the remembered height.
    assert!((fitted(at(333.0), f32::INFINITY) - 333.0).abs() < f32::EPSILON);
}
