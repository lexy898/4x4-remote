use std::borrow::Cow;
use std::collections::HashMap;

use i18n_embed::unic_langid::LanguageIdentifier;
use x4_core::{
    CompactDuration, ConnectionError, ConnectionState, DisplayState, EtaView, HumanSize, QueueKind,
    Ratio, SizeUnit, Subtitle, TimeUnit, UnsupportedReason, human_size, human_speed,
};

use super::*;

/// Message files held in memory, by path such as `ru/x4.ftl`.
struct MemoryAssets(HashMap<&'static str, &'static str>);

impl I18nAssets for MemoryAssets {
    fn get_files(&self, file_path: &str) -> Vec<Cow<'_, [u8]>> {
        self.0
            .get(file_path)
            .map(|text| Cow::Borrowed(text.as_bytes()))
            .into_iter()
            .collect()
    }

    fn filenames_iter(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.0.keys().map(|path| (*path).to_owned()))
    }
}

fn eta(major: (u64, TimeUnit), minor: Option<(u64, TimeUnit)>) -> EtaView {
    EtaView::In(CompactDuration { major, minor })
}

#[test]
fn size_and_speed_texts() {
    assert_eq!(size_text(human_size(24_592_841_836)), "22.9 GiB");
    assert_eq!(size_text(human_size(0)), "0 B");
    assert_eq!(size_text(human_size(1536)), "1.5 KiB");
    assert_eq!(size_text(human_size(150 * 1024 * 1024)), "150 MiB");
    assert_eq!(size_text(human_size(3 * 1024_u64.pow(4))), "3.0 TiB");
    assert_eq!(speed_text(human_speed(184_320)), "180 KiB/s");
    assert_eq!(speed_text(human_speed(0)), "0 B/s");
    assert_eq!(speed_text(human_speed(1_572_864)), "1.5 MiB/s");
    assert_eq!(speed_text(human_speed(2 * 1024_u64.pow(3))), "2.0 GiB/s");
}

#[test]
fn progress_text_joins_the_percent_and_the_size() {
    assert_eq!(
        progress_text(63, human_size(24_592_841_836)),
        "63% of 22.9 GiB"
    );
    assert_eq!(progress_text(0, human_size(0)), "0% of 0 B");
}

#[test]
fn decimal_rounds_to_the_digits_asked_for() {
    assert_eq!(decimal(22.94, 1), "22.9");
    assert_eq!(decimal(0.0, 2), "0.00");
    assert_eq!(decimal(180.0, 0), "180");
    assert_eq!(decimal(1.0, 2), "1.00");
}

#[test]
fn the_separator_replaces_only_the_decimal_point() {
    assert_eq!(with_separator(22.9, 1, ","), "22,9");
    assert_eq!(with_separator(1234.5, 2, ","), "1234,50");
    assert_eq!(with_separator(180.0, 0, ","), "180");
}

#[test]
fn ratio_texts() {
    assert_eq!(ratio_text(Ratio::Value(1.5)), "1.50");
    assert_eq!(ratio_text(Ratio::Value(0.0)), "0.00");
    assert_eq!(ratio_text(Ratio::Value(12.3456)), "12.35");
    assert_eq!(ratio_text(Ratio::Infinite), "∞");
    assert_eq!(ratio_text(Ratio::NotAvailable), "—");
}

#[test]
fn eta_texts() {
    assert_eq!(eta_text(&EtaView::None), "—");
    assert_eq!(eta_text(&EtaView::Unknown), "∞");
    assert_eq!(
        eta_text(&eta((1, TimeUnit::Days), Some((2, TimeUnit::Hours)))),
        "1 d 2 h"
    );
    assert_eq!(eta_text(&eta((5, TimeUnit::Minutes), None)), "5 m");
    assert_eq!(
        eta_text(&eta((3, TimeUnit::Hours), Some((15, TimeUnit::Minutes)))),
        "3 h 15 m"
    );
    assert_eq!(eta_text(&eta((59, TimeUnit::Seconds), None)), "59 s");
}

