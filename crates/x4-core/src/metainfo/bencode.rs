//! A strict reader of bencode, the format of `.torrent` files, sized to the limits of
//! libtransmission's own reader (T/benc.h, T/variant-benc.cc).
//!
//! It never recurses and never builds a tree. [`parse`] checks the whole document once, in a loop
//! over a stack of at most [`MAX_DEPTH`] frames, and gives its first value. A list or a dictionary
//! is a [`List`] or a [`Dict`] that borrows its raw bytes from the `l` or `d` through the `e`, and
//! reads its items only when they are walked. So reading costs a few passes over the input and
//! no memory beyond the stack, and a dictionary's raw bytes are there to be hashed.
//!
//! It accepts what libtransmission accepts, except that a dictionary key must be a byte string,
//! and notes what libtransmission reads without checking, keys out of order or repeated, in
//! [`Document::odd_keys`]. Like libtransmission it ignores whatever follows the first value.

/// The most lists and dictionaries that may be open at once. libtransmission's parser stack has
/// 32 slots and refuses the push that would fill the last one (T/benc.h:248-258, with
/// `MaxBencDepth` = 32, T/torrent-metainfo.cc:57), so 31 levels are read and the 32nd is not.
pub(super) const MAX_DEPTH: usize = 31;

/// A byte string must be shorter than this (`MaxBencStrLength`, T/variant-benc.cc:32, 109-113).
pub(super) const MAX_STRING_BYTES: usize = 128 * 1024 * 1024;

/// Why the input is not bencode the reader accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Error {
    /// The value at `offset` is malformed, or the input ends there.
    Syntax { offset: usize },
    /// The list or dictionary at `offset` would be the 32nd level of nesting.
    TooDeep { offset: usize },
}

/// A whole document: its first value, and whether a dictionary anywhere in it has a key that is
/// not greater than the one before it, which bencode forbids.
#[derive(Debug, Clone, Copy)]
pub(super) struct Document<'a> {
    pub(super) root: Value<'a>,
    pub(super) odd_keys: bool,
}

/// A value, borrowed from the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Value<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(List<'a>),
    Dict(Dict<'a>),
}

/// A list: its raw bytes, from the `l` through the `e`, and where they start in the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct List<'a> {
    raw: &'a [u8],
    offset: usize,
}

/// A dictionary: its raw bytes, from the `d` through the `e`, and where they start in the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Dict<'a> {
    raw: &'a [u8],
    offset: usize,
}

/// Reads `input` as one bencoded value, checking all of it.
///
/// # Errors
///
/// [`Error::Syntax`] where the value is malformed or the input ends too early, and
/// [`Error::TooDeep`] at the list or dictionary that nests too deep.
pub(super) fn parse(input: &[u8]) -> Result<Document<'_>, Error> {
    let read = read_value(input, 0, 0)?;
    Ok(Document {
        root: read.value,
        odd_keys: read.odd_keys,
    })
}

impl<'a> List<'a> {
    /// Whether the list has no items.
    pub(super) fn is_empty(&self) -> bool {
        self.raw.len() <= 2
    }

    /// The items, in order.
    pub(super) fn items(&self) -> Items<'a> {
        Items::new(self.raw, self.offset)
    }
}

impl<'a> Dict<'a> {
    /// The raw bytes, from the `d` through the `e`: what the info hash is computed over.
    pub(super) fn raw(&self) -> &'a [u8] {
        self.raw
    }

    /// The keys and their values, in the order they are written.
    pub(super) fn entries(&self) -> Entries<'a> {
        Entries {
            items: Items::new(self.raw, self.offset),
        }
    }
}

/// The values directly inside a list or a dictionary (for a dictionary, keys and values
/// alternately). Each item is checked again as it is read, so a fault would come out as an
/// error, not a panic, though [`parse`] has already found none.
#[derive(Debug, Clone)]
pub(super) struct Items<'a> {
    raw: &'a [u8],
    offset: usize,
    /// Where the next item starts in `raw`, or `None` once the end or an error was reached.
    position: Option<usize>,
}

