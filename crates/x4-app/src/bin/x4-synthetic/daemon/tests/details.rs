//! The details of the synthetic torrents through the real client: the fields of a full and of a
//! progress read, the files each torrent has, its metainfo and its trackers.

use std::collections::BTreeSet;

use x4_rpc::{Status, TrackerStatRecord};

use super::support::*;
use super::*;

/// Every torrent with its files and what is needed to check them, but not its pieces, which are
/// heavy: a read of all the torrents' pieces takes seconds.
fn everything(client: &mut Client<SyntheticDaemon>) -> Vec<TorrentRecord> {
    let fields = [
        TorrentField::HashString,
        TorrentField::Name,
        TorrentField::TotalSize,
        TorrentField::PieceSize,
        TorrentField::Files,
    ];
    client
        .torrent_get(&GetIds::All, &fields)
        .expect("the client parses the answer")
        .torrents
}

#[test]
fn synthetic_details_answer_full_and_progress_fields() {
    let mut client = client(Activity::Static);
    let hash = pick(&mut client, |_, files| (3..=12).contains(&files));

    let full = read(&mut client, &hash, &full_and(&[TorrentField::TotalSize]));

    // Every field of the full read is there, and nothing that was not asked for.
    assert!(full.id.is_some());
    assert_eq!(full.hash_string.as_deref(), Some(hash.to_string().as_str()));
    assert!(full.comment.is_some() && full.creator.is_some() && full.source.is_some());
    assert!(full.date_created.is_some() && full.is_private.is_some());
    assert!(full.downloaded_ever.is_some() && full.uploaded_ever.is_some());
    assert!(full.tracker_stats.is_some());
    assert!(full.name.is_none() && full.status.is_none() && full.magnet_link.is_none());
    assert!(full.bytes_completed.is_none() && full.file_stats.is_none());
    let files = full.files.as_deref().expect("files");
    let count = usize::try_from(full.file_count.expect("a file count")).expect("fits");
    assert_eq!(files.len(), count);
    assert_eq!(full.priorities.as_deref().map(<[_]>::len), Some(count));
    assert_eq!(full.wanted.as_deref().map(<[_]>::len), Some(count));
    let total: u64 = files.iter().map(|file| file.length).sum();
    assert_eq!(
        Some(total),
        full.total_size,
        "the files add up to the torrent"
    );
    let piece_size = full.piece_size.expect("a piece size");
    assert!(piece_size.is_power_of_two() && piece_size >= 64 * 1024);
    let pieces = u64::from(full.piece_count.expect("a piece count"));
    assert_eq!(pieces, total.div_ceil(piece_size));
    assert!((1..=16_384).contains(&pieces));

    // The progress read has what changes, and none of what does not.
    let progress = read(&mut client, &hash, TorrentField::DETAILS_PROGRESS);
    assert_eq!(progress.file_count, full.file_count);
    assert_eq!(progress.priorities, full.priorities);
    assert_eq!(progress.wanted, full.wanted);
    assert_eq!(progress.pieces, full.pieces);
    let completed = files.iter().map(|file| file.bytes_completed).collect();
    assert_eq!(progress.bytes_completed, Some(completed));
    assert!(progress.downloaded_ever.is_some() && progress.uploaded_ever.is_some());
    assert!(progress.files.is_none() && progress.tracker_stats.is_none());
    assert!(progress.comment.is_none() && progress.creator.is_none() && progress.source.is_none());
    assert!(progress.date_created.is_none() && progress.is_private.is_none());
    assert!(progress.piece_count.is_none() && progress.piece_size.is_none());

    // The read with the trackers adds them and nothing else.
    let with_trackers = read(&mut client, &hash, TorrentField::DETAILS_PROGRESS_TRACKERS);
    assert_eq!(with_trackers.tracker_stats, full.tracker_stats);
    assert_eq!(with_trackers.bytes_completed, progress.bytes_completed);
    assert!(with_trackers.files.is_none() && with_trackers.comment.is_none());
}