#[test]
fn subtitle_texts_use_plurals() {
    let seeding = |to_peers| subtitle_text(&Subtitle::Seeding { to_peers });
    assert_eq!(seeding(0), "Seeding");
    assert_eq!(seeding(1), "Seeding · 1 peer downloading from you");
    assert_eq!(seeding(3), "Seeding · 3 peers downloading from you");

    let downloading = |from_peers, connected| {
        subtitle_text(&Subtitle::Downloading {
            from_peers,
            connected,
        })
    };
    assert_eq!(downloading(0, 0), "Downloading");
    assert_eq!(downloading(1, 1), "Downloading · from 1 of 1 peer");
    assert_eq!(downloading(0, 5), "Downloading · from 0 of 5 peers");
    assert_eq!(downloading(3, 12), "Downloading · from 3 of 12 peers");
}

#[test]
fn the_other_subtitles() {
    let text = |subtitle| subtitle_text(&subtitle);
    assert_eq!(text(Subtitle::Paused), "Paused");
    assert_eq!(
        text(Subtitle::Queued(QueueKind::Download)),
        "Queued to download"
    );
    assert_eq!(text(Subtitle::Queued(QueueKind::Seed)), "Queued to seed");
    assert_eq!(
        text(Subtitle::Checking { percent: 63 }),
        "Checking files · 63%"
    );
    assert_eq!(
        text(Subtitle::FetchingMetadata { percent: 5 }),
        "Fetching metadata · 5%"
    );
    assert_eq!(
        text(Subtitle::Error("No space left on device".to_owned())),
        "Error: No space left on device"
    );
    assert_eq!(
        text(Subtitle::TrackerWarning("Tracker is busy".to_owned())),
        "Tracker warning: Tracker is busy"
    );
    assert_eq!(
        text(Subtitle::TrackerError("Torrent not registered".to_owned())),
        "Tracker error: Torrent not registered"
    );
}

#[test]
fn error_subtitle_without_text_is_generic() {
    let generic = |subtitle: Subtitle| {
        let text = subtitle_text(&subtitle);
        assert!(!text.ends_with(':') && !text.ends_with(": "), "{text:?}");
        text
    };
    assert_eq!(
        generic(Subtitle::Error(String::new())),
        "Error: no details from the server"
    );
    assert_eq!(
        generic(Subtitle::TrackerWarning(String::new())),
        "Tracker warning: no details from the server"
    );
    assert_eq!(
        generic(Subtitle::TrackerError(String::new())),
        "Tracker error: no details from the server"
    );
    // Text of nothing but spaces says nothing either.
    assert_eq!(
        generic(Subtitle::Error("  \n ".to_owned())),
        "Error: no details from the server"
    );
}

#[test]
fn state_texts() {
    assert_eq!(state_text(DisplayState::Downloading), "Downloading");
    assert_eq!(state_text(DisplayState::Seeding), "Seeding");
    assert_eq!(state_text(DisplayState::Paused), "Paused");
    assert_eq!(
        state_text(DisplayState::Queued(QueueKind::Download)),
        "Queued"
    );
    assert_eq!(state_text(DisplayState::Queued(QueueKind::Seed)), "Queued");
    assert_eq!(state_text(DisplayState::Checking), "Checking");
    assert_eq!(state_text(DisplayState::Error), "Error");
}

#[test]
fn connection_texts() {
    let text = |state| connection_text(&state);
    assert_eq!(text(ConnectionState::Connecting), "Connecting…");
    assert_eq!(text(ConnectionState::Connected), "");
    assert_eq!(
        text(ConnectionState::Unsupported {
            reason: UnsupportedReason::Legacy
        }),
        "Transmission 4.1 or newer required"
    );
    assert_eq!(
        text(ConnectionState::Unsupported {
            reason: UnsupportedReason::NoVersion
        }),
        "Transmission 4.1 or newer required (the server reports no version)"
    );
    assert_eq!(
        text(ConnectionState::Unsupported {
            reason: UnsupportedReason::Version("5.0.1".to_owned())
        }),
        "Transmission 4.1 or newer required (server RPC 5.0.1)"
    );
}

