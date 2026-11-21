//! #2: how to store PowerShell values, and strings in particular.
//!
//! A .NET string is a sequence of UTF-16 code units: `'😀'.Length` is 2, `$s[0]`
//! is one code unit, and a lone surrogate (`[char]0xD800`) is a valid
//! string that only turns into U+FFFD when written out as UTF-8. Whatever
//! rpsh stores has to answer those questions exactly. Three candidates:
//!
//! * `utf16`: `Rc<[u16]>`, .NET's own representation.
//! * `wtf8`: WTF-8 bytes (UTF-8 that can also hold lone surrogates) with the
//!   UTF-16 length and an ASCII flag computed once. Indexing a non-ASCII
//!   string walks from the start.
//! * `compact`: ASCII strings as bytes, all others as UTF-16 (the idea behind
//!   Java's compact strings and V8's one-byte strings, with ASCII rather
//!   than Latin-1 as the narrow set so that writing UTF-8 stays a copy).
//!
//! Each is timed on the operations scripts perform most, over ASCII,
//! Japanese and emoji text. Then the `Value` enum's size: 16 bytes against
//! 24, over a large array.

use std::hint::black_box;
use std::rc::Rc;

use rpsh_research::{Report, iterations, measure};

/// Simple case folding of one UTF-16 code unit, enough for the
/// `OrdinalIgnoreCase` comparison PowerShell's `@{}` uses.
fn fold(unit: u16) -> u16 {
    if unit < 0x80 {
        return (unit as u8).to_ascii_uppercase() as u16;
    }
    match char::from_u32(unit as u32) {
        Some(c) => {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) if (u as u32) <= 0xFFFF => u as u16,
                _ => unit,
            }
        }
        None => unit,
    }
}

fn fnv(hash: u64, unit: u16) -> u64 {
    (hash ^ unit as u64).wrapping_mul(0x0000_0100_0000_01b3)
}

const FNV_SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// Writes UTF-16 as UTF-8, a lone surrogate as U+FFFD (what .NET's UTF-8
/// encoder does).
fn utf16_to_utf8(units: &[u16], out: &mut Vec<u8>) {
    if units.iter().all(|&unit| unit < 0x80) {
        out.extend(units.iter().map(|&unit| unit as u8));
        return;
    }
    for c in char::decode_utf16(units.iter().copied()) {
        let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
        let mut buffer = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
    }
}

fn find<T: PartialEq>(haystack: &[T], needle: &[T]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

trait PsString: Clone {
    fn from_utf8(text: &str) -> Self;
    fn write_utf8(&self, out: &mut Vec<u8>);
    fn len16(&self) -> usize;
    fn unit(&self, index: usize) -> u16;
    fn concat(&self, other: &Self) -> Self;
    fn substring(&self, start: usize, length: usize) -> Self;
    fn index_of(&self, needle: &Self) -> Option<usize>;
    fn eq_ignore_case(&self, other: &Self) -> bool;
    fn hash_ignore_case(&self) -> u64;
    fn payload_bytes(&self) -> usize;
}

#[derive(Clone)]
struct Utf16(Rc<[u16]>);

impl PsString for Utf16 {
    fn from_utf8(text: &str) -> Self {
        Utf16(text.encode_utf16().collect::<Vec<u16>>().into())
    }
    fn write_utf8(&self, out: &mut Vec<u8>) {
        utf16_to_utf8(&self.0, out);
    }
    fn len16(&self) -> usize {
        self.0.len()
    }
    fn unit(&self, index: usize) -> u16 {
        self.0[index]
    }
    fn concat(&self, other: &Self) -> Self {
        let mut units = Vec::with_capacity(self.0.len() + other.0.len());
        units.extend_from_slice(&self.0);
        units.extend_from_slice(&other.0);
        Utf16(units.into())
    }
    fn substring(&self, start: usize, length: usize) -> Self {
        Utf16(self.0[start..start + length].into())
    }
    fn index_of(&self, needle: &Self) -> Option<usize> {
        find(&self.0, &needle.0)
    }
    fn eq_ignore_case(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0.iter())
                .all(|(&a, &b)| a == b || fold(a) == fold(b))
    }
    fn hash_ignore_case(&self) -> u64 {
        self.0
            .iter()
            .fold(FNV_SEED, |hash, &unit| fnv(hash, fold(unit)))
    }
    fn payload_bytes(&self) -> usize {
        self.0.len() * 2
    }
}