#[test]
fn heavy_fields_are_sent_only_when_asked_for() {
    let mut client = client(Activity::Static);

    // The list carries none of the details.
    let listed = list(&mut client, &GetIds::All);
    assert!(listed.torrents.iter().all(|record| {
        record.files.is_none()
            && record.pieces.is_none()
            && record.file_count.is_none()
            && record.tracker_stats.is_none()
            && record.comment.is_none()
    }));

    // And a read that names no fields gets the fields of the list, not the details.
    let unnamed = post(
        &SyntheticDaemon::new(30, Activity::Static),
        r#"{"jsonrpc":"2.0","id":1,"method":"torrent_get","params":{}}"#,
    );
    let first = unnamed.pointer("/result/torrents/0").expect("a torrent");
    assert!(first.get("name").is_some());
    for heavy in [
        "files",
        "pieces",
        "tracker_stats",
        "bytes_completed",
        "wanted",
        "comment",
    ] {
        assert!(
            first.get(heavy).is_none(),
            "{heavy} is sent unasked: {first}"
        );
    }
}

#[test]
fn a_list_read_does_not_build_metainfo() {
    let mut client = client(Activity::Ticking);
    let hash = pick(&mut client, |_, files| (3..=12).contains(&files));
    let start = metainfo::built();

    // None of these asks for a field of the metainfo, so none builds one, for any torrent, and
    // the records carry none of its keys.
    list(&mut client, &GetIds::All);
    recently_active(&mut client);
    let without = [
        TorrentField::FileCount,
        TorrentField::TrackerStats,
        TorrentField::DownloadedEver,
        TorrentField::UploadedEver,
    ];
    let records = client
        .torrent_get(&GetIds::All, &without)
        .expect("parsed")
        .torrents;
    assert_eq!(records.len(), usize::try_from(COUNT).expect("fits"));
    let files = [
        TorrentField::Files,
        TorrentField::Pieces,
        TorrentField::Wanted,
    ];
    read(&mut client, &hash, &files);
    let raw = client
        .call_raw(
            "torrent_get",
            Some(json!({
                "ids": [hash.to_string()],
                "fields": ["file_count", "tracker_stats", "downloaded_ever", "uploaded_ever"],
            })),
        )
        .expect("an answer");
    let built = metainfo::built() - start;
    assert_eq!(
        built, 0,
        "{built} metainfos were built by reads that ask for none"
    );
    assert!(records.iter().all(|record| {
        record.comment.is_none()
            && record.creator.is_none()
            && record.date_created.is_none()
            && record.source.is_none()
            && record.is_private.is_none()
    }));
    let torrent = raw.pointer("/torrents/0").expect("a torrent");
    assert!(
        torrent.get("tracker_stats").is_some(),
        "the read was answered"
    );
    for key in ["comment", "creator", "date_created", "source", "is_private"] {
        assert!(torrent.get(key).is_none(), "{key} is sent unasked");
    }

    // A field of the metainfo builds it, once for each torrent read, whichever field it is.
    for field in [
        TorrentField::Comment,
        TorrentField::Creator,
        TorrentField::DateCreated,
        TorrentField::Source,
        TorrentField::IsPrivate,
    ] {
        let before = metainfo::built();
        read(&mut client, &hash, &[TorrentField::HashString, field]);
        assert_eq!(metainfo::built() - before, 1, "{field:?}");
    }
    let before = metainfo::built();
    read(&mut client, &hash, TorrentField::DETAILS_FULL);
    assert_eq!(metainfo::built() - before, 1, "a full details read");
}