impl<'a> Items<'a> {
    fn new(raw: &'a [u8], offset: usize) -> Self {
        Self {
            raw,
            offset,
            position: Some(1),
        }
    }

    /// The offset in the input that the reader has reached.
    fn input_offset(&self) -> usize {
        self.offset + self.position.unwrap_or(self.raw.len())
    }
}

impl<'a> Iterator for Items<'a> {
    type Item = Result<Value<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let position = self.position?;
        // The last byte is the container's own `e`.
        if position + 1 >= self.raw.len() {
            self.position = None;
            return None;
        }
        match read_value(self.raw, position, self.offset) {
            Ok(read) => {
                self.position = Some(read.end);
                Some(Ok(read.value))
            }
            Err(error) => {
                self.position = None;
                Some(Err(error))
            }
        }
    }
}

/// The entries of a dictionary.
#[derive(Debug, Clone)]
pub(super) struct Entries<'a> {
    items: Items<'a>,
}

impl<'a> Iterator for Entries<'a> {
    type Item = Result<(&'a [u8], Value<'a>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let offset = self.items.input_offset();
        let key = match self.items.next()? {
            Ok(Value::Bytes(key)) => key,
            Ok(_) => return Some(Err(Error::Syntax { offset })),
            Err(error) => return Some(Err(error)),
        };
        let offset = self.items.input_offset();
        Some(match self.items.next() {
            Some(Ok(value)) => Ok((key, value)),
            Some(Err(error)) => Err(error),
            None => Err(Error::Syntax { offset }),
        })
    }
}

/// A value read from a slice of the input.
struct Read<'a> {
    value: Value<'a>,
    /// Where the value ends in the slice: just past its last byte.
    end: usize,
    odd_keys: bool,
}

/// Reads the value that starts at `start` in `bytes`, checking all of it. `base` is where `bytes`
/// starts in the input, for the offsets in errors and containers.
fn read_value(bytes: &[u8], start: usize, base: usize) -> Result<Read<'_>, Error> {
    let syntax = Error::Syntax {
        offset: base + start,
    };
    let scalar = |value, end| Read {
        value,
        end,
        odd_keys: false,
    };
    match bytes.get(start) {
        Some(b'i') => {
            let (value, end) = read_int(bytes, start).ok_or(syntax)?;
            Ok(scalar(Value::Int(value), end))
        }
        Some(b'0'..=b'9') => {
            let (text, end) = read_string(bytes, start).ok_or(syntax)?;
            Ok(scalar(Value::Bytes(text), end))
        }
        Some(&kind @ (b'l' | b'd')) => {
            let (end, odd_keys) = scan_container(bytes, start, base)?;
            let raw = bytes.get(start..end).ok_or(syntax)?;
            let offset = base + start;
            let value = if kind == b'l' {
                Value::List(List { raw, offset })
            } else {
                Value::Dict(Dict { raw, offset })
            };
            Ok(Read {
                value,
                end,
                odd_keys,
            })
        }
        _ => Err(syntax),
    }
}

/// One open list or dictionary while a container is scanned.
#[derive(Clone, Copy)]
struct Frame<'a> {
    dict: bool,
    /// The items read so far; in a dictionary, keys and values both count.
    items: usize,
    /// The last key read, in a dictionary.
    last_key: Option<&'a [u8]>,
}

impl Frame<'_> {
    const CLOSED: Frame<'static> = Frame {
        dict: false,
        items: 0,
        last_key: None,
    };

    fn expects_key(&self) -> bool {
        self.dict && self.items.is_multiple_of(2)
    }
}

/// The open lists and dictionaries, in a fixed array: scanning allocates nothing.
struct Stack<'a> {
    frames: [Frame<'a>; MAX_DEPTH],
    len: usize,
}

impl<'a> Stack<'a> {
    fn new() -> Self {
        Self {
            frames: [Frame::CLOSED; MAX_DEPTH],
            len: 0,
        }
    }

