//! The rules of the [scrubber](super::Scrubber) for peers and trackers: what stands for a real
//! address, client name, tracker host or tracker URL.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde_json::Value;

use super::{REDACTED, Scrubber};

/// Written for a peer address when the documentation addresses ran out; it is no address, so
/// [`stray_addresses`] counts it and a recording that holds it is not written.
const NO_ADDRESS_LEFT: &str = "no documentation address left";

/// The `/24` blocks reserved for documentation (RFC 5737), in the order they are used; `.0` and
/// `.255` of each stay unused.
const DOCUMENTATION_BLOCKS: [[u8; 3]; 3] = [[192, 0, 2], [198, 51, 100], [203, 0, 113]];
/// How many peer addresses of one block are used: `.1` to `.254`.
const BLOCK_SIZE: usize = 254;

impl Scrubber {
    /// The documentation address standing for the real peer address `text`: the same one every
    /// time. IPv4 addresses (also in their IPv4-mapped IPv6 form) take the blocks of
    /// [`DOCUMENTATION_BLOCKS`] in turn, IPv6 ones count up from `2001:db8::1`. Text that is no
    /// address is redacted.
    pub(super) fn fake_address(&mut self, text: &str) -> String {
        let Ok(address) = text.parse::<IpAddr>() else {
            return if text.is_empty() {
                String::new()
            } else {
                REDACTED.to_owned()
            };
        };
        let address = match address {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
            IpAddr::V4(_) => address,
        };
        if let Some(fake) = self.addresses.get(&address) {
            return fake.clone();
        }
        let same_family = self
            .addresses
            .keys()
            .filter(|known| known.is_ipv4() == address.is_ipv4())
            .count();
        let fake = match address {
            IpAddr::V4(_) => fake_ipv4(same_family),
            IpAddr::V6(_) => fake_ipv6(same_family),
        };
        self.addresses.insert(address, fake.clone());
        fake
    }

    /// The number of the client name `name`; a name seen for the first time gets the next one.
    pub(super) fn client_number(&mut self, name: &str) -> usize {
        let next = self.clients.len() + 1;
        *self.clients.entry(name.to_owned()).or_insert(next)
    }

    /// The number of the tracker host `host` (which is written the way the real one is, in any
    /// case); a host seen for the first time gets the next one.
    pub(super) fn tracker_number(&mut self, host: &str) -> usize {
        self.originals.insert(host.to_owned());
        let next = self.trackers.len() + 1;
        *self
            .trackers
            .entry(host.to_ascii_lowercase())
            .or_insert(next)
    }

    /// `tracker{N}.example.org`, with the port of the real `host:port`.
    pub(super) fn fake_host(&mut self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let (host, port) = split_host_port(text);
        let number = self.tracker_number(host);
        let fake = format!("tracker{number}.example.org");
        match port {
            Some(port) => format!("{fake}:{port}"),
            None => fake,
        }
    }

    /// `https://tracker{N}.example.org/{path}` for the tracker URL `text`; an empty URL stays
    /// empty, and one with no host is redacted. The real path and query, which may hold a
    /// passkey, are not kept.
    pub(super) fn fake_url(&mut self, text: &str, path: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let Some((_, rest)) = text.split_once("://") else {
            return REDACTED.to_owned();
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let authority = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let (host, _) = split_host_port(authority);
        if host.is_empty() {
            return REDACTED.to_owned();
        }
        let number = self.tracker_number(host);
        format!("https://tracker{number}.example.org/{path}")
    }
}

/// How many peer `address` strings in `value` are neither a documentation address nor text the
/// scrubber redacted (or an empty one): zero for a response the scrubber has been through, unless
/// it ran out of documentation addresses. The recorder writes nothing when it is not zero.
#[must_use]
pub fn stray_addresses(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.iter().map(stray_addresses).sum(),
        Value::Object(object) => object
            .iter()
            .map(|(key, item)| match item {
                Value::String(text) if key == "address" => usize::from(!is_safe_address(text)),
                _ => stray_addresses(item),
            })
            .sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 0,
    }
}

