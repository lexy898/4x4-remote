//! The torrent as the rest of the application sees it, built from the server's raw record.

use std::time::{Duration, SystemTime};

use x4_rpc::{ErrorKind, HashString, TorrentId, TorrentRecord};

use crate::state::{Attention, DisplayState, attention, classify, is_active};

/// How long a torrent needs to finish, from the wire `eta`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Eta {
    /// There is no estimate: the torrent is not downloading, or is complete (wire `-1`).
    NotAvailable,
    /// The server cannot tell yet (wire `-2`, or any other negative value).
    Unknown,
    /// Finished in this long at the current rate (wire `0` or more, in seconds).
    In(Duration),
}

/// Uploaded divided by downloaded, from the wire `upload_ratio`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ratio {
    /// Nothing to compare yet (wire `-1`, or any other invalid value).
    NotAvailable,
    /// Uploaded data but downloaded none (wire `-2`; a positive infinity also maps here).
    Infinite,
    /// A finite ratio, 0 or more.
    Value(f64),
}

/// A torrent, with the server's raw codes turned into display values.
///
/// Sizes are bytes, rates are bytes per second and fields the server did not send hold a neutral
/// default (zero, empty, [`Eta::NotAvailable`], no date). Identify a torrent by `hash`; `id`
/// changes when the daemon restarts.
#[derive(Debug, Clone, PartialEq)]
pub struct Torrent {
    /// The daemon's numeric id; valid only until the daemon restarts.
    pub id: TorrentId,
    /// The stable identity.
    pub hash: HashString,
    /// The name as the server reports it.
    pub name: String,
    /// The state the torrent is shown in.
    pub state: DisplayState,
    /// A reason to draw the user's eye to the torrent, if any.
    pub attention: Option<Attention>,
    /// Whether data moves right now.
    pub active: bool,
    /// Progress of the wanted files, 0.0 to 1.0.
    pub progress: f64,
    /// Progress of fetching the metadata of a magnet link, 0.0 to 1.0; 1.0 when unknown.
    pub metadata_progress: f64,
    /// Size of all files.
    pub total_size: u64,
    /// Size of the wanted files.
    pub size_when_done: u64,
    /// Bytes of the wanted files still missing.
    pub left_until_done: u64,
    /// Download rate.
    pub rate_down: u64,
    /// Upload rate.
    pub rate_up: u64,
    /// Time to finish.
    pub eta: Eta,
    /// Share ratio.
    pub ratio: Ratio,
    /// Connected peers.
    pub peers_connected: u32,
    /// Peers we upload to.
    pub peers_getting_from_us: u32,
    /// Peers we download from.
    pub peers_sending_to_us: u32,
    /// The directory on the server the data is saved in.
    pub download_dir: String,
    /// When the torrent was added.
    pub added: Option<SystemTime>,
    /// When the torrent finished downloading; `None` while it has not.
    pub done: Option<SystemTime>,
    /// Position in the queue.
    pub queue_position: i64,
    /// Whether pieces are downloaded in order.
    pub sequential: bool,
    /// Progress of the running verification, 0.0 to 1.0.
    pub recheck_progress: f64,
    /// The user's labels.
    pub labels: Vec<String>,
}

/// Why a [`TorrentRecord`] cannot become a [`Torrent`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// The record lacks a field a torrent cannot do without, so `torrent_get` was asked for the
    /// wrong fields.
    #[error("cannot build a torrent from a record without `{0}`")]
    MissingField(&'static str),
    /// The record's `hash_string` is not a valid info hash; holds the value as sent.
    #[error("cannot build a torrent from a record with the invalid hash `{0}`")]
    InvalidHash(String),
}

