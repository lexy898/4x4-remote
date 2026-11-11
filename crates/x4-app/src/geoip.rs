//! The country databases the system has installed, the legacy `GeoIP.dat` and `GeoIPv6.dat`:
//! where they are looked for, and opening them for the service, which looks up the countries of
//! the peers.
//!
//! The `geoip-database` package installs `GeoIP.dat` (countries by IPv4 address) and
//! `GeoIPv6.dat` (by IPv6 address) in `/usr/share/GeoIP`. Each file is looked for on its own, and
//! the first place that has it wins: `GeoIP` in `$XDG_DATA_HOME` (`~/.local/share` when it is not
//! set), in each directory of `$XDG_DATA_DIRS` (`/usr/local/share:/usr/share` when it is not set),
//! then `/usr/share/GeoIP` and `/var/lib/GeoIP`, where libGeoIP and `geoipupdate` keep them,
//! whatever the XDG variables say: a sandbox or a test session may change them. A relative XDG
//! entry is ignored, as the XDG base directory spec has it.
//!
//! The files are optional: without them the peers show no country. A missing file is logged once,
//! as `info`, with the directories searched; a file that is there but cannot be used is logged as
//! a `warn` with its path and why, and one that a lookup finds corrupt later is logged once by
//! the lookup, also with its path. Each file opened is logged as `info` with the build it names.
//!
//! Opening checks the end of each file and reads no more of it, but it touches the disk, so it
//! runs on the service thread, when the first peers are shown ([`x4_core::service::OpenCountries`]).

use std::collections::HashSet;
use std::env;
use std::error::Error as _;
use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use x4_core::{CountryDb, Edition, GeoIp, GeoIpError};

/// The files of the two databases, with the edition each holds.
const FILES: [(&str, Edition); 2] = [("GeoIP.dat", Edition::V4), ("GeoIPv6.dat", Edition::V6)];
/// The directory the files are in, inside a data directory.
const SUBDIR: &str = "GeoIP";
/// The data home inside the home directory when `$XDG_DATA_HOME` is not set.
const DEFAULT_DATA_HOME: &str = ".local/share";
/// The data directories when `$XDG_DATA_DIRS` is not set.
const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";
/// Where libGeoIP looks by default, and where `geoipupdate` and Debian put the files.
const FIXED_DIRS: [&str; 2] = ["/usr/share/GeoIP", "/var/lib/GeoIP"];

/// Opens the installed country databases, either of which may be missing, as the environment
/// and the home directory say where to look.
pub(crate) fn open_installed() -> GeoIp {
    let home = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    open_in(&search_dirs(
        env::var_os("XDG_DATA_HOME"),
        env::var_os("XDG_DATA_DIRS"),
        home.as_deref(),
    ))
}

/// The directories to look for the files in, in order, each once: for `data_home`, the value of
/// `$XDG_DATA_HOME`, `data_dirs`, that of `$XDG_DATA_DIRS`, and `home`, the home directory.
/// An unset or empty variable takes its default; a relative entry is left out.
fn search_dirs(
    data_home: Option<OsString>,
    data_dirs: Option<OsString>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let data_home = data_home
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(DEFAULT_DATA_HOME)));
    let data_dirs = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| DEFAULT_DATA_DIRS.into());
    let mut seen = HashSet::new();
    data_home
        .into_iter()
        .chain(env::split_paths(&data_dirs).filter(|dir| dir.is_absolute()))
        .map(|dir| dir.join(SUBDIR))
        .chain(FIXED_DIRS.map(PathBuf::from))
        .filter(|dir| seen.insert(dir.clone()))
        .collect()
}

/// Opens each database from the first of `dirs` that has its file, and logs what it found.
fn open_in(dirs: &[PathBuf]) -> GeoIp {
    let mut missing = Vec::new();
    let [v4, v6] = FILES.map(|(name, edition)| {
        let found = dirs
            .iter()
            .map(|dir| dir.join(name))
            .find(|path| path.exists());
        if found.is_none() {
            missing.push(name);
        }
        found.and_then(|path| open_file(&path, edition))
    });
    if !missing.is_empty() {
        tracing::info!(
            files = ?missing,
            dirs = ?dirs,
            "no GeoIP country database found; peers show no country from it"
        );
    }
    GeoIp::new(v4, v6)
}