/// Whether `text` is empty, redacted, or an IP address in a documentation range.
fn is_safe_address(text: &str) -> bool {
    match text.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            let [a, b, c, host] = address.octets();
            DOCUMENTATION_BLOCKS.contains(&[a, b, c]) && (1..=254).contains(&host)
        }
        Ok(IpAddr::V6(address)) => address.segments().starts_with(&[0x2001, 0x0db8]),
        Err(_) => text.is_empty() || text == REDACTED,
    }
}

/// The `n`-th (from 0) fake IPv4 peer address: `192.0.2.1` to `.254`, then the next block. Past
/// the last block, [`NO_ADDRESS_LEFT`].
fn fake_ipv4(n: usize) -> String {
    let (block, host) = (n / BLOCK_SIZE, n % BLOCK_SIZE + 1);
    let (Some([a, b, c]), Ok(host)) = (DOCUMENTATION_BLOCKS.get(block), u8::try_from(host)) else {
        return NO_ADDRESS_LEFT.to_owned();
    };
    Ipv4Addr::new(*a, *b, *c, host).to_string()
}

/// The `n`-th (from 0) fake IPv6 peer address: `2001:db8::1`, `2001:db8::2`, and so on. Past
/// `2001:db8::ffff`, [`NO_ADDRESS_LEFT`].
fn fake_ipv6(n: usize) -> String {
    match u16::try_from(n + 1) {
        Ok(last) => Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, last).to_string(),
        Err(_) => NO_ADDRESS_LEFT.to_owned(),
    }
}

/// `host` and the port of `host:port`, or of `[v6]:port`; a text with no port is all host.
fn split_host_port(text: &str) -> (&str, Option<&str>) {
    if let Some(rest) = text.strip_prefix('[')
        && let Some((host, after)) = rest.split_once(']')
    {
        return (host, after.strip_prefix(':').filter(|port| is_port(port)));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && is_port(port) => (host, Some(port)),
        Some((host, _)) if !host.contains(':') => (host, None),
        _ => (text, None),
    }
}