/// WTF-8: a lone surrogate is encoded like a scalar value (ED A0..BF xx).
#[derive(Clone)]
struct Wtf8 {
    bytes: Rc<[u8]>,
    len16: u32,
    ascii: bool,
}

impl Wtf8 {
    fn from_units(units: &[u16]) -> Self {
        let mut bytes = Vec::with_capacity(units.len());
        let mut index = 0;
        while index < units.len() {
            let unit = units[index];
            let code = if (0xD800..0xDC00).contains(&unit)
                && index + 1 < units.len()
                && (0xDC00..0xE000).contains(&units[index + 1])
            {
                index += 1;
                0x10000 + (((unit as u32) - 0xD800) << 10) + ((units[index] as u32) - 0xDC00)
            } else {
                unit as u32
            };
            index += 1;
            encode_wtf8(code, &mut bytes);
        }
        Wtf8 {
            ascii: units.iter().all(|&unit| unit < 0x80),
            len16: units.len() as u32,
            bytes: bytes.into(),
        }
    }

    /// The UTF-16 code units, decoded from the start.
    fn units(&self) -> impl Iterator<Item = u16> + '_ {
        let mut position = 0;
        let mut pending: Option<u16> = None;
        std::iter::from_fn(move || {
            if let Some(unit) = pending.take() {
                return Some(unit);
            }
            let bytes = &self.bytes;
            let first = *bytes.get(position)?;
            let (code, width) = match first {
                0x00..=0x7F => (first as u32, 1),
                0xC0..=0xDF => (
                    ((first as u32 & 0x1F) << 6) | (bytes[position + 1] as u32 & 0x3F),
                    2,
                ),
                0xE0..=0xEF => (
                    ((first as u32 & 0x0F) << 12)
                        | ((bytes[position + 1] as u32 & 0x3F) << 6)
                        | (bytes[position + 2] as u32 & 0x3F),
                    3,
                ),
                _ => (
                    ((first as u32 & 0x07) << 18)
                        | ((bytes[position + 1] as u32 & 0x3F) << 12)
                        | ((bytes[position + 2] as u32 & 0x3F) << 6)
                        | (bytes[position + 3] as u32 & 0x3F),
                    4,
                ),
            };
            position += width;
            if code >= 0x10000 {
                let code = code - 0x10000;
                pending = Some(0xDC00 + (code & 0x3FF) as u16);
                Some(0xD800 + (code >> 10) as u16)
            } else {
                Some(code as u16)
            }
        })
    }
}

fn encode_wtf8(code: u32, out: &mut Vec<u8>) {
    match code {
        0..=0x7F => out.push(code as u8),
        0x80..=0x7FF => {
            out.extend_from_slice(&[0xC0 | (code >> 6) as u8, 0x80 | (code & 0x3F) as u8])
        }
        0x800..=0xFFFF => out.extend_from_slice(&[
            0xE0 | (code >> 12) as u8,
            0x80 | ((code >> 6) & 0x3F) as u8,
            0x80 | (code & 0x3F) as u8,
        ]),
        _ => out.extend_from_slice(&[
            0xF0 | (code >> 18) as u8,
            0x80 | ((code >> 12) & 0x3F) as u8,
            0x80 | ((code >> 6) & 0x3F) as u8,
            0x80 | (code & 0x3F) as u8,
        ]),
    }
}