    /// Opens `frame`, unless [`MAX_DEPTH`] frames are open already.
    fn push(&mut self, frame: Frame<'a>) -> bool {
        let Some(slot) = self.frames.get_mut(self.len) else {
            return false;
        };
        *slot = frame;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<Frame<'a>> {
        self.len = self.len.checked_sub(1)?;
        self.frames.get(self.len).copied()
    }

    fn last(&self) -> Option<&Frame<'a>> {
        self.frames.get(self.len.checked_sub(1)?)
    }

    fn last_mut(&mut self) -> Option<&mut Frame<'a>> {
        self.frames.get_mut(self.len.checked_sub(1)?)
    }
}

/// Checks the list or dictionary that starts at `start` in `bytes`, in a loop over an explicit
/// stack. Returns where it ends (just past its `e`), and whether a dictionary in it has keys out
/// of order or repeated.
fn scan_container(bytes: &[u8], start: usize, base: usize) -> Result<(usize, bool), Error> {
    let opens = |byte: u8| Frame {
        dict: byte == b'd',
        items: 0,
        last_key: None,
    };
    let first = *bytes
        .get(start)
        .filter(|byte| matches!(byte, b'l' | b'd'))
        .ok_or(Error::Syntax {
            offset: base + start,
        })?;
    let mut stack = Stack::new();
    if !stack.push(opens(first)) {
        return Err(Error::TooDeep {
            offset: base + start,
        });
    }
    let mut odd_keys = false;
    let mut position = start + 1;
    loop {
        let syntax = Error::Syntax {
            offset: base + position,
        };
        let expects_key = stack.last().ok_or(syntax)?.expects_key();
        let byte = *bytes.get(position).ok_or(syntax)?;
        match byte {
            b'l' | b'd' if !expects_key => {
                if !stack.push(opens(byte)) {
                    return Err(Error::TooDeep {
                        offset: base + position,
                    });
                }
                position += 1;
            }
            b'e' => {
                let frame = stack.pop().ok_or(syntax)?;
                if frame.dict && !frame.items.is_multiple_of(2) {
                    // A key without its value.
                    return Err(syntax);
                }
                position += 1;
                match stack.last_mut() {
                    Some(parent) => parent.items += 1,
                    None => return Ok((position, odd_keys)),
                }
            }
            b'i' if !expects_key => {
                let (_, end) = read_int(bytes, position).ok_or(syntax)?;
                position = end;
                stack.last_mut().ok_or(syntax)?.items += 1;
            }
            b'0'..=b'9' => {
                let (text, end) = read_string(bytes, position).ok_or(syntax)?;
                let frame = stack.last_mut().ok_or(syntax)?;
                if expects_key {
                    odd_keys |= frame.last_key.is_some_and(|last| text <= last);
                    frame.last_key = Some(text);
                }
                frame.items += 1;
                position = end;
            }
            _ => return Err(syntax),
        }
    }
}

/// Reads the integer whose `i` is at `start`: an optional `-`, digits and an `e`, as
/// libtransmission's `ParseInt` does (T/variant-benc.cc:49-85). A leading zero is refused, but
/// `i-0e` is read as 0, and the value must fit in an `i64`. Returns the value and where it ends.
fn read_int(bytes: &[u8], start: usize) -> Option<(i64, usize)> {
    let mut position = start + 1;
    let negative = bytes.get(position) == Some(&b'-');
    if negative {
        position += 1;
    }
    let digits_start = position;
    let mut value: i64 = 0;
    while let Some(&digit) = bytes.get(position).filter(|byte| byte.is_ascii_digit()) {
        let digit = i64::from(digit - b'0');
        value = value.checked_mul(10)?;
        // Built on the side of the sign, so that `i64::MIN` fits.
        value = if negative {
            value.checked_sub(digit)?
        } else {
            value.checked_add(digit)?
        };
        position += 1;
    }
    let digits = position - digits_start;
    let leading_zero = digits > 1 && bytes.get(digits_start) == Some(&b'0');
    if digits == 0 || leading_zero || bytes.get(position) != Some(&b'e') {
        return None;
    }
    Some((value, position + 1))
}

