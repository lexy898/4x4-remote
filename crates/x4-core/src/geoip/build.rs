//! [`build_country_file`]: a database file in the layout of the installed ones, for tests and
//! the synthetic demo, which must not use excerpts of the real data (its licence forbids passing
//! them on).

use std::net::IpAddr;

use super::{COUNTRY_BEGIN, CountryCode, DELIMITER, Edition, MARKER, country_codes};

/// The text that names the build in every file made here.
pub(super) const INFO: &str = "4x4 synthetic Country";

/// The most nodes a tree can have: node numbers must stay below the first leaf value.
const MAX_NODES: usize = COUNTRY_BEGIN as usize;

/// A record of the tree while it is built.
#[derive(Debug, Clone, Copy)]
enum Child {
    /// No block covers it: written as "not found".
    Empty,
    /// The leaf of the table entry with this index.
    Leaf(u8),
    /// The node with this number.
    Node(usize),
}

/// A database of `edition` in which each block `(network, prefix length, country)` has its
/// country, ending like the files of the package (with the node count).
///
/// Blocks may nest: an address gets the country of the longest prefix that holds it, whatever
/// the order of the blocks. The bits of `network` past the prefix do not matter, and a prefix
/// longer than the address counts as the whole address. A block of the other address family is
/// left out (an IPv4 network in [`Edition::V6`] too, since IPv4 addresses are looked up in the
/// IPv4 file only), and so is a block that would take the tree past its 16 776 960 nodes. A
/// country the table of the format does not have, such as XK, is written as "not found"; EU and
/// AP are written as the regions they are, which a lookup gives no country.
#[must_use]
pub fn build_country_file(edition: Edition, blocks: &[(IpAddr, u8, CountryCode)]) -> Vec<u8> {
    let indexed: Vec<(IpAddr, u8, u8)> = blocks
        .iter()
        .map(|&(network, prefix, country)| {
            (
                network,
                prefix,
                country_codes::index_of(country).unwrap_or(0),
            )
        })
        .collect();
    build_file(edition, &indexed, true)
}

/// A database of `edition` with blocks of table indexes rather than countries, with or without
/// the node count at its end; otherwise as [`build_country_file`].
pub(super) fn build_file(
    edition: Edition,
    blocks: &[(IpAddr, u8, u8)],
    with_node_count: bool,
) -> Vec<u8> {
    let bits = edition.bits();
    let mut blocks: Vec<(u128, u32, u8)> = blocks
        .iter()
        .filter_map(|&(network, prefix, index)| {
            let network = match (edition, network) {
                (Edition::V4, IpAddr::V4(v4)) => u128::from(u32::from(v4)),
                (Edition::V6, IpAddr::V6(v6)) => u128::from(v6),
                _ => return None,
            };
            Some((network, u32::from(prefix).min(bits), index))
        })
        .collect();
    // Shorter prefixes first, so that a longer one splits their leaf and keeps the rest of it.
    blocks.sort_by_key(|&(_, prefix, _)| prefix);
    let mut tree = vec![[Child::Empty; 2]];
    for (network, prefix, index) in blocks {
        if !insert(&mut tree, network, bits, prefix, index) {
            break;
        }
    }
    let records: Vec<[u32; 2]> = tree.iter().map(|pair| pair.map(record)).collect();
    encode(&records, edition, with_node_count)
}

/// Makes the `prefix` first bits of `network` lead to the leaf of `index`, splitting the leaves of
/// shorter blocks on the way. `false` when the tree is full.
fn insert(tree: &mut Vec<[Child; 2]>, network: u128, bits: u32, prefix: u32, index: u8) -> bool {
    let leaf = Child::Leaf(index);
    let Some(last) = prefix.checked_sub(1) else {
        // A /0 is every address: both records of the root.
        if let Some(root) = tree.first_mut() {
            *root = [leaf; 2];
        }
        return true;
    };
    let mut node = 0;
    for depth in 0..last {
        let one = bit(network, bits, depth);
        let child = tree.get(node).map_or(Child::Empty, |pair| *side(pair, one));
        node = match child {
            Child::Node(next) => next,
            Child::Empty | Child::Leaf(_) => {
                if tree.len() >= MAX_NODES {
                    return false;
                }
                // The new node keeps what the record said for both halves.
                tree.push([child; 2]);
                let next = tree.len() - 1;
                if let Some(pair) = tree.get_mut(node) {
                    *side_mut(pair, one) = Child::Node(next);
                }
                next
            }
        };
    }
    if let Some(pair) = tree.get_mut(node) {
        *side_mut(pair, bit(network, bits, last)) = leaf;
    }
    true
}

/// The bit `depth` of a `bits`-wide address, counted from the most significant one.
fn bit(address: u128, bits: u32, depth: u32) -> bool {
    (address >> (bits - 1 - depth)) & 1 == 1
}

fn side(pair: &[Child; 2], one: bool) -> &Child {
    let [zero, one_bit] = pair;
    if one { one_bit } else { zero }
}

fn side_mut(pair: &mut [Child; 2], one: bool) -> &mut Child {
    let [zero, one_bit] = pair;
    if one { one_bit } else { zero }
}

fn record(child: Child) -> u32 {
    match child {
        Child::Empty => COUNTRY_BEGIN,
        Child::Leaf(index) => COUNTRY_BEGIN + u32::from(index),
        // Below MAX_NODES, so it always fits.
        Child::Node(node) => u32::try_from(node).unwrap_or(COUNTRY_BEGIN),
    }
}

/// The file of a tree given as its records, for `edition`: the nodes, the delimiter, [`INFO`], the
/// marker, the type and, `with_node_count`, the number of nodes.
pub(super) fn encode(nodes: &[[u32; 2]], edition: Edition, with_node_count: bool) -> Vec<u8> {
    let mut file = Vec::with_capacity(nodes.len() * 6 + 32);
    for &[zero, one] in nodes {
        file.extend(three_bytes(zero));
        file.extend(three_bytes(one));
    }
    file.extend(DELIMITER);
    file.extend(INFO.as_bytes());
    file.extend(MARKER);
    file.push(edition.type_byte());
    if with_node_count {
        file.extend(three_bytes(u32::try_from(nodes.len()).unwrap_or(0)));
    }
    file
}

/// The low 3 bytes of `value`, little-endian.
fn three_bytes(value: u32) -> [u8; 3] {
    let [low, middle, high, _] = value.to_le_bytes();
    [low, middle, high]
}