/// Whether `text` is a port number, which carries no information about the real host.
fn is_port(text: &str) -> bool {
    !text.is_empty() && text.len() <= 5 && text.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{Value, json};

    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    fn response(torrents: &[Value]) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "torrents": torrents }
        })
    }

    /// A peer entry as the daemon sends it, with a real address and a real client name.
    fn peer(address: &str, client_name: &str) -> Value {
        json!({
            "address": address,
            "port": 51413,
            "client_name": client_name,
            "peer_id": "AAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "flag_str": "TKEH",
            "progress": 0.5,
            "rate_to_client": 1234,
            "rate_to_peer": 0,
            "bytes_to_client": 99,
            "is_utp": true,
        })
    }

    fn peers_response(peers: &[Value]) -> Value {
        response(&[json!({
            "id": 7,
            "hash_string": HASH,
            "peers": peers,
            "peers_from": { "from_dht": 1 },
        })])
    }

    fn peer_texts(value: &Value, key: &str) -> Vec<String> {
        value["result"]["torrents"][0]["peers"]
            .as_array()
            .map(|peers| {
                peers
                    .iter()
                    .map(|peer| peer[key].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn peer_addresses_become_documentation_addresses_in_order_of_appearance() {
        let mut scrubber = Scrubber::new();
        let mut value = peers_response(&[
            peer("203.0.113.50", "A"),
            peer("8.8.4.4", "B"),
            peer("203.0.113.50", "C"),
        ]);

        scrubber.scrub(&mut value);

        assert_eq!(
            peer_texts(&value, "address"),
            ["192.0.2.1", "192.0.2.2", "192.0.2.1"]
        );
        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn one_counter_numbers_the_addresses_of_every_fixture() {
        let mut scrubber = Scrubber::new();
        let mut first = peers_response(&[peer("8.8.4.4", "A"), peer("1.1.1.1", "A")]);
        let mut second = peers_response(&[peer("9.9.9.9", "A"), peer("8.8.4.4", "A")]);

        scrubber.scrub(&mut first);
        scrubber.scrub(&mut second);

        assert_eq!(peer_texts(&first, "address"), ["192.0.2.1", "192.0.2.2"]);
        assert_eq!(peer_texts(&second, "address"), ["192.0.2.3", "192.0.2.1"]);
    }

    #[test]
    fn addresses_go_on_into_the_next_documentation_range_when_one_is_full() {
        let mut scrubber = Scrubber::new();
        let reals: Vec<Value> = (0..509_u32)
            .map(|n| {
                peer(
                    &format!("10.{}.{}.{}", n / 65_536, (n / 256) % 256, n % 256),
                    "A",
                )
            })
            .collect();
        let mut value = peers_response(&reals);

        scrubber.scrub(&mut value);

        let fakes = peer_texts(&value, "address");
        assert_eq!(fakes.first().map(String::as_str), Some("192.0.2.1"));
        assert_eq!(fakes.get(253).map(String::as_str), Some("192.0.2.254"));
        assert_eq!(fakes.get(254).map(String::as_str), Some("198.51.100.1"));
        assert_eq!(fakes.get(507).map(String::as_str), Some("198.51.100.254"));
        assert_eq!(fakes.get(508).map(String::as_str), Some("203.0.113.1"));
        let distinct: BTreeSet<&String> = fakes.iter().collect();
        assert_eq!(distinct.len(), 509);
    }

    #[test]
    fn ipv6_addresses_become_numbers_below_2001_db8() {
        let mut scrubber = Scrubber::new();
        let mut value = peers_response(&[
            peer("2606:4700:4700::1111", "A"),
            peer("8.8.4.4", "A"),
            peer("2001:4860:4860::8888", "A"),
            peer("2606:4700:4700::1111", "A"),
        ]);

        scrubber.scrub(&mut value);

        assert_eq!(
            peer_texts(&value, "address"),
            ["2001:db8::1", "192.0.2.1", "2001:db8::2", "2001:db8::1"]
        );
        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn an_ipv4_mapped_address_is_the_peer_of_its_ipv4_form() {
        let mut scrubber = Scrubber::new();
        let mut value = peers_response(&[
            peer("::ffff:8.8.4.4", "A"),
            peer("8.8.4.4", "A"),
            peer("1.1.1.1", "A"),
        ]);

        scrubber.scrub(&mut value);

        assert_eq!(
            peer_texts(&value, "address"),
            ["192.0.2.1", "192.0.2.1", "192.0.2.2"]
        );
    }

    #[test]
    fn text_that_is_no_address_is_redacted() {
        let mut scrubber = Scrubber::new();
        let mut value = peers_response(&[peer("Invalid address", "A"), peer("", "A")]);

        scrubber.scrub(&mut value);

        assert_eq!(peer_texts(&value, "address"), ["redacted", ""]);
        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn client_names_are_numbered_per_distinct_name_and_the_empty_one_stays() {
        let mut scrubber = Scrubber::new();
        let mut value = peers_response(&[
            peer("8.8.4.4", "RealClient/9.9.9"),
            peer("1.1.1.1", "ab%01%02cd%FE%00"),
            peer("9.9.9.9", "RealClient/9.9.9"),
            peer("4.4.4.4", ""),
        ]);

        scrubber.scrub(&mut value);

        assert_eq!(
            peer_texts(&value, "client_name"),
            ["Client 1", "Client 2", "Client 1", ""]
        );
        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn flag_strings_are_kept_only_within_the_daemons_letters() {
        let flags = |text: &str| {
            let mut entry = peer("8.8.4.4", "A");
            entry["flag_str"] = json!(text);
            let mut value = peers_response(&[entry]);
            Scrubber::new().scrub(&mut value);
            peer_texts(&value, "flag_str").remove(0)
        };

        for kept in ["TKEH", "TODdUuK?EHXI", "", "?"] {
            assert_eq!(flags(kept), kept);
        }
        for dropped in ["TKEQ", "tkeh", "TK EH", "TKEH\u{e9}"] {
            assert_eq!(flags(dropped), "redacted", "{dropped:?}");
        }
    }

    #[test]
    fn peer_ids_are_redacted_and_ports_rates_and_progress_are_kept() {
        let mut value = peers_response(&[peer("8.8.4.4", "A")]);

        Scrubber::new().scrub(&mut value);

        let entry = &value["result"]["torrents"][0]["peers"][0];
        assert_eq!(entry["peer_id"], "redacted");
        assert_eq!(entry["port"], 51413);
        assert_eq!(entry["rate_to_client"], 1234);
        assert_eq!(entry["bytes_to_client"], 99);
        assert_eq!(entry["progress"], 0.5);
        assert_eq!(entry["is_utp"], true);
        assert_eq!(value["result"]["torrents"][0]["peers_from"]["from_dht"], 1);
    }

    fn tracker_entry(host: &str, announce: &str, scrape: &str) -> Value {
        json!({
            "id": 3,
            "tier": 0,
            "host": host,
            "sitename": "realsite",
            "announce": announce,
            "scrape": scrape,
            "last_announce_result": "Success",
            "last_scrape_result": "",
            "announce_state": 1,
        })
    }

    fn trackers_response(entries: &[Value]) -> Value {
        response(&[json!({ "id": 7, "hash_string": HASH, "tracker_stats": entries })])
    }

    #[test]
    fn tracker_hosts_and_urls_become_numbered_example_hosts_that_keep_the_port() {
        let mut scrubber = Scrubber::new();
        let mut value = trackers_response(&[
            tracker_entry(
                "tracker.real.org:6969",
                "http://tracker.real.org:6969/ann?pk=SECRETKEY1",
                "http://tracker.real.org:6969/scrape?pk=SECRETKEY1",
            ),
            tracker_entry("other.real.net:80", "http://other.real.net/announce", ""),
            tracker_entry(
                "Tracker.Real.Org:80",
                "udp://TRACKER.real.org:80/announce",
                "",
            ),
        ]);

        scrubber.scrub(&mut value);

        let entries = &value["result"]["torrents"][0]["tracker_stats"];
        assert_eq!(entries[0]["host"], "tracker1.example.org:6969");
        assert_eq!(
            entries[0]["announce"],
            "https://tracker1.example.org/announce"
        );
        assert_eq!(entries[0]["scrape"], "https://tracker1.example.org/scrape");
        assert_eq!(entries[1]["host"], "tracker2.example.org:80");
        assert_eq!(
            entries[1]["announce"],
            "https://tracker2.example.org/announce"
        );
        assert_eq!(entries[1]["scrape"], "");
        // The same host in another spelling is the same tracker.
        assert_eq!(entries[2]["host"], "tracker1.example.org:80");
        assert_eq!(
            entries[2]["announce"],
            "https://tracker1.example.org/announce"
        );
        assert_eq!(entries[0]["sitename"], "example");
        let text = value.to_string();
        for secret in ["SECRETKEY1", "real.org", "real.net", "realsite"] {
            assert!(!text.contains(secret), "{secret} survived in {text}");
        }
        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn a_tracker_keeps_its_number_in_every_fixture() {
        let mut scrubber = Scrubber::new();
        let mut first = trackers_response(&[tracker_entry("a.real.org:80", "", "")]);
        let mut second = trackers_response(&[
            tracker_entry("b.real.org:80", "", ""),
            tracker_entry("a.real.org:81", "", ""),
        ]);

        scrubber.scrub(&mut first);
        scrubber.scrub(&mut second);

        let host = |value: &Value, index: usize| {
            value["result"]["torrents"][0]["tracker_stats"][index]["host"].clone()
        };
        assert_eq!(host(&first, 0), "tracker1.example.org:80");
        assert_eq!(host(&second, 0), "tracker2.example.org:80");
        assert_eq!(host(&second, 1), "tracker1.example.org:81");
    }

    #[test]
    fn urls_and_hosts_in_other_shapes_are_still_scrubbed() {
        let mut scrubber = Scrubber::new();
        let mut value = trackers_response(&[
            tracker_entry(
                "[2001:4860::1]:6969",
                "udp://[2001:4860::1]:6969/announce",
                "",
            ),
            tracker_entry("nowhere", "not a url at all", "real-scrape-text"),
            tracker_entry("", "", ""),
            tracker_entry(
                "user@real.example:80",
                "http://user:pw@real.example/ann",
                "",
            ),
        ]);

        scrubber.scrub(&mut value);

        let entries = &value["result"]["torrents"][0]["tracker_stats"];
        assert_eq!(entries[0]["host"], "tracker1.example.org:6969");
        assert_eq!(
            entries[0]["announce"],
            "https://tracker1.example.org/announce"
        );
        assert_eq!(entries[1]["announce"], "redacted");
        assert_eq!(entries[1]["scrape"], "redacted");
        assert_eq!(entries[2]["host"], "");
        assert_eq!(entries[2]["announce"], "");
        let text = value.to_string();
        for secret in [
            "2001:4860",
            "pw",
            "real.example",
            "real-scrape-text",
            "not a url",
        ] {
            assert!(!text.contains(secret), "{secret} survived in {text}");
        }
        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn announce_and_scrape_results_keep_only_the_daemons_own_texts() {
        let result = |text: &str| {
            let mut entry = tracker_entry("a.real.org:80", "", "");
            entry["last_announce_result"] = json!(text);
            entry["last_scrape_result"] = json!(text);
            let mut value = trackers_response(&[entry]);
            Scrubber::new().scrub(&mut value);
            let entry = &value["result"]["torrents"][0]["tracker_stats"][0];
            (
                entry["last_announce_result"].clone(),
                entry["last_scrape_result"].clone(),
            )
        };

        for kept in [
            "Success",
            "Could not connect to tracker",
            "Tracker did not respond",
            "",
        ] {
            assert_eq!(result(kept), (json!(kept), json!(kept)));
        }
        for dropped in ["Tracker gave an error: real.example", "success", "Success!"] {
            assert_eq!(
                result(dropped),
                (json!("redacted error"), json!("redacted error")),
                "{dropped:?}"
            );
        }
    }

    #[test]
    fn numbers_flags_and_times_of_a_tracker_entry_are_kept() {
        let mut value = trackers_response(&[tracker_entry("a.real.org:80", "", "")]);
        value["result"]["torrents"][0]["tracker_stats"][0]["next_announce_time"] =
            json!(1_700_000_000);

        Scrubber::new().scrub(&mut value);

        let entry = &value["result"]["torrents"][0]["tracker_stats"][0];
        assert_eq!(entry["id"], 3);
        assert_eq!(entry["tier"], 0);
        assert_eq!(entry["announce_state"], 1);
        assert_eq!(entry["next_announce_time"], 1_700_000_000);
    }

    #[test]
    fn stray_addresses_counts_what_is_outside_the_documentation_ranges() {
        let value = peers_response(&[
            peer("192.0.2.1", "A"),
            peer("198.51.100.254", "A"),
            peer("203.0.113.7", "A"),
            peer("2001:db8::ffff", "A"),
            peer("redacted", "A"),
            peer("", "A"),
            peer("8.8.4.4", "A"),
            peer("192.0.3.1", "A"),
            peer("192.0.2.0", "A"),
            peer("2001:4860::1", "A"),
            peer("Invalid address", "A"),
            peer("::ffff:192.0.2.1", "A"),
        ]);

        // The last six: a mapped address is not what the scrubber writes.
        assert_eq!(stray_addresses(&value), 6);
    }

    #[test]
    fn a_scrubbed_response_has_no_stray_address() {
        let mut value = peers_response(&[
            peer("8.8.4.4", "A"),
            peer("2606:4700:4700::1111", "A"),
            peer("::ffff:1.1.1.1", "A"),
            peer("Invalid address", "A"),
        ]);

        Scrubber::new().scrub(&mut value);

        assert_eq!(stray_addresses(&value), 0);
    }

    #[test]
    fn when_the_documentation_addresses_run_out_the_next_one_is_a_stray() {
        let reals = |count: u32| -> Vec<Value> {
            (0..count)
                .map(|n| peer(&format!("10.0.{}.{}", n / 256, n % 256), "A"))
                .collect()
        };
        let mut full = peers_response(&reals(762));
        let mut over = peers_response(&reals(763));

        Scrubber::new().scrub(&mut full);
        Scrubber::new().scrub(&mut over);

        assert_eq!(stray_addresses(&full), 0);
        assert_eq!(stray_addresses(&over), 1);
    }
}