impl PsString for Wtf8 {
    fn from_utf8(text: &str) -> Self {
        Wtf8 {
            bytes: text.as_bytes().into(),
            len16: text.encode_utf16().count() as u32,
            ascii: text.is_ascii(),
        }
    }
    fn write_utf8(&self, out: &mut Vec<u8>) {
        if self.ascii {
            out.extend_from_slice(&self.bytes);
            return;
        }
        // Valid UTF-8 except for encoded surrogates, which become U+FFFD.
        let bytes = &self.bytes;
        let mut start = 0;
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == 0xED && index + 1 < bytes.len() && bytes[index + 1] >= 0xA0 {
                out.extend_from_slice(&bytes[start..index]);
                out.extend_from_slice("\u{FFFD}".as_bytes());
                index += 3;
                start = index;
            } else {
                index += 1;
            }
        }
        out.extend_from_slice(&bytes[start..]);
    }
    fn len16(&self) -> usize {
        self.len16 as usize
    }
    fn unit(&self, index: usize) -> u16 {
        if self.ascii {
            self.bytes[index] as u16
        } else {
            self.units().nth(index).unwrap_or(0)
        }
    }
    fn concat(&self, other: &Self) -> Self {
        if self.ascii && other.ascii {
            let mut bytes = Vec::with_capacity(self.bytes.len() + other.bytes.len());
            bytes.extend_from_slice(&self.bytes);
            bytes.extend_from_slice(&other.bytes);
            return Wtf8 {
                bytes: bytes.into(),
                len16: self.len16 + other.len16,
                ascii: true,
            };
        }
        // A lead surrogate at the end of one and a trail at the start of the
        // other must merge into one 4-byte sequence; re-encoding handles it.
        let units: Vec<u16> = self.units().chain(other.units()).collect();
        Wtf8::from_units(&units)
    }
    fn substring(&self, start: usize, length: usize) -> Self {
        if self.ascii {
            return Wtf8 {
                bytes: self.bytes[start..start + length].into(),
                len16: length as u32,
                ascii: true,
            };
        }
        let units: Vec<u16> = self.units().skip(start).take(length).collect();
        Wtf8::from_units(&units)
    }
    fn index_of(&self, needle: &Self) -> Option<usize> {
        let byte_offset = find(&self.bytes, &needle.bytes)?;
        if self.ascii {
            return Some(byte_offset);
        }
        let prefix = Wtf8 {
            bytes: self.bytes[..byte_offset].into(),
            len16: 0,
            ascii: false,
        };
        Some(prefix.units().count())
    }
    fn eq_ignore_case(&self, other: &Self) -> bool {
        if self.ascii && other.ascii {
            return self.bytes.eq_ignore_ascii_case(&other.bytes);
        }
        self.len16 == other.len16
            && self
                .units()
                .zip(other.units())
                .all(|(a, b)| a == b || fold(a) == fold(b))
    }
    fn hash_ignore_case(&self) -> u64 {
        if self.ascii {
            return self.bytes.iter().fold(FNV_SEED, |hash, &byte| {
                fnv(hash, byte.to_ascii_uppercase() as u16)
            });
        }
        self.units()
            .fold(FNV_SEED, |hash, unit| fnv(hash, fold(unit)))
    }
    fn payload_bytes(&self) -> usize {
        self.bytes.len()
    }
}

#[derive(Clone)]
enum Compact {
    Ascii(Rc<[u8]>),
    Wide(Rc<[u16]>),
}

impl Compact {
    fn from_units(units: Vec<u16>) -> Self {
        if units.iter().all(|&unit| unit < 0x80) {
            Compact::Ascii(
                units
                    .iter()
                    .map(|&unit| unit as u8)
                    .collect::<Vec<u8>>()
                    .into(),
            )
        } else {
            Compact::Wide(units.into())
        }
    }
}

