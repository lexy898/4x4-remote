//! A bencode writer for the test torrents. It writes exactly what it is told: a dictionary made
//! by [`unsorted_dict`] keeps its keys in the order given, repeated ones included, and
//! [`Bencode::Raw`] splices in bytes that need not be bencode at all, so a test can build the odd
//! and broken files the reader must cope with.

/// A bencode value.
#[derive(Debug, Clone)]
pub(crate) enum Bencode {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Bencode>),
    /// Written in the order given.
    Dict(Vec<(Vec<u8>, Bencode)>),
    /// Written as is.
    Raw(Vec<u8>),
}

impl Bencode {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::Int(value) => out.extend_from_slice(format!("i{value}e").as_bytes()),
            Self::Bytes(bytes) => {
                out.extend_from_slice(format!("{}:", bytes.len()).as_bytes());
                out.extend_from_slice(bytes);
            }
            Self::List(items) => {
                out.push(b'l');
                for item in items {
                    item.encode_into(out);
                }
                out.push(b'e');
            }
            Self::Dict(entries) => {
                out.push(b'd');
                for (key, value) in entries {
                    Self::Bytes(key.clone()).encode_into(out);
                    value.encode_into(out);
                }
                out.push(b'e');
            }
            Self::Raw(bytes) => out.extend_from_slice(bytes),
        }
    }
}

pub(crate) fn int(value: i64) -> Bencode {
    Bencode::Int(value)
}

pub(crate) fn bytes(value: impl AsRef<[u8]>) -> Bencode {
    Bencode::Bytes(value.as_ref().to_vec())
}

pub(crate) fn raw(value: impl AsRef<[u8]>) -> Bencode {
    Bencode::Raw(value.as_ref().to_vec())
}

pub(crate) fn list(items: impl IntoIterator<Item = Bencode>) -> Bencode {
    Bencode::List(items.into_iter().collect())
}

/// A dictionary with its keys sorted by their bytes, as bencode wants them.
pub(crate) fn dict<'k>(entries: impl IntoIterator<Item = (&'k str, Bencode)>) -> Bencode {
    let mut entries: Vec<_> = entries.into_iter().collect();
    entries.sort_by_key(|(key, _)| *key);
    unsorted_dict(entries)
}

/// A dictionary with its keys in the order given.
pub(crate) fn unsorted_dict<'k>(entries: impl IntoIterator<Item = (&'k str, Bencode)>) -> Bencode {
    Bencode::Dict(
        entries
            .into_iter()
            .map(|(key, value)| (key.as_bytes().to_vec(), value))
            .collect(),
    )
}