#[test]
fn synthetic_details_have_the_wire_shapes_of_transmission() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let hash = pick(&mut client(Activity::Static), |_, files| {
        (3..=12).contains(&files)
    });

    let fields = [
        "file_count",
        "files",
        "file_stats",
        "bytes_completed",
        "priorities",
        "wanted",
        "pieces",
        "tracker_stats",
    ];
    let answer = raw_get(&daemon, &hash, &fields);

    let torrent = answer.pointer("/result/torrents/0").expect("a torrent");
    let array = |key: &str| torrent.get(key).and_then(Value::as_array).expect(key);
    assert!(torrent.get("file_count").is_some_and(Value::is_u64));
    assert!(
        array("bytes_completed").iter().all(Value::is_u64),
        "a list of integers"
    );
    let priorities = array("priorities");
    assert!(
        priorities
            .iter()
            .all(|priority| matches!(priority.as_i64(), Some(-1..=1)))
    );
    assert!(
        array("wanted").iter().all(Value::is_boolean),
        "a list of booleans"
    );
    assert!(torrent.get("pieces").is_some_and(Value::is_string));
    let keys = |value: &Value| -> BTreeSet<String> {
        value
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect()
    };
    let named = |names: &[&str]| -> BTreeSet<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    };
    let file = array("files").first().expect("a file");
    let expected = [
        "begin_piece",
        "bytes_completed",
        "end_piece",
        "length",
        "name",
    ];
    assert_eq!(keys(file), named(&expected));
    let stat = array("file_stats").first().expect("a file");
    assert_eq!(
        keys(stat),
        named(&["bytes_completed", "priority", "wanted"])
    );
    let tracker = array("tracker_stats").first().expect("a tracker");
    for key in [
        "host",
        "has_announced",
        "last_announce_succeeded",
        "last_announce_timed_out",
        "last_announce_result",
        "has_scraped",
        "last_scrape_succeeded",
        "last_scrape_timed_out",
        "last_scrape_result",
        "seeder_count",
        "leecher_count",
    ] {
        assert!(
            tracker.get(key).is_some(),
            "a tracker lacks {key}: {tracker}"
        );
    }
}

#[test]
fn most_torrents_have_one_file_and_some_have_a_folder_or_dozens_or_thousands() {
    let counts = counts(&mut client(Activity::Static));

    let share_of = |wanted: fn(u32) -> bool| -> f64 {
        let matching = counts
            .iter()
            .filter(|&&(_, _, files)| wanted(files))
            .count();
        f64::from(u32::try_from(matching).expect("fits")) / f64::from(COUNT)
    };
    let single = share_of(|files| files == 1);
    assert!((0.65..0.80).contains(&single), "{single} have one file");
    // A folder of 3 to 12 files, and up to 11 padding files between them.
    let folder = share_of(|files| (3..=23).contains(&files));
    assert!(
        (0.20..0.30).contains(&folder),
        "{folder} have a folder of files"
    );
    let series = counts
        .iter()
        .filter(|&&(_, _, files)| (40..=79).contains(&files))
        .count();
    assert!(
        (3..=40).contains(&series),
        "{series} torrents of about 40 files"
    );
    let albums = counts
        .iter()
        .filter(|&&(_, _, files)| files == 2000)
        .count();
    assert_eq!(albums, 1, "one torrent has 2000 files");
    assert!(counts.iter().all(|&(_, _, files)| files != 0 && files != 2));
    let album = counts
        .iter()
        .find(|&&(_, _, files)| files == 2000)
        .expect("the album");
    assert_eq!(album.1, Status::Downloading, "its files and pieces move");
}