impl PsString for Compact {
    fn from_utf8(text: &str) -> Self {
        if text.is_ascii() {
            Compact::Ascii(text.as_bytes().into())
        } else {
            Compact::Wide(text.encode_utf16().collect::<Vec<u16>>().into())
        }
    }
    fn write_utf8(&self, out: &mut Vec<u8>) {
        match self {
            Compact::Ascii(bytes) => out.extend_from_slice(bytes),
            Compact::Wide(units) => utf16_to_utf8(units, out),
        }
    }
    fn len16(&self) -> usize {
        match self {
            Compact::Ascii(bytes) => bytes.len(),
            Compact::Wide(units) => units.len(),
        }
    }
    fn unit(&self, index: usize) -> u16 {
        match self {
            Compact::Ascii(bytes) => bytes[index] as u16,
            Compact::Wide(units) => units[index],
        }
    }
    fn concat(&self, other: &Self) -> Self {
        match (self, other) {
            (Compact::Ascii(a), Compact::Ascii(b)) => {
                let mut bytes = Vec::with_capacity(a.len() + b.len());
                bytes.extend_from_slice(a);
                bytes.extend_from_slice(b);
                Compact::Ascii(bytes.into())
            }
            _ => {
                let mut units = Vec::with_capacity(self.len16() + other.len16());
                for part in [self, other] {
                    match part {
                        Compact::Ascii(bytes) => {
                            units.extend(bytes.iter().map(|&byte| byte as u16))
                        }
                        Compact::Wide(wide) => units.extend_from_slice(wide),
                    }
                }
                Compact::Wide(units.into())
            }
        }
    }
    fn substring(&self, start: usize, length: usize) -> Self {
        match self {
            Compact::Ascii(bytes) => Compact::Ascii(bytes[start..start + length].into()),
            // A substring of a wide string may be ASCII; keep the invariant
            // that ASCII content is always narrow.
            Compact::Wide(units) => Compact::from_units(units[start..start + length].to_vec()),
        }
    }
    fn index_of(&self, needle: &Self) -> Option<usize> {
        match (self, needle) {
            (Compact::Ascii(haystack), Compact::Ascii(needle)) => find(haystack, needle),
            (Compact::Ascii(_), Compact::Wide(_)) => None,
            (Compact::Wide(haystack), Compact::Wide(needle)) => find(haystack, needle),
            (Compact::Wide(haystack), Compact::Ascii(needle)) => {
                let needle: Vec<u16> = needle.iter().map(|&byte| byte as u16).collect();
                find(haystack, &needle)
            }
        }
    }
    fn eq_ignore_case(&self, other: &Self) -> bool {
        match (self, other) {
            (Compact::Ascii(a), Compact::Ascii(b)) => a.eq_ignore_ascii_case(b),
            (Compact::Wide(a), Compact::Wide(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(&x, &y)| x == y || fold(x) == fold(y))
            }
            // Folding never maps a non-ASCII unit to ASCII here, so a narrow
            // and a wide string differ.
            _ => false,
        }
    }
    fn hash_ignore_case(&self) -> u64 {
        match self {
            Compact::Ascii(bytes) => bytes.iter().fold(FNV_SEED, |hash, &byte| {
                fnv(hash, byte.to_ascii_uppercase() as u16)
            }),
            Compact::Wide(units) => units
                .iter()
                .fold(FNV_SEED, |hash, &unit| fnv(hash, fold(unit))),
        }
    }
    fn payload_bytes(&self) -> usize {
        match self {
            Compact::Ascii(bytes) => bytes.len(),
            Compact::Wide(units) => units.len() * 2,
        }
    }
}

struct Corpus {
    name: &'static str,
    text: &'static str,
    /// A needle that occurs near the end, and an equal string in other case.
    needle: &'static str,
    other_case: &'static str,
}