impl Torrent {
    /// Builds a torrent from the server's record.
    ///
    /// `id`, `hash_string`, `name` and `status` are required; every other field falls back to a
    /// neutral default. Missing `metadata_percent_complete` counts as complete, so a record that
    /// merely lacks the field is not shown as fetching metadata.
    ///
    /// # Errors
    ///
    /// [`ModelError::MissingField`] naming the first of `id`, `hash_string`, `name` and `status`
    /// that the record lacks, or [`ModelError::InvalidHash`] when `hash_string` is not a valid
    /// info hash.
    pub fn from_record(record: TorrentRecord) -> Result<Torrent, ModelError> {
        let id = record.id.ok_or(ModelError::MissingField("id"))?;
        let raw_hash = record
            .hash_string
            .ok_or(ModelError::MissingField("hash_string"))?;
        let name = record.name.ok_or(ModelError::MissingField("name"))?;
        let status = record.status.ok_or(ModelError::MissingField("status"))?;
        let hash = HashString::parse(&raw_hash).map_err(|_| ModelError::InvalidHash(raw_hash))?;

        let error = record.error.unwrap_or(ErrorKind::None);
        let error_string = record.error_string.unwrap_or_default();
        let metadata_progress = record.metadata_percent_complete.unwrap_or(1.0);
        let rate_down = record.rate_download.unwrap_or_default();
        let rate_up = record.rate_upload.unwrap_or_default();

        Ok(Torrent {
            id,
            hash,
            name,
            state: classify(status, error),
            attention: attention(error, &error_string, metadata_progress),
            active: is_active(rate_down, rate_up),
            progress: record.percent_done.unwrap_or_default(),
            metadata_progress,
            total_size: record.total_size.unwrap_or_default(),
            size_when_done: record.size_when_done.unwrap_or_default(),
            left_until_done: record.left_until_done.unwrap_or_default(),
            rate_down,
            rate_up,
            eta: record.eta.map_or(Eta::NotAvailable, Eta::from_wire),
            ratio: record
                .upload_ratio
                .map_or(Ratio::NotAvailable, Ratio::from_wire),
            peers_connected: record.peers_connected.unwrap_or_default(),
            peers_getting_from_us: record.peers_getting_from_us.unwrap_or_default(),
            peers_sending_to_us: record.peers_sending_to_us.unwrap_or_default(),
            download_dir: record.download_dir.unwrap_or_default(),
            added: record.added_date.and_then(date_from_wire),
            done: record.done_date.and_then(date_from_wire),
            queue_position: record.queue_position.unwrap_or_default(),
            sequential: record.sequential_download.unwrap_or_default(),
            recheck_progress: record.recheck_progress.unwrap_or_default(),
            labels: record.labels.unwrap_or_default(),
        })
    }
}

impl Eta {
    /// `-1` is "not available", any other negative value "unknown".
    fn from_wire(seconds: i64) -> Self {
        match u64::try_from(seconds) {
            Ok(seconds) => Self::In(Duration::from_secs(seconds)),
            Err(_) if seconds == -1 => Self::NotAvailable,
            Err(_) => Self::Unknown,
        }
    }
}

impl Ratio {
    /// `-2` and positive infinity are infinite; `-1`, any other negative value and NaN are "not
    /// available"; a negative zero becomes a plain zero.
    fn from_wire(ratio: f64) -> Self {
        match ratio {
            -2.0 => Self::Infinite,
            r if r.is_nan() || r < 0.0 => Self::NotAvailable,
            r if r.is_infinite() => Self::Infinite,
            // `abs` turns -0.0 into 0.0 and leaves every other value alone.
            r => Self::Value(r.abs()),
        }
    }
}