/// Reads the byte string whose length starts at `start`: decimal digits, a `:` and that many
/// bytes, as libtransmission's `ParseString` does (T/variant-benc.cc:93-125). Leading zeros in
/// the length are allowed there, so they are here. Returns the bytes and where they end.
fn read_string(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let mut position = start;
    let mut length: usize = 0;
    while let Some(&digit) = bytes.get(position).filter(|byte| byte.is_ascii_digit()) {
        length = length
            .checked_mul(10)?
            .checked_add(usize::from(digit - b'0'))?;
        position += 1;
    }
    if position == start || bytes.get(position) != Some(&b':') || length >= MAX_STRING_BYTES {
        return None;
    }
    let text_start = position + 1;
    let text_end = text_start.checked_add(length)?;
    let text = bytes.get(text_start..text_end)?;
    Some((text, text_end))
}

#[cfg(test)]
mod tests {
    use super::{Dict, Document, Error, MAX_DEPTH, MAX_STRING_BYTES, Value, parse};

    fn root(input: &[u8]) -> Value<'_> {
        parse(input).unwrap().root
    }

    fn dict(input: &[u8]) -> Dict<'_> {
        match root(input) {
            Value::Dict(dict) => dict,
            other => panic!("not a dictionary: {other:?}"),
        }
    }

    fn syntax_at(input: &[u8]) -> usize {
        match parse(input) {
            Err(Error::Syntax { offset }) => offset,
            other => panic!("{:?}: {other:?}", String::from_utf8_lossy(input)),
        }
    }

    #[test]
    fn integers_are_read_as_libtransmission_reads_them() {
        assert_eq!(root(b"i0e"), Value::Int(0));
        assert_eq!(root(b"i42e"), Value::Int(42));
        assert_eq!(root(b"i-42e"), Value::Int(-42));
        // libtransmission refuses `i-03e` but not `i-0e`.
        assert_eq!(root(b"i-0e"), Value::Int(0));
        assert_eq!(root(b"i9223372036854775807e"), Value::Int(i64::MAX));
        assert_eq!(root(b"i-9223372036854775808e"), Value::Int(i64::MIN));

        for bad in [
            &b"i03e"[..],
            b"i00e",
            b"i-03e",
            b"i-00e",
            b"i-e",
            b"ie",
            b"i+3e",
            b"i3",
            b"i3x",
            b"i 3e",
            b"i9223372036854775808e",
            b"i-9223372036854775809e",
        ] {
            assert_eq!(syntax_at(bad), 0, "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn strings_are_read_as_libtransmission_reads_them() {
        assert_eq!(root(b"0:"), Value::Bytes(b""));
        assert_eq!(root(b"4:spam"), Value::Bytes(b"spam"));
        // A leading zero in the length is allowed (T/variant-benc.cc:103-108).
        assert_eq!(root(b"04:spam"), Value::Bytes(b"spam"));
        assert_eq!(root(b"2:\0\xff"), Value::Bytes(b"\0\xff"));

        for bad in [&b"5:spam"[..], b"4spam", b"4", b"99999999999999999999999:x"] {
            assert_eq!(syntax_at(bad), 0, "{:?}", String::from_utf8_lossy(bad));
        }
        let too_long = format!("{MAX_STRING_BYTES}:");
        assert_eq!(syntax_at(too_long.as_bytes()), 0);
    }

    #[test]
    fn nesting_is_read_to_31_levels_and_refused_at_32() {
        let nested = |depth: usize| {
            let mut input = b"l".repeat(depth);
            input.extend(b"e".repeat(depth));
            input
        };
        assert!(parse(&nested(MAX_DEPTH)).is_ok());
        assert_eq!(
            parse(&nested(MAX_DEPTH + 1)).unwrap_err(),
            Error::TooDeep { offset: MAX_DEPTH }
        );

        // Dictionaries count the same.
        let mut input = b"d1:a".repeat(MAX_DEPTH);
        input.extend(b"i1ee");
        input.extend(b"e".repeat(MAX_DEPTH - 1));
        assert!(parse(&input).is_ok());
        let mut input = b"d1:a".repeat(MAX_DEPTH + 1);
        input.extend(b"i1ee");
        input.extend(b"e".repeat(MAX_DEPTH));
        assert_eq!(
            parse(&input).unwrap_err(),
            Error::TooDeep {
                offset: 4 * MAX_DEPTH
            }
        );
    }

    #[test]
    fn keys_out_of_order_or_repeated_are_noted() {
        let odd = |input: &[u8]| parse(input).unwrap().odd_keys;

        assert!(!odd(b"d1:ai1e1:bi2ee"));
        assert!(!odd(b"d0:i0e1:ai1e2:aai2ee"));
        assert!(odd(b"d1:bi1e1:ai2ee"));
        assert!(odd(b"d1:ai1e1:ai2ee"));
        // Anywhere in the document, and per dictionary: keys of sibling dictionaries do not
        // compare.
        assert!(odd(b"d1:ad1:bi1e1:ai2eee"));
        assert!(odd(b"ld1:ai1e1:ai1eee"));
        assert!(!odd(b"ld1:bi1eed1:ai1eee"));
        // Bytes compare unsigned, as bencode sorts them.
        assert!(!odd(b"d1:ai1e1:\xffi2ee"));
    }

    #[test]
    fn malformed_containers_are_refused_where_they_break() {
        // Keys must be byte strings; libtransmission would take the next value as a key.
        assert_eq!(syntax_at(b"di1ei2ee"), 1);
        assert_eq!(syntax_at(b"dle1:ae"), 1);
        // A key without a value.
        assert_eq!(syntax_at(b"d1:ae"), 4);
        // Cut short.
        assert_eq!(syntax_at(b""), 0);
        assert_eq!(syntax_at(b"l"), 1);
        assert_eq!(syntax_at(b"d1:a"), 4);
        assert_eq!(syntax_at(b"li1e"), 4);
        // A stray `e` or byte.
        assert_eq!(syntax_at(b"e"), 0);
        assert_eq!(syntax_at(b"lxe"), 1);
        assert_eq!(syntax_at(b"x"), 0);
    }

    #[test]
    fn what_follows_the_first_value_is_ignored() {
        assert_eq!(root(b"i1etrailing"), Value::Int(1));
        assert!(parse(b"d1:ai1eeee").is_ok());
    }

    #[test]
    fn a_dictionary_gives_its_entries_and_raw_bytes() {
        let input = b"d4:infod1:ai1e1:bl1:xi2eee4:zzzz0:e";
        let Document { root, odd_keys } = parse(input).unwrap();
        assert!(!odd_keys);
        let Value::Dict(top) = root else {
            panic!("not a dictionary")
        };
        assert_eq!(top.raw(), input);

        let entries: Vec<_> = top.entries().collect::<Result<_, _>>().unwrap();
        assert_eq!(entries.len(), 2);
        let (key, Value::Dict(info)) = entries[0] else {
            panic!("info is not a dictionary")
        };
        assert_eq!(key, b"info");
        assert_eq!(info.raw(), b"d1:ai1e1:bl1:xi2eee");
        assert_eq!(entries[1], (&b"zzzz"[..], Value::Bytes(b"")));

        let inner: Vec<_> = info.entries().collect::<Result<_, _>>().unwrap();
        assert_eq!(inner[0], (&b"a"[..], Value::Int(1)));
        let (_, Value::List(list)) = inner[1] else {
            panic!("b is not a list")
        };
        assert!(!list.is_empty());
        let items: Vec<_> = list.items().collect::<Result<_, _>>().unwrap();
        assert_eq!(items, [Value::Bytes(b"x"), Value::Int(2)]);
    }

    #[test]
    fn empty_containers_have_no_items() {
        let Value::List(list) = root(b"le") else {
            panic!("not a list")
        };
        assert!(list.is_empty());
        assert_eq!(list.items().count(), 0);
        assert_eq!(dict(b"de").entries().count(), 0);
    }
}