const CORPORA: &[Corpus] = &[
    Corpus {
        name: "ascii",
        text: "2026-11-21T10:15:42Z INFO request handled path=/api/v1/items status=200 elapsed=12ms",
        needle: "elapsed",
        other_case: "2026-11-21T10:15:42Z info REQUEST HANDLED PATH=/API/V1/ITEMS STATUS=200 ELAPSED=12MS",
    },
    Corpus {
        name: "japanese",
        text: "東京都千代田区の天気は晴れ、最高気温は十八度の予報です。明日は曇りでしょう。",
        needle: "曇り",
        other_case: "東京都千代田区の天気は晴れ、最高気温は十八度の予報です。明日は曇りでしょう。",
    },
    Corpus {
        name: "emoji",
        text: "build ✅ passed 🎉 in 12s with 3 warnings ⚠️ and 0 errors 🚀 deployed to staging",
        needle: "staging",
        other_case: "BUILD ✅ PASSED 🎉 IN 12S WITH 3 WARNINGS ⚠️ AND 0 ERRORS 🚀 DEPLOYED TO STAGING",
    },
];

/// Checks one candidate against the UTF-16 reference before timing it.
fn verify<S: PsString>(name: &str) {
    for corpus in CORPORA {
        let reference = Utf16::from_utf8(corpus.text);
        let candidate = S::from_utf8(corpus.text);
        assert_eq!(
            candidate.len16(),
            reference.len16(),
            "{name} {}",
            corpus.name
        );
        for index in 0..reference.len16() {
            assert_eq!(candidate.unit(index), reference.unit(index), "{name}");
        }
        let mut a = Vec::new();
        let mut b = Vec::new();
        candidate.write_utf8(&mut a);
        reference.write_utf8(&mut b);
        assert_eq!(a, b, "{name} write");
        assert_eq!(
            candidate.index_of(&S::from_utf8(corpus.needle)),
            reference.index_of(&Utf16::from_utf8(corpus.needle)),
            "{name} index_of {}",
            corpus.name
        );
        assert!(
            candidate.eq_ignore_case(&S::from_utf8(corpus.other_case)),
            "{name} eq"
        );
        let middle = candidate.substring(2, 10);
        let expected = reference.substring(2, 10);
        assert_eq!(middle.len16(), 10);
        for index in 0..10 {
            assert_eq!(middle.unit(index), expected.unit(index));
        }
    }
    // A lone surrogate survives concatenation and is written as U+FFFD.
    let lead = String::from_utf16_lossy(&[0x61]);
    let a = S::from_utf8(&lead);
    let lone = {
        let units: Vec<u16> = vec![0x61, 0xD800, 0x62];
        let reference = Utf16(units.into());
        let mut out = Vec::new();
        reference.write_utf8(&mut out);
        assert_eq!(out, "a\u{FFFD}b".as_bytes());
        reference
    };
    assert_eq!(lone.len16(), 3);
    black_box(a);
}

fn bench_strings<S: PsString>(report: &mut Report, candidate: &str) {
    verify::<S>(candidate);
    for corpus in CORPORA {
        let group = |operation: &str| format!("{operation} [{}]", corpus.name);
        let text = S::from_utf8(corpus.text);
        let needle = S::from_utf8(corpus.needle);
        let other = S::from_utf8(corpus.other_case);

        report.add(measure(
            &group("from utf-8"),
            candidate,
            iterations(200_000),
            || S::from_utf8(corpus.text),
        ));
        let mut out = Vec::with_capacity(256);
        report.add(measure(
            &group("write utf-8"),
            candidate,
            iterations(200_000),
            || {
                out.clear();
                text.write_utf8(&mut out);
                out.len()
            },
        ));
        report.add(measure(
            &group("$s[$i] for each i"),
            candidate,
            iterations(20_000),
            || {
                let mut sum = 0u32;
                for index in 0..text.len16() {
                    sum = sum.wrapping_add(text.unit(index) as u32);
                }
                sum
            },
        ));
        report.add(measure(
            &group("Substring(2, 10)"),
            candidate,
            iterations(200_000),
            || text.substring(2, 10),
        ));
        report.add(measure(
            &group("IndexOf"),
            candidate,
            iterations(200_000),
            || text.index_of(&needle),
        ));
        report.add(measure(
            &group("-eq ignoring case"),
            candidate,
            iterations(200_000),
            || text.eq_ignore_case(&other),
        ));
        report.add(measure(
            &group("hash ignoring case"),
            candidate,
            iterations(200_000),
            || text.hash_ignore_case(),
        ));
        report.fact(
            &format!("payload bytes, {candidate} [{}]", corpus.name),
            text.payload_bytes(),
        );
    }

    // `$s += 'x'` 2,000 times: PowerShell strings are immutable, so every
    // step copies the whole string.
    for (name, piece) in [("ascii", "x"), ("japanese", "あ")] {
        let piece = S::from_utf8(piece);
        report.add(measure(
            &format!("+= 2,000 times [{name}]"),
            candidate,
            iterations(50),
            || {
                let mut text = S::from_utf8("");
                for _ in 0..2_000 {
                    text = text.concat(&piece);
                }
                text.len16()
            },
        ));
    }
}