#[test]
fn reconnecting_says_why_in_a_few_words() {
    let reconnecting = |attempt, last_error| {
        connection_text(&ConnectionState::Reconnecting {
            attempt,
            last_error,
        })
    };
    assert_eq!(
        reconnecting(1, ConnectionError::Unreachable),
        "Reconnecting (attempt 1)… The server cannot be reached"
    );
    assert_eq!(
        reconnecting(12, ConnectionError::Timeout),
        "Reconnecting (attempt 12)… The server did not answer in time"
    );
    let reasons: Vec<SharedString> = [
        ConnectionError::Unreachable,
        ConnectionError::Timeout,
        ConnectionError::Forbidden,
        ConnectionError::Unauthorized,
        ConnectionError::Protocol("odd".to_owned()),
        ConnectionError::Server("odd".to_owned()),
    ]
    .into_iter()
    .map(|error| reconnecting(2, error))
    .collect();
    for (index, reason) in reasons.iter().enumerate() {
        assert!(
            !reasons.iter().skip(index + 1).any(|other| other == reason),
            "two kinds of error read the same: {reason:?}"
        );
    }
}

#[test]
fn the_details_of_protocol_and_server_errors_stay_out_of_the_status_text() {
    for error in [
        ConnectionError::Protocol("secret detail".to_owned()),
        ConnectionError::Server("secret detail".to_owned()),
    ] {
        let text = connection_text(&ConnectionState::Reconnecting {
            attempt: 1,
            last_error: error,
        });
        assert!(!text.contains("secret"), "{text:?}");
    }
}

#[test]
fn texts_carry_no_invisible_isolation_marks() {
    let texts = [
        subtitle_text(&Subtitle::Error("disk full".to_owned())),
        subtitle_text(&Subtitle::Seeding { to_peers: 3 }),
        progress_text(63, human_size(24_592_841_836)),
        eta_text(&eta((1, TimeUnit::Days), Some((2, TimeUnit::Hours)))),
        connection_text(&ConnectionState::Unsupported {
            reason: UnsupportedReason::Version("5.0.1".to_owned()),
        }),
    ];
    for text in texts {
        assert!(!text.contains(['\u{2068}', '\u{2069}']), "{text:?}");
    }
}

#[test]
fn a_size_of_the_largest_unit_keeps_growing() {
    let size = HumanSize {
        value: 2048.0,
        unit: SizeUnit::TiB,
    };
    assert_eq!(size_text(size), "2048 TiB");
}

#[test]
fn a_loader_without_messages_starts_and_shows_placeholders() {
    let loader = load(&MemoryAssets(HashMap::new()));
    let text = loader.get("unit-gib");
    assert!(text.contains("unit-gib"), "{text:?}");
}

#[test]
fn a_loader_with_a_broken_message_file_still_starts() {
    let assets = MemoryAssets(HashMap::from([(
        "en/x4.ftl",
        "unit-gib = GiB\nbroken = { $\n",
    )]));
    let loader = load(&assets);
    assert_eq!(loader.get("unit-gib"), "GiB");
}

/// Another language is one more file: its messages win, the ones it lacks fall back to English,
/// and the plural rules of its language pick the wording.
#[test]
fn a_second_language_is_one_more_file() {
    let assets = MemoryAssets(HashMap::from([
        (
            "en/x4.ftl",
            "files = { $count ->\n    [one] { $count } file\n   *[other] { $count } files\n}\nonly-english = Hello\n",
        ),
        (
            "ru/x4.ftl",
            "files = { $count ->\n    [one] { $count } файл\n    [few] { $count } файла\n    [many] { $count } файлов\n   *[other] { $count } файла\n}\n",
        ),
    ]));
    let loader = load(&assets);
    let russian: LanguageIdentifier = "ru".parse().expect("ru is a language code");
    let english = loader.fallback_language().clone();
    loader
        .load_languages(&assets, &[russian, english])
        .expect("both languages load");
    loader.set_use_isolating(false);

    let files =
        |count: u32| loader.get_args_concrete("files", HashMap::from([("count", count.into())]));
    assert_eq!(files(1), "1 файл");
    assert_eq!(files(3), "3 файла");
    assert_eq!(files(5), "5 файлов");
    assert_eq!(files(21), "21 файл");
    assert_eq!(loader.get("only-english"), "Hello");
}
