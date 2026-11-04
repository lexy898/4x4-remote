//! A live test of the country reader against the installed database files, run only by
//! `just test-live`.
//!
//! It reads `GeoIP.dat` and `GeoIPv6.dat` (read-only) from `X4_GEOIP_DIR`, or from
//! `/usr/share/GeoIP`, where the `geoip-database` package puts them. The data changes with each
//! build, so it checks only what is not expected to change: the countries of two public resolvers,
//! whose addresses are each registered to one country, and that loopback addresses have none.

use std::fs::File;
use std::net::IpAddr;
use std::path::PathBuf;

use x4_core::{CountryCode, CountryDb, CountryLookup, Edition, GeoIp};

/// Opens `file` of the directory under test, or says which path failed and why.
fn open(file: &str, edition: Edition) -> Result<CountryDb, String> {
    let dir = std::env::var_os("X4_GEOIP_DIR")
        .map_or_else(|| PathBuf::from("/usr/share/GeoIP"), PathBuf::from);
    let path = dir.join(file);
    let source = File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    CountryDb::open(Box::new(source), edition)
        .map_err(|error| format!("{}: {error}", path.display()))
}

#[test]
#[ignore = "live: needs the installed GeoIP database (geoip-database), run with just test-live"]
fn installed_databases_know_stable_addresses() {
    let v4 = open("GeoIP.dat", Edition::V4).unwrap();
    let v6 = open("GeoIPv6.dat", Edition::V6).unwrap();
    println!("GeoIP.dat: {}; GeoIPv6.dat: {}", v4.info(), v6.info());
    assert!(v4.info().starts_with("GeoLite2 Country"), "{}", v4.info());
    assert!(v6.info().starts_with("GeoLite2 Country"), "{}", v6.info());
    let mut geo = GeoIp::new(Some(v4), Some(v6));
    for (address, country) in [
        ("8.8.8.8", Some("US")),
        ("77.88.8.8", Some("RU")),
        ("2001:4860:4860::8888", Some("US")),
        ("127.0.0.1", None),
        ("::1", None),
    ] {
        let ip: IpAddr = address.parse().unwrap();
        assert_eq!(
            geo.country(ip),
            country.and_then(CountryCode::new),
            "{address}"
        );
    }
}