/// Checks the files of `record` against its size and its pieces, and returns the names of the
/// padding files, if any.
fn assert_files_add_up(record: &TorrentRecord) -> Vec<String> {
    let hash = record.hash_string.as_deref().expect("a hash");
    let files = record.files.as_deref().expect("files");
    let name = record.name.as_deref().expect("a name");
    let piece_size = record.piece_size.expect("a piece size");
    assert_eq!(
        files.iter().map(|file| file.length).sum::<u64>(),
        record.total_size.expect("a size")
    );
    assert!(
        files.iter().all(|file| file.length > 0),
        "{hash}: an empty file"
    );
    let names: BTreeSet<&str> = files.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names.len(), files.len(), "{hash}: names repeat");
    if files.len() == 1 {
        assert_eq!(
            names.first(),
            Some(&name),
            "a single file is named like its torrent"
        );
    }
    let mut pads = Vec::new();
    let mut offset = 0;
    for file in files {
        assert_eq!(
            u64::from(file.begin_piece),
            offset / piece_size,
            "{hash}: {}",
            file.name
        );
        offset += file.length;
        assert_eq!(
            u64::from(file.end_piece),
            offset.div_ceil(piece_size),
            "{hash}: {}",
            file.name
        );
        if files.len() > 1 {
            assert!(
                file.name.starts_with(&format!("{name}/")),
                "{hash}: {}",
                file.name
            );
        }
        if file.name.contains("/.pad/") || file.name.contains("/_____padding_file_") {
            // A padding file ends where the next file may start: on a piece boundary.
            assert_eq!(offset % piece_size, 0, "{hash}: {} does not pad", file.name);
            pads.push(file.name.clone());
        }
    }
    pads
}

#[test]
fn every_torrent_has_files_that_add_up_and_padding_that_fills_pieces() {
    let records = everything(&mut client(Activity::Static));

    let pads: Vec<String> = records.iter().flat_map(assert_files_add_up).collect();

    assert!(
        pads.iter().any(|name| name.contains("/.pad/")),
        "some pad with a .pad folder"
    );
    assert!(
        pads.iter().any(|name| name.contains("/_____padding_file_")),
        "and some with padding files"
    );
    let padded = records.iter().filter(|record| {
        let files = record.files.as_deref().unwrap_or_default();
        files
            .iter()
            .any(|file| file.name.contains("/.pad/") || file.name.contains("/_____padding_file_"))
    });
    assert!(padded.count() >= 5, "a few torrents are padded");
    // The big one is in folders three deep, so that its tree has depth.
    let album = records
        .iter()
        .filter_map(|record| record.files.as_deref())
        .find(|files| files.len() == 2000)
        .expect("the 2000-file torrent");
    let shallowest = album
        .iter()
        .map(|file| file.name.matches('/').count())
        .min();
    assert!(
        shallowest >= Some(3),
        "its files are in nested folders, not {shallowest:?} deep"
    );
    assert!(
        album
            .iter()
            .any(|file| file.name.ends_with("Track 2000.flac"))
    );
}

#[test]
fn the_files_are_the_same_on_every_run() {
    let (mut first, mut second) = (client(Activity::Static), client(Activity::Static));

    let (one, other) = (everything(&mut first), everything(&mut second));

    assert_eq!(one.len(), other.len());
    for (one, other) in one.iter().zip(&other) {
        assert_eq!(one, other);
    }
    // And so is everything else a details read holds, for a sample of the torrents.
    for (hash, ..) in counts(&mut first).iter().step_by(131) {
        let fields = full_and(&[TorrentField::LeftUntilDone]);
        assert_eq!(
            read(&mut first, hash, &fields),
            read(&mut second, hash, &fields)
        );
    }
}

#[test]
fn the_metainfo_covers_what_the_general_tab_shows() {
    let mut client = client(Activity::Static);
    let fields = [
        TorrentField::Comment,
        TorrentField::Creator,
        TorrentField::DateCreated,
        TorrentField::Source,
        TorrentField::IsPrivate,
    ];
    let records = client
        .torrent_get(&GetIds::All, &fields)
        .expect("parsed")
        .torrents;

    let with = |wanted: fn(&TorrentRecord) -> bool| records.iter().filter(|r| wanted(r)).count();
    assert!(
        with(|record| record
            .comment
            .as_deref()
            .is_some_and(|text| !text.is_empty()))
            > 100
    );
    assert!(with(|record| record.comment.as_deref() == Some("")) > 100);
    assert!(with(|record| record.creator.as_deref() == Some("mktorrent 1.1")) > 100);
    assert!(with(|record| record.creator.as_deref() == Some("")) > 10);
    assert!(with(|record| record.date_created.is_some_and(|date| date > 0)) > 100);
    assert!(
        records.iter().all(|record| {
            let dated = record.date_created.is_some_and(|date| date > 0);
            dated
                == record
                    .creator
                    .as_deref()
                    .is_some_and(|creator| !creator.is_empty())
        }),
        "a client that leaves its name out leaves the date out too"
    );
    let sourced = with(|record| {
        record
            .source
            .as_deref()
            .is_some_and(|text| !text.is_empty())
    });
    assert!(
        (3..300).contains(&sourced),
        "{sourced} torrents have a source"
    );
    assert!(
        records.iter().all(|record| {
            record.source.as_deref() == Some("") || record.is_private == Some(true)
        }),
        "a source tag belongs to a private torrent"
    );
}

