//! The country table of the legacy format: a leaf of the tree holds an index into it.
//!
//! The files store indexes, so the order is fixed by them, and it follows a rule:
//!
//! - index 0 is `--`, no country; 1 and 2 are the regions AP (Asia/Pacific) and EU (Europe);
//! - 3 to 243 are the ISO 3166-1 alpha-2 codes of 2003 in alphabetical order, where five codes
//!   that ISO has withdrawn since lend their place to a newer code: AN to CW (Curaçao), FX to SX
//!   (Sint Maarten), TP to TL (Timor-Leste), YU to RS (Serbia) and ZR to ME (Montenegro);
//! - 244 to 254 are the codes added later, in the order they were added: A1 (anonymous proxy),
//!   A2 (satellite provider), O1 (other country), then AX, GG, IM, JE, BL, MF, BQ and SS.
//!
//! The table was typed for 4x4 from that rule, not copied from a library, and checked against
//! the installed files of the 2026-02 build. For every index that occurs in them (251 besides
//! "not found"), the country that `geoiplookup` names for an address of such a leaf is the one
//! this reader gives, in `GeoIP.dat` and in both the IPv4-mapped and the native part of
//! `GeoIPv6.dat` (754 addresses, no difference). For 224 of the indexes, the delegation files of
//! the regional internet registries also register most of the leaves' addresses to that country;
//! the others are AP, EU and territories registered under another country. The tests pin the
//! rule and the entries the reader depends on.

use super::CountryCode;

/// The number of entries; an index of 255, which the converter never writes, has none.
pub(super) const TABLE_LEN: u32 = 255;

/// Index 0 to 2.
const REGIONS: [&str; 3] = ["--", "AP", "EU"];

/// Index 3 to 243, one row per first letter. The five codes out of order are those that took the
/// place of a withdrawn one.
const ISO_3166_OF_2003: &str = "
    AD AE AF AG AI AL AM CW AO AQ AR AS AT AU AW AZ
    BA BB BD BE BF BG BH BI BJ BM BN BO BR BS BT BV BW BY BZ
    CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CX CY CZ
    DE DJ DK DM DO DZ
    EC EE EG EH ER ES ET
    FI FJ FK FM FO FR SX
    GA GB GD GE GF GH GI GL GM GN GP GQ GR GS GT GU GW GY
    HK HM HN HR HT HU
    ID IE IL IN IO IQ IR IS IT
    JM JO JP
    KE KG KH KI KM KN KP KR KW KY KZ
    LA LB LC LI LK LR LS LT LU LV LY
    MA MC MD MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ
    NA NC NE NF NG NI NL NO NP NR NU NZ
    OM
    PA PE PF PG PH PK PL PM PN PR PS PT PW PY
    QA
    RE RO RU RW
    SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR ST SV SY SZ
    TC TD TF TG TH TJ TK TM TN TO TL TR TT TV TW TZ
    UA UG UM US UY UZ
    VA VC VE VG VI VN VU
    WF WS
    YE YT RS
    ZA ZM ME ZW
";

/// Index 244 to 254.
const ADDED_LATER: [&str; 11] = [
    "A1", "A2", "O1", "AX", "GG", "IM", "JE", "BL", "MF", "BQ", "SS",
];

/// The entries that do not name a country: no country, the two regions, and the three kinds of
/// address that were given no country.
const NOT_COUNTRIES: [&str; 6] = ["--", "AP", "EU", "A1", "A2", "O1"];

/// The entries in index order.
fn entries() -> impl Iterator<Item = &'static str> {
    REGIONS
        .into_iter()
        .chain(ISO_3166_OF_2003.split_ascii_whitespace())
        .chain(ADDED_LATER)
}

/// The entry at `index` as it is in the table, countries or not; `None` past the end.
pub(super) fn code_at(index: u32) -> Option<&'static str> {
    entries().nth(usize::try_from(index).ok()?)
}

/// The country a leaf with `index` stands for; `None` for an entry that is not a country and for
/// an index past the end.
pub(super) fn country(index: u32) -> Option<CountryCode> {
    code_at(index)
        .filter(|entry| !NOT_COUNTRIES.contains(entry))
        .and_then(CountryCode::new)
}

/// The index of the entry `code`, which is not always a country (EU is a region).
pub(super) fn index_of(code: CountryCode) -> Option<u8> {
    let position = entries().position(|entry| entry == code.as_str())?;
    u8::try_from(position).ok()
}