/// Unix seconds as a time; zero, negative values (the server's "unset") and times that
/// `SystemTime` cannot hold are `None`.
fn date_from_wire(seconds: i64) -> Option<SystemTime> {
    let seconds = u64::try_from(seconds).ok().filter(|&seconds| seconds > 0)?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use x4_rpc::testing::fixtures::TORRENT_GET_LIST;
    use x4_rpc::{ErrorKind, Status, TorrentGetResponse};

    use crate::state::QueueKind;

    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    /// A record with only the required fields.
    fn minimal() -> TorrentRecord {
        TorrentRecord {
            id: Some(TorrentId(7)),
            hash_string: Some(HASH.to_owned()),
            name: Some("Example".to_owned()),
            status: Some(Status::Stopped),
            ..TorrentRecord::default()
        }
    }

    fn torrent(record: TorrentRecord) -> Torrent {
        Torrent::from_record(record).unwrap()
    }

    /// The records of the recorded `torrent_get` list: the `result` of the JSON-RPC envelope.
    fn fixture_records() -> Vec<TorrentRecord> {
        let envelope: serde_json::Value = serde_json::from_str(TORRENT_GET_LIST).unwrap();
        let response: TorrentGetResponse =
            serde_json::from_value(envelope["result"].clone()).unwrap();
        response.torrents
    }

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn from_record_requires_identity() {
        let missing = |record: TorrentRecord| Torrent::from_record(record).unwrap_err();

        assert_eq!(
            missing(TorrentRecord {
                hash_string: None,
                ..minimal()
            }),
            ModelError::MissingField("hash_string")
        );
        assert_eq!(
            missing(TorrentRecord {
                id: None,
                ..minimal()
            }),
            ModelError::MissingField("id")
        );
        assert_eq!(
            missing(TorrentRecord {
                name: None,
                ..minimal()
            }),
            ModelError::MissingField("name")
        );
        assert_eq!(
            missing(TorrentRecord {
                status: None,
                ..minimal()
            }),
            ModelError::MissingField("status")
        );
        assert_eq!(
            missing(TorrentRecord::default()),
            ModelError::MissingField("id")
        );
    }

    #[test]
    fn from_record_rejects_an_invalid_hash_with_the_raw_value() {
        let record = TorrentRecord {
            hash_string: Some("not-a-hash".to_owned()),
            ..minimal()
        };
        assert_eq!(
            Torrent::from_record(record).unwrap_err(),
            ModelError::InvalidHash("not-a-hash".to_owned())
        );
    }

    #[test]
    fn from_record_normalises_the_hash() {
        let record = TorrentRecord {
            hash_string: Some(HASH.to_ascii_uppercase()),
            ..minimal()
        };
        assert_eq!(torrent(record).hash.as_str(), HASH);
    }

    #[test]
    fn model_errors_say_what_was_missing_or_invalid() {
        assert_eq!(
            ModelError::MissingField("name").to_string(),
            "cannot build a torrent from a record without `name`"
        );
        assert_eq!(
            ModelError::InvalidHash("zz".to_owned()).to_string(),
            "cannot build a torrent from a record with the invalid hash `zz`"
        );
    }

    #[test]
    fn from_record_defaults_the_optional_fields() {
        let t = torrent(minimal());

        assert_eq!(t.id, TorrentId(7));
        assert_eq!(t.hash, HashString::parse(HASH).unwrap());
        assert_eq!(t.name, "Example");
        assert_eq!(t.state, DisplayState::Paused);
        assert_eq!(t.attention, None);
        assert!(!t.active);
        assert_eq!(t.progress, 0.0);
        assert_eq!(t.metadata_progress, 1.0);
        assert_eq!(
            (t.total_size, t.size_when_done, t.left_until_done),
            (0, 0, 0)
        );
        assert_eq!((t.rate_down, t.rate_up), (0, 0));
        assert_eq!(t.eta, Eta::NotAvailable);
        assert_eq!(t.ratio, Ratio::NotAvailable);
        assert_eq!(
            (
                t.peers_connected,
                t.peers_getting_from_us,
                t.peers_sending_to_us
            ),
            (0, 0, 0)
        );
        assert_eq!(t.download_dir, "");
        assert_eq!((t.added, t.done), (None, None));
        assert_eq!(t.queue_position, 0);
        assert!(!t.sequential);
        assert_eq!(t.recheck_progress, 0.0);
        assert_eq!(t.labels, Vec::<String>::new());
    }

    #[test]
    fn from_record_copies_the_present_fields() {
        let record = TorrentRecord {
            percent_done: Some(0.25),
            metadata_percent_complete: Some(1.0),
            total_size: Some(1_000),
            size_when_done: Some(800),
            left_until_done: Some(600),
            rate_download: Some(10),
            rate_upload: Some(20),
            eta: Some(90),
            upload_ratio: Some(1.5),
            peers_connected: Some(5),
            peers_getting_from_us: Some(2),
            peers_sending_to_us: Some(3),
            download_dir: Some("/data".to_owned()),
            added_date: Some(1_700_000_000),
            done_date: Some(1_700_000_100),
            queue_position: Some(4),
            sequential_download: Some(true),
            recheck_progress: Some(0.5),
            labels: Some(vec!["a".to_owned(), "b".to_owned()]),
            ..minimal()
        };
        let t = torrent(record);

        assert_eq!(t.progress, 0.25);
        assert_eq!(t.metadata_progress, 1.0);
        assert_eq!(
            (t.total_size, t.size_when_done, t.left_until_done),
            (1_000, 800, 600)
        );
        assert_eq!((t.rate_down, t.rate_up), (10, 20));
        assert!(t.active);
        assert_eq!(t.eta, Eta::In(Duration::from_secs(90)));
        assert_eq!(t.ratio, Ratio::Value(1.5));
        assert_eq!(
            (
                t.peers_connected,
                t.peers_getting_from_us,
                t.peers_sending_to_us
            ),
            (5, 2, 3)
        );
        assert_eq!(t.download_dir, "/data");
        assert_eq!(t.added, Some(at(1_700_000_000)));
        assert_eq!(t.done, Some(at(1_700_000_100)));
        assert_eq!(t.queue_position, 4);
        assert!(t.sequential);
        assert_eq!(t.recheck_progress, 0.5);
        assert_eq!(t.labels, ["a", "b"]);
    }

    #[test]
    fn from_record_maps_sentinels() {
        let with = |eta, ratio, added| {
            torrent(TorrentRecord {
                eta: Some(eta),
                upload_ratio: Some(ratio),
                added_date: Some(added),
                ..minimal()
            })
        };

        let t = with(-1, -1.0, 0);
        assert_eq!(t.eta, Eta::NotAvailable);
        assert_eq!(t.ratio, Ratio::NotAvailable);
        assert_eq!(t.added, None);

        let t = with(-2, -2.0, 0);
        assert_eq!(t.eta, Eta::Unknown);
        assert_eq!(t.ratio, Ratio::Infinite);

        assert_eq!(with(90, 0.0, 0).eta, Eta::In(Duration::from_secs(90)));
    }

    #[test]
    fn eta_edges() {
        let eta = |value| {
            torrent(TorrentRecord {
                eta: value,
                ..minimal()
            })
            .eta
        };

        assert_eq!(eta(None), Eta::NotAvailable);
        assert_eq!(eta(Some(0)), Eta::In(Duration::ZERO));
        assert_eq!(eta(Some(-1)), Eta::NotAvailable);
        assert_eq!(eta(Some(-2)), Eta::Unknown);
        assert_eq!(eta(Some(-3)), Eta::Unknown);
        assert_eq!(eta(Some(i64::MIN)), Eta::Unknown);
        assert_eq!(
            eta(Some(i64::MAX)),
            Eta::In(Duration::from_secs(i64::MAX.unsigned_abs()))
        );
    }

    #[test]
    fn ratio_edges() {
        let ratio = |value| {
            torrent(TorrentRecord {
                upload_ratio: value,
                ..minimal()
            })
            .ratio
        };

        assert_eq!(ratio(None), Ratio::NotAvailable);
        assert_eq!(ratio(Some(0.0)), Ratio::Value(0.0));
        assert_eq!(ratio(Some(22.5)), Ratio::Value(22.5));
        assert_eq!(ratio(Some(-1.0)), Ratio::NotAvailable);
        assert_eq!(ratio(Some(-2.0)), Ratio::Infinite);
        assert_eq!(ratio(Some(-0.5)), Ratio::NotAvailable);
        assert_eq!(ratio(Some(-3.0)), Ratio::NotAvailable);
        assert_eq!(ratio(Some(f64::NEG_INFINITY)), Ratio::NotAvailable);
        assert_eq!(ratio(Some(f64::NAN)), Ratio::NotAvailable);
        assert_eq!(ratio(Some(f64::INFINITY)), Ratio::Infinite);
    }

    #[test]
    fn negative_zero_ratio_is_a_plain_zero() {
        let record = TorrentRecord {
            upload_ratio: Some(-0.0),
            ..minimal()
        };
        // `-0.0 == 0.0`, so the sign has to be checked on its own.
        let Ratio::Value(value) = torrent(record).ratio else {
            panic!("a zero ratio is a value");
        };
        assert!(value.is_sign_positive(), "{value:?} keeps its minus sign");
        assert_eq!(Ratio::Value(value), Ratio::Value(0.0));
    }

    #[test]
    fn date_edges() {
        let dates = |added, done| {
            let t = torrent(TorrentRecord {
                added_date: added,
                done_date: done,
                ..minimal()
            });
            (t.added, t.done)
        };

        assert_eq!(dates(None, None), (None, None));
        assert_eq!(dates(Some(0), Some(0)), (None, None));
        assert_eq!(dates(Some(-1), Some(i64::MIN)), (None, None));
        assert_eq!(
            dates(Some(1), Some(1_700_000_000)),
            (Some(at(1)), Some(at(1_700_000_000)))
        );
    }

    #[test]
    fn missing_metadata_progress_is_not_fetching_metadata() {
        let t = torrent(minimal());
        assert_eq!(t.metadata_progress, 1.0);
        assert_eq!(t.attention, None);
    }

    #[test]
    fn magnet_without_metadata_is_fetching_it() {
        let t = torrent(TorrentRecord {
            status: Some(Status::Downloading),
            metadata_percent_complete: Some(0.0),
            ..minimal()
        });
        assert_eq!(t.metadata_progress, 0.0);
        assert_eq!(t.attention, Some(Attention::FetchingMetadata));
        assert_eq!(t.state, DisplayState::Downloading);
    }

    #[test]
    fn downloading_record_is_downloading_and_active() {
        let t = torrent(TorrentRecord {
            status: Some(Status::Downloading),
            percent_done: Some(0.4),
            rate_download: Some(2_516_582),
            eta: Some(3_725),
            ..minimal()
        });
        assert_eq!(t.state, DisplayState::Downloading);
        assert!(t.active);
        assert_eq!(t.attention, None);
        assert_eq!(t.eta, Eta::In(Duration::from_secs(3_725)));
    }

    #[test]
    fn queued_and_checking_records_map_their_states() {
        let state = |status| {
            torrent(TorrentRecord {
                status: Some(status),
                ..minimal()
            })
            .state
        };
        assert_eq!(state(Status::Stopped), DisplayState::Paused);
        assert_eq!(
            state(Status::DownloadWait),
            DisplayState::Queued(QueueKind::Download)
        );
        assert_eq!(
            state(Status::SeedWait),
            DisplayState::Queued(QueueKind::Seed)
        );
        assert_eq!(state(Status::CheckWait), DisplayState::Checking);
        assert_eq!(state(Status::Checking), DisplayState::Checking);
    }

    #[test]
    fn local_error_record_is_an_error_with_its_text() {
        let t = torrent(TorrentRecord {
            status: Some(Status::Stopped),
            error: Some(ErrorKind::LocalError),
            error_string: Some("No space left on device".to_owned()),
            ..minimal()
        });
        assert_eq!(t.state, DisplayState::Error);
        assert_eq!(
            t.attention,
            Some(Attention::LocalError("No space left on device".to_owned()))
        );
    }

    #[test]
    fn tracker_error_record_keeps_seeding_and_flags_attention() {
        let t = torrent(TorrentRecord {
            status: Some(Status::Seeding),
            error: Some(ErrorKind::TrackerError),
            error_string: Some("unreachable".to_owned()),
            ..minimal()
        });
        assert_eq!(t.state, DisplayState::Seeding);
        assert_eq!(
            t.attention,
            Some(Attention::TrackerError("unreachable".to_owned()))
        );
    }

    #[test]
    fn unknown_error_kind_record_is_a_tracker_warning() {
        let t = torrent(TorrentRecord {
            status: Some(Status::Seeding),
            error: Some(ErrorKind::Other(9)),
            error_string: Some("odd".to_owned()),
            ..minimal()
        });
        assert_eq!(t.state, DisplayState::Seeding);
        assert_eq!(
            t.attention,
            Some(Attention::TrackerWarning("odd".to_owned()))
        );
    }

    #[test]
    fn from_fixture_list() {
        let records = fixture_records();
        assert_ne!(records.len(), 0, "the fixture has no torrents");

        for record in records {
            let id = record.id;
            let converted = Torrent::from_record(record);
            assert!(converted.is_ok(), "torrent {id:?}: {converted:?}");
        }
    }

    #[test]
    fn fixture_torrents_are_complete_and_seeding() {
        for record in fixture_records() {
            let t = torrent(record);
            assert_eq!(t.state, DisplayState::Seeding, "{}", t.name);
            assert_eq!(t.attention, None, "{}", t.name);
            assert_eq!(t.progress, 1.0, "{}", t.name);
            assert_eq!(t.left_until_done, 0, "{}", t.name);
            assert!(t.added.is_some(), "{}", t.name);
        }
    }
}