/// The trackers of every torrent.
fn all_trackers(client: &mut Client<SyntheticDaemon>) -> Vec<(HashString, Vec<TrackerStatRecord>)> {
    let fields = [TorrentField::HashString, TorrentField::TrackerStats];
    let records = client
        .torrent_get(&GetIds::All, &fields)
        .expect("parsed")
        .torrents;
    records
        .into_iter()
        .map(|record| {
            let hash = HashString::parse(&record.hash_string.expect("a hash")).expect("valid");
            (hash, record.tracker_stats.expect("trackers"))
        })
        .collect()
}

#[test]
fn the_trackers_come_in_every_shape_the_summary_words() {
    let all = all_trackers(&mut client(Activity::Static));

    // No tracker, one, or twelve.
    let sizes: BTreeSet<usize> = all.iter().map(|(_, trackers)| trackers.len()).collect();
    assert_eq!(sizes, BTreeSet::from([0, 1, 12]));
    let stats: Vec<&TrackerStatRecord> = all.iter().flat_map(|(_, trackers)| trackers).collect();
    for tracker in &stats {
        let (name, port) = tracker.host.rsplit_once(':').expect("a host and a port");
        assert!(
            name.ends_with(".example.org") || name.ends_with(".example.net"),
            "{name}"
        );
        assert!(matches!(port, "6969" | "443" | "80"), "{port}");
        if tracker.has_announced {
            let timed_out = tracker.last_announce_result == "Tracker did not respond";
            assert_eq!(
                tracker.last_announce_succeeded,
                tracker.last_announce_result == "Success"
            );
            assert_eq!(
                tracker.last_announce_timed_out, timed_out,
                "the flag and the text agree"
            );
        }
    }
    // Working trackers, and each way to fail: unreachable, no answer, and a refusal.
    let answered = |text: &str| {
        stats
            .iter()
            .filter(|t| t.last_announce_result == text)
            .count()
    };
    assert!(answered("Success") > 100);
    for text in [
        "Could not connect to tracker",
        "Tracker did not respond",
        "Unregistered torrent",
    ] {
        assert!(answered(text) > 0, "no tracker says {text:?}");
    }
    assert!(
        stats
            .iter()
            .all(|tracker| tracker.seeder_count.is_some_and(|count| count >= -1))
    );
    // Twelve trackers: one only a backup, never announced to, and a torrent with one failing.
    let twelve: Vec<_> = all
        .iter()
        .filter(|(_, trackers)| trackers.len() == 12)
        .collect();
    assert!(
        twelve
            .iter()
            .all(|(_, trackers)| trackers.iter().any(|t| !t.has_announced))
    );
    let one_failing = |trackers: &[TrackerStatRecord]| {
        trackers
            .iter()
            .filter(|t| t.has_announced && !t.last_announce_succeeded)
            .count()
            == 1
    };
    assert!(twelve.iter().any(|(_, trackers)| one_failing(trackers)));
    assert!(twelve.iter().any(|(_, trackers)| {
        trackers
            .iter()
            .all(|t| !t.has_announced || t.last_announce_succeeded)
    }));
}