/// Opens the database of `edition` at `path`, or logs why it cannot be used.
fn open_file(path: &Path, edition: Edition) -> Option<CountryDb> {
    let opened = File::open(path)
        .map_err(GeoIpError::Read)
        .and_then(|file| CountryDb::open(Box::new(file), edition));
    match opened {
        Ok(db) => {
            tracing::info!(
                file = %path.display(),
                %edition,
                info = db.info(),
                "opened the GeoIP country database"
            );
            Some(db.named(path.display().to_string()))
        }
        Err(error) => {
            tracing::warn!(
                file = %path.display(),
                %edition,
                error = reason(&error),
                "the GeoIP country database cannot be used; peers show no country from it"
            );
            None
        }
    }
}

/// `error` and the causes behind it on one line, such as `reading the file failed: Permission
/// denied (os error 13)`.
fn reason(error: &GeoIpError) -> String {
    let mut reason = error.to_string();
    let mut cause = error.source();
    while let Some(error) = cause {
        reason.push_str(": ");
        reason.push_str(&error.to_string());
        cause = error.source();
    }
    reason
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::IpAddr;
    use std::slice;

    use tracing::Level;
    use x4_core::{CountryCode, CountryLookup as _, Edition, build_country_file};

    use super::*;
    use crate::test_log::{Record, records_of};

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn code(letters: &str) -> CountryCode {
        CountryCode::new(letters).unwrap()
    }

    fn paths(dirs: &[&str]) -> Vec<PathBuf> {
        dirs.iter().map(PathBuf::from).collect()
    }

    /// A database of `edition` that puts the documentation range of the edition in `country`.
    fn database(edition: Edition, country: &str) -> Vec<u8> {
        let network = match edition {
            Edition::V4 => ip("192.0.2.0"),
            Edition::V6 => ip("2001:db8::"),
        };
        let prefix = match edition {
            Edition::V4 => 24,
            Edition::V6 => 32,
        };
        build_country_file(edition, &[(network, prefix, code(country))])
    }

    /// The `GeoIP` directory of a data directory under `root`, with `files` in it.
    fn geoip_dir(root: &Path, name: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
        let dir = root.join(name).join("GeoIP");
        fs::create_dir_all(&dir).unwrap();
        for (file, bytes) in files {
            fs::write(dir.join(file), bytes).unwrap();
        }
        dir
    }

    /// The country of `address` in `geoip`, as text.
    fn country(geoip: &mut GeoIp, address: &str) -> Option<String> {
        geoip
            .country(ip(address))
            .map(|code| code.as_str().to_owned())
    }

    fn at(records: &[Record], level: Level) -> Vec<&Record> {
        records
            .iter()
            .filter(|record| record.level == level)
            .collect()
    }

    #[test]
    fn the_data_home_comes_first_then_the_data_dirs_then_the_fixed_places() {
        let dirs = search_dirs(
            Some("/home/u/data".into()),
            Some("/opt/share:/usr/share".into()),
            Some(Path::new("/home/u")),
        );

        assert_eq!(
            dirs,
            paths(&[
                "/home/u/data/GeoIP",
                "/opt/share/GeoIP",
                "/usr/share/GeoIP",
                "/var/lib/GeoIP",
            ]),
            "each directory once, in the order of its first place"
        );
    }

    #[test]
    fn unset_or_empty_xdg_variables_take_their_defaults() {
        let defaults = paths(&[
            "/home/u/.local/share/GeoIP",
            "/usr/local/share/GeoIP",
            "/usr/share/GeoIP",
            "/var/lib/GeoIP",
        ]);
        for (data_home, data_dirs) in [(None, None), (Some(OsString::new()), Some(OsString::new()))]
        {
            assert_eq!(
                search_dirs(
                    data_home.clone(),
                    data_dirs.clone(),
                    Some(Path::new("/home/u"))
                ),
                defaults,
                "{data_home:?} {data_dirs:?}"
            );
        }
        // Without a home, there is no data home to default to.
        assert_eq!(search_dirs(None, None, None), defaults[1..]);
    }

    #[test]
    fn relative_xdg_entries_are_ignored() {
        let dirs = search_dirs(
            Some("data".into()),
            Some("share:/opt/share::./more".into()),
            Some(Path::new("/home/u")),
        );

        assert_eq!(
            dirs,
            paths(&[
                "/home/u/.local/share/GeoIP",
                "/opt/share/GeoIP",
                "/usr/share/GeoIP",
                "/var/lib/GeoIP",
            ])
        );
    }

    #[test]
    fn each_file_is_taken_from_the_first_directory_that_has_it() {
        let root = tempfile::tempdir().unwrap();
        let first = geoip_dir(
            root.path(),
            "first",
            &[("GeoIP.dat", database(Edition::V4, "GB"))],
        );
        let second = geoip_dir(
            root.path(),
            "second",
            &[
                ("GeoIP.dat", database(Edition::V4, "US")),
                ("GeoIPv6.dat", database(Edition::V6, "DE")),
            ],
        );

        let records = records_of(|| {
            let mut geoip = open_in(&[first.clone(), second.clone()]);
            assert_eq!(country(&mut geoip, "192.0.2.1").as_deref(), Some("GB"));
            assert_eq!(country(&mut geoip, "2001:db8::1").as_deref(), Some("DE"));
        });

        // One line for each file opened, with its path and the build it names.
        let opened = at(&records, Level::INFO);
        assert_eq!(opened.len(), 2, "{records:?}");
        let v4 = first.join("GeoIP.dat").display().to_string();
        let v6 = second.join("GeoIPv6.dat").display().to_string();
        assert_eq!(opened[0].field("file"), Some(v4.as_str()), "{records:?}");
        assert_eq!(opened[1].field("file"), Some(v6.as_str()), "{records:?}");
        for record in opened {
            assert!(
                record
                    .field("info")
                    .is_some_and(|info| info.contains("synthetic")),
                "{record:?}"
            );
        }
        assert_eq!(at(&records, Level::WARN), Vec::<&Record>::new());
    }

    #[test]
    fn missing_files_give_no_countries_and_one_line_with_the_directories_searched() {
        let root = tempfile::tempdir().unwrap();
        let empty = geoip_dir(root.path(), "empty", &[]);
        let absent = root.path().join("absent").join("GeoIP");

        let records = records_of(|| {
            let mut geoip = open_in(&[empty.clone(), absent.clone()]);
            assert_eq!(country(&mut geoip, "192.0.2.1"), None);
            assert_eq!(country(&mut geoip, "2001:db8::1"), None);
        });

        let [missing] = records.as_slice() else {
            panic!("one line: {records:?}");
        };
        assert_eq!(missing.level, Level::INFO);
        let dirs = missing.field("dirs").unwrap_or_default();
        assert!(
            dirs.contains(&empty.display().to_string())
                && dirs.contains(&absent.display().to_string()),
            "{missing:?}"
        );
        let files = missing.field("files").unwrap_or_default();
        assert!(
            files.contains("GeoIP.dat") && files.contains("GeoIPv6.dat"),
            "{missing:?}"
        );

        // Only the IPv6 file missing: the line names it alone.
        let records = records_of(|| {
            let only_v4 = geoip_dir(
                root.path(),
                "only-v4",
                &[("GeoIP.dat", database(Edition::V4, "GB"))],
            );
            open_in(&[only_v4]);
        });
        let missing = at(&records, Level::INFO)
            .into_iter()
            .find(|record| record.field("files").is_some())
            .expect("a line for the missing file");
        assert_eq!(missing.field("files"), Some(r#"["GeoIPv6.dat"]"#));
    }

    #[test]
    fn a_broken_file_is_logged_once_with_its_path_and_why_and_gives_no_countries() {
        let root = tempfile::tempdir().unwrap();
        // The first place holds a cut file, and the IPv6 database where the IPv4 one belongs;
        // the good files further on are not looked at: the first file found is the one used.
        let mut cut = database(Edition::V4, "GB");
        cut.truncate(cut.len() - 2);
        let broken = geoip_dir(
            root.path(),
            "broken",
            &[
                ("GeoIP.dat", cut),
                ("GeoIPv6.dat", database(Edition::V4, "GB")),
            ],
        );
        let good = geoip_dir(
            root.path(),
            "good",
            &[
                ("GeoIP.dat", database(Edition::V4, "US")),
                ("GeoIPv6.dat", database(Edition::V6, "DE")),
            ],
        );

        let records = records_of(|| {
            let mut geoip = open_in(&[broken.clone(), good.clone()]);
            assert_eq!(country(&mut geoip, "192.0.2.1"), None);
            assert_eq!(country(&mut geoip, "2001:db8::1"), None);
        });

        let warnings = at(&records, Level::WARN);
        assert_eq!(warnings.len(), 2, "{records:?}");
        for (warning, name, reason) in [
            (warnings[0], "GeoIP.dat", "after its marker"),
            (warnings[1], "GeoIPv6.dat", "not of type 12"),
        ] {
            let path = broken.join(name).display().to_string();
            assert_eq!(warning.field("file"), Some(path.as_str()), "{warning:?}");
            assert!(
                warning
                    .field("error")
                    .is_some_and(|error| error.contains(reason)),
                "{warning:?}"
            );
        }
        assert_eq!(at(&records, Level::INFO), Vec::<&Record>::new());
        assert!(!records.iter().any(|record| record.level == Level::ERROR));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_opened_is_logged_with_the_reason_of_the_system() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let dir = geoip_dir(
            root.path(),
            "locked",
            &[("GeoIP.dat", database(Edition::V4, "GB"))],
        );
        let file = dir.join("GeoIP.dat");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        if File::open(&file).is_ok() {
            eprintln!("skipped: the mode of a file does not stop this user (root?)");
            return;
        }

        let records = records_of(|| {
            open_in(slice::from_ref(&dir));
        });

        let [warning] = at(&records, Level::WARN)[..] else {
            panic!("one warning: {records:?}");
        };
        let path = file.display().to_string();
        assert_eq!(warning.field("file"), Some(path.as_str()));
        let error = warning.field("error").unwrap_or_default();
        assert!(error.contains("Permission denied"), "{warning:?}");
    }

    /// A database of two nodes whose two walks from the root end in a leaf, so that it opens,
    /// but whose walk for an address that starts with the bits 1 and 0 meets a node past the end.
    fn corrupt_past_its_probes() -> Vec<u8> {
        let three = |value: u32| {
            let [low, middle, high, _] = value.to_le_bytes();
            [low, middle, high]
        };
        let leaf = |index: u32| 0xFF_FF00 + index;
        let mut file = Vec::new();
        for [zero, one] in [[leaf(225), 1], [99, leaf(56)]] {
            file.extend(three(zero));
            file.extend(three(one));
        }
        file.extend([0, 0, 0]);
        file.extend(b"4x4 corrupt Country");
        file.extend([0xFF, 0xFF, 0xFF, 1]);
        file.extend(three(2));
        file
    }

    #[test]
    fn a_file_a_lookup_finds_corrupt_is_logged_once_with_its_path() {
        let root = tempfile::tempdir().unwrap();
        let dir = geoip_dir(
            root.path(),
            "corrupt",
            &[("GeoIP.dat", corrupt_past_its_probes())],
        );
        let mut geoip = open_in(slice::from_ref(&dir));

        let records = records_of(|| {
            assert_eq!(country(&mut geoip, "128.0.0.1"), None);
            assert_eq!(country(&mut geoip, "255.0.0.1"), None);
            assert_eq!(country(&mut geoip, "128.0.0.2"), None);
        });

        let [warning] = records.as_slice() else {
            panic!("one line: {records:?}");
        };
        assert_eq!(warning.level, Level::WARN);
        let path = dir.join("GeoIP.dat").display().to_string();
        assert_eq!(warning.field("file"), Some(path.as_str()), "{warning:?}");
    }
}
