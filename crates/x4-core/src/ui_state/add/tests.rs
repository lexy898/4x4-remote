use x4_rpc::MAX_DOWNLOAD_DIR_BYTES;

use super::*;
use crate::ui_state::{UiState, UiStateError};

fn dir(path: &str) -> DownloadDir {
    DownloadDir::new(path).unwrap()
}

/// The history of a state file whose `[add]` table lists `folders`.
fn loaded(folders: &[String]) -> Vec<String> {
    let state = AddState {
        recent_folders: folders.to_vec(),
    };
    let text = UiState {
        add: state,
        ..UiState::default()
    }
    .to_toml()
    .unwrap();
    UiState::parse(&text).unwrap().state.add.recent_folders
}

fn strings(folders: &[&str]) -> Vec<String> {
    folders.iter().map(|&folder| folder.to_owned()).collect()
}

#[test]
fn the_history_is_normalized_on_load_and_capped() {
    assert_eq!(MAX_RECENT_FOLDERS, 10);
    let too_long = format!("/{}", "a".repeat(MAX_DOWNLOAD_DIR_BYTES));
    let folders = strings(&[
        "/data/films/",
        "//data//music",
        "/data/films",
        "relative/path",
        "~/torrents",
        "",
        "/data/../etc",
        "/data/a\u{7}b",
        &too_long,
        "/data/Films",
        "\t/data/books\n",
        "/",
    ]);

    assert_eq!(
        loaded(&folders),
        [
            "/data/films",
            "/data/music",
            "/data/Films",
            "/data/books",
            "/",
        ],
        "absolute, normalized, each once (the first counts), and in their order"
    );

    // At most ten, the newest.
    let many: Vec<String> = (0..15).map(|n| format!("/f/{n}")).collect();
    assert_eq!(loaded(&many), many.get(..10).unwrap());
    // Repeats and bad entries do not take the room of good ones.
    let mut crowded = strings(&["/f/0", "/f/0", "bad", "/f/0/"]);
    crowded.extend(many.iter().skip(1).cloned());
    assert_eq!(loaded(&crowded), many.get(..10).unwrap());
    // The longest folder the daemon takes is kept.
    let longest = format!("/{}", "a".repeat(MAX_DOWNLOAD_DIR_BYTES - 1));
    assert_eq!(loaded(std::slice::from_ref(&longest)), [longest]);
}

#[test]
fn a_file_without_a_history_has_an_empty_one() {
    let old = "version = 1\n\n[details]\nopen = false\n";
    assert_eq!(UiState::parse(old).unwrap().state.add, AddState::default());
    assert_eq!(UiState::parse("").unwrap().state.add, AddState::default());
    assert_eq!(AddState::default().recent_folders, Vec::<String>::new());

    // Unknown keys are ignored, a missing list is empty, and the rest of the file is read.
    let text = "theme = \"dark\"\n[add]\nfuture = 1\n";
    let state = UiState::parse(text).unwrap().state;
    assert_eq!(state.add, AddState::default());
    assert_eq!(state.theme, crate::ui_state::ThemeChoice::Dark);

    // Hand-written folders are read as the dialog writes them.
    let text = "[add]\nrecent_folders = [\"/data/films/\", \"/data/films\", \"films\"]\n";
    assert_eq!(
        UiState::parse(text).unwrap().state.add.recent_folders,
        ["/data/films"]
    );
}

#[test]
fn a_history_of_the_wrong_type_is_an_error() {
    for text in [
        "[add]\nrecent_folders = \"/data\"\n",
        "[add]\nrecent_folders = [1, 2]\n",
        "add = 3\n",
    ] {
        assert!(
            matches!(UiState::parse(text), Err(UiStateError::Parse(_))),
            "{text}"
        );
    }
}

#[test]
fn a_remembered_folder_comes_first_once_and_the_history_keeps_ten() {
    let mut state = AddState::default();

    state.remember(&dir("/data/films"));
    state.remember(&dir("/data/music"));
    assert_eq!(state.recent_folders, ["/data/music", "/data/films"]);

    // Again: it moves to the front instead of being listed twice.
    state.remember(&dir("/data/films/"));
    assert_eq!(state.recent_folders, ["/data/films", "/data/music"]);

    for n in 0..12 {
        state.remember(&dir(&format!("/f/{n}")));
    }
    let expected: Vec<String> = (2..12).rev().map(|n| format!("/f/{n}")).collect();
    assert_eq!(state.recent_folders, expected);

    // What is remembered is written and read back as it is.
    let text = UiState {
        add: state.clone(),
        ..UiState::default()
    }
    .to_toml()
    .unwrap();
    assert_eq!(UiState::parse(&text).unwrap().state.add, state);
}