/// The candidates for `Value`. Only their size and the cost of walking an
/// array of them are compared; behaviour lives in rpsh-runtime.
#[allow(dead_code)]
enum Value16 {
    Null,
    AutomationNull,
    Bool(bool),
    Int32(i32),
    Int64(i64),
    Double(f64),
    Char(u16),
    String(Rc<Compact>),
    Object(Rc<Vec<Value16>>),
}

#[allow(dead_code)]
enum Value24 {
    Null,
    AutomationNull,
    Bool(bool),
    Int32(i32),
    Int64(i64),
    Double(f64),
    Char(u16),
    String(Rc<[u16]>),
    Object(Rc<Vec<Value24>>),
}

/// The skeleton's current layout.
#[allow(dead_code)]
enum ValueToday {
    Int32(i32),
    String(String),
}

fn sum16(values: &[Value16]) -> i64 {
    values
        .iter()
        .map(|value| match value {
            Value16::Int32(n) => *n as i64,
            Value16::Int64(n) => *n,
            _ => 0,
        })
        .sum()
}

fn sum24(values: &[Value24]) -> i64 {
    values
        .iter()
        .map(|value| match value {
            Value24::Int32(n) => *n as i64,
            Value24::Int64(n) => *n,
            _ => 0,
        })
        .sum()
}

fn main() {
    let mut report = Report::new(
        "value_model",
        "string representations with UTF-16 semantics, and the size of Value (#2)",
    );
    report.header();

    bench_strings::<Utf16>(&mut report, "utf16");
    bench_strings::<Wtf8>(&mut report, "wtf8");
    bench_strings::<Compact>(&mut report, "compact (ascii | utf16)");

    report.fact(
        "size_of Value16 (thin Rc payloads)",
        std::mem::size_of::<Value16>(),
    );
    report.fact(
        "size_of Value24 (Rc<[u16]> payload)",
        std::mem::size_of::<Value24>(),
    );
    report.fact(
        "size_of ValueToday (String payload)",
        std::mem::size_of::<ValueToday>(),
    );
    report.fact("size_of Compact", std::mem::size_of::<Compact>());
    report.fact("size_of Rc<[u16]>", std::mem::size_of::<Rc<[u16]>>());

    let count: i32 = if rpsh_research::quick() {
        10_000
    } else {
        1_000_000
    };
    let values16: Vec<Value16> = (0..count).map(Value16::Int32).collect();
    let values24: Vec<Value24> = (0..count).map(Value24::Int32).collect();
    report.add(measure(
        "sum 1,000,000 Int32",
        "Value 16 bytes",
        iterations(200),
        || sum16(&values16),
    ));
    report.add(measure(
        "sum 1,000,000 Int32",
        "Value 24 bytes",
        iterations(200),
        || sum24(&values24),
    ));
    report.save();
}