#[test]
fn a_torrent_the_list_shows_with_a_tracker_problem_has_a_tracker_that_fails() {
    let mut client = client(Activity::Static);
    let trackers = all_trackers(&mut client);
    let all = shown(list(&mut client, &GetIds::All));

    let with_problem = |attention: fn(&Attention) -> bool| -> Vec<&ShownTorrent> {
        all.iter()
            .filter(|torrent| torrent.attention.as_ref().is_some_and(attention))
            .collect()
    };
    let warned = with_problem(|attention| matches!(attention, Attention::TrackerWarning(_)));
    let refused = with_problem(|attention| matches!(attention, Attention::TrackerError(_)));

    assert!(!warned.is_empty() && !refused.is_empty());
    for (torrents, text) in [
        (warned, "Could not connect to tracker"),
        (refused, "Unregistered torrent"),
    ] {
        for torrent in torrents {
            let (_, own) = trackers
                .iter()
                .find(|(hash, _)| *hash == torrent.hash)
                .expect("listed");
            assert_eq!(own.len(), 1, "{}", torrent.hash);
            assert_eq!(
                own.first().map(|t| t.last_announce_result.as_str()),
                Some(text)
            );
        }
    }
}

#[test]
fn what_was_downloaded_and_uploaded_follows_the_progress_and_the_ratio() {
    let mut client = client(Activity::Static);
    let fields = [TorrentField::DownloadedEver, TorrentField::UploadedEver];
    let records = client
        .torrent_get(&GetIds::All, &fields)
        .expect("parsed")
        .torrents;
    let all = shown(list(&mut client, &GetIds::All));

    for (record, torrent) in records.iter().zip(&all) {
        let downloaded = record.downloaded_ever.expect("downloaded");
        assert_eq!(downloaded, torrent.total_size - torrent.left_until_done);
        let uploaded = record.uploaded_ever.expect("uploaded");
        if downloaded > 0 {
            assert!(share(uploaded, downloaded) < 5.0, "a ratio below 5");
        }
    }
}

/// An engine over the daemon, as the demo runs it, reads the details of a selected torrent: the
/// full read comes back as details with every file, and the progress read follows.
#[test]
fn the_service_reads_the_details_of_the_selected_torrent() {
    let mut probe = client(Activity::Static);
    let hash = pick(&mut probe, |status, files| {
        status == Status::Downloading && files > 2
    });
    let expected = read(&mut probe, &hash, &[TorrentField::FileCount]).file_count;
    let config = EngineConfig {
        details_settle: Duration::from_millis(10),
        details_interval: Duration::from_millis(50),
        visible_interval: Duration::from_millis(50),
        ..EngineConfig::default()
    };
    let service = service::spawn(client(Activity::Ticking), config).expect("the service starts");
    let (events, received) = crossbeam_channel::unbounded();
    let source = service.events().clone();
    let forward = thread::spawn(move || {
        while let Ok(event) = source.recv_blocking() {
            if events.send(event).is_err() {
                break;
            }
        }
    });
    service
        .commands()
        .send(Command::Select(Some(hash.clone())))
        .expect("the service listens");

    let mut seen = Vec::new();
    while seen.len() < 2 {
        seen.push(wait_for(&received, |event| match event {
            Event::Details(details) => Some(details.clone()),
            _ => None,
        }));
    }
    service.shutdown();
    forward.join().expect("the forwarding thread ends");

    let (first, second) = (seen.first().expect("one"), seen.get(1).expect("two"));
    assert_eq!(first.hash, hash);
    assert_eq!(
        Some(u32::try_from(first.files.len()).expect("fits")),
        expected
    );
    assert!(first.files.iter().all(|file| file.length > 0));
    assert_eq!(
        first.files.len(),
        second.files.len(),
        "the progress read keeps the files"
    );
    assert!(first.piece_size > 0 && first.piece_count > 0);
    assert!(
        first.files.iter().any(|file| file.bytes_completed > 0),
        "a downloading torrent has some of its files"
    );
    assert!(
        second
            .files
            .iter()
            .zip(&first.files)
            .all(|(now, then)| now.bytes_completed >= then.bytes_completed)
    );
}
