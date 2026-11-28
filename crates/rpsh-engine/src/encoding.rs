//! Text encodings at ConsoleHost and native object-pipeline boundaries.
use rpsh_runtime::{RuntimeError, Value};
use std::fmt;
use std::io::{self, Write};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ascii,
    Latin1,
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
}

impl Kind {
    fn details(self) -> (i32, &'static str, &'static str, &'static str) {
        match self {
            Self::Ascii => (
                20127,
                "us-ascii",
                "US-ASCII",
                "System.Text.ASCIIEncoding+ASCIIEncodingSealed",
            ),
            Self::Latin1 => (
                28591,
                "iso-8859-1",
                "Western European (ISO)",
                "System.Text.Latin1Encoding+Latin1EncodingSealed",
            ),
            Self::Utf8 => (
                65001,
                "utf-8",
                "Unicode (UTF-8)",
                "System.Text.UTF8Encoding+UTF8EncodingSealed",
            ),
            Self::Utf16Le => (1200, "utf-16", "Unicode", "System.Text.UnicodeEncoding"),
            Self::Utf16Be => (
                1201,
                "utf-16BE",
                "Unicode (Big-Endian)",
                "System.Text.UnicodeEncoding",
            ),
            Self::Utf32Le => (
                12000,
                "utf-32",
                "Unicode (UTF-32)",
                "System.Text.UTF32Encoding",
            ),
            Self::Utf32Be => (
                12001,
                "utf-32BE",
                "Unicode (UTF-32 Big-Endian)",
                "System.Text.UTF32Encoding",
            ),
        }
    }

    fn from_code_page(page: i32) -> Option<Self> {
        match page {
            20127 => Some(Self::Ascii),
            28591 => Some(Self::Latin1),
            65001 => Some(Self::Utf8),
            1200 => Some(Self::Utf16Le),
            1201 => Some(Self::Utf16Be),
            12000 => Some(Self::Utf32Le),
            12001 => Some(Self::Utf32Be),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Encoding {
    kind: Kind,
    bom: bool,
    class: Option<&'static str>,
}

pub(super) const UTF8: Encoding = Encoding {
    kind: Kind::Utf8,
    bom: false,
    class: None,
};
static CONSOLE: Mutex<Encoding> = Mutex::new(Encoding {
    class: Some("System.Text.ConsoleEncoding"),
    ..UTF8
});

impl Encoding {
    fn marked(kind: Kind) -> Self {
        Self {
            kind,
            bom: !matches!(kind, Kind::Ascii | Kind::Latin1),
            class: None,
        }
    }

    pub(super) fn property(name: &str) -> Option<Self> {
        Some(match name {
            "ascii" => Self::marked(Kind::Ascii),
            "latin1" => Self::marked(Kind::Latin1),
            "utf8" => Self::marked(Kind::Utf8),
            "default" => UTF8,
            "unicode" => Self::marked(Kind::Utf16Le),
            "bigendianunicode" => Self::marked(Kind::Utf16Be),
            "utf32" => Self::marked(Kind::Utf32Le),
            _ => return None,
        })
    }

    pub(super) fn by_name(value: &Value) -> Option<Self> {
        let kind = match value {
            Value::Int32(page) => Kind::from_code_page(*page)?,
            Value::String(text) => match text.to_string_lossy().to_ascii_lowercase().as_str() {
                "ascii" | "us-ascii" => Kind::Ascii,
                "iso-8859-1" | "latin1" => Kind::Latin1,
                "utf-8" => Kind::Utf8,
                "utf-16" | "unicode" | "utf-16le" => Kind::Utf16Le,
                "utf-16be" | "unicodefffe" => Kind::Utf16Be,
                "utf-32" | "utf-32le" => Kind::Utf32Le,
                "utf-32be" => Kind::Utf32Be,
                _ => return None,
            },
            _ => return None,
        };
        Some(Self::marked(kind))
    }

    pub(super) fn new_utf8(bom: bool) -> Self {
        Self {
            bom,
            class: Some("System.Text.UTF8Encoding"),
            ..UTF8
        }
    }

    pub(super) fn value(self) -> Value {
        let (page, web, label, standard_class) = self.kind.details();
        let class = self.class.unwrap_or(standard_class);
        Value::object(vec![
            ("__TypeName".into(), Value::String(class.into())),
            ("__Text".into(), Value::String(class.into())),
            ("__EncodingPreamble".into(), Value::Bool(self.bom)),
            ("WebName".into(), Value::String(web.into())),
            ("CodePage".into(), Value::Int32(page)),
            ("EncodingName".into(), Value::String(label.into())),
        ])
    }

    pub(super) fn from_value(value: &Value) -> Option<Self> {
        let Value::Object(entries) = value else {
            return None;
        };
        Self::from_entries(entries)
    }

    pub(super) fn from_entries(entries: &[(String, Value)]) -> Option<Self> {
        let field = |key| {
            entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value)
        };
        let Value::Int32(page) = field("CodePage")? else {
            return None;
        };
        let Value::Bool(bom) = field("__EncodingPreamble")? else {
            return None;
        };
        let kind = Kind::from_code_page(*page)?;
        let class = match field("__TypeName")?.to_string().as_str() {
            "System.Text.ConsoleEncoding" => Some("System.Text.ConsoleEncoding"),
            "System.Text.UTF8Encoding" => Some("System.Text.UTF8Encoding"),
            standard if standard == kind.details().3 => None,
            _ => return None,
        };
        Some(Self {
            kind,
            bom: *bom,
            class,
        })
    }

    pub(super) fn preamble(self) -> &'static [u8] {
        if !self.bom {
            return &[];
        }
        match self.kind {
            Kind::Utf8 => &[0xef, 0xbb, 0xbf],
            Kind::Utf16Le => &[0xff, 0xfe],
            Kind::Utf16Be => &[0xfe, 0xff],
            Kind::Utf32Le => &[0xff, 0xfe, 0, 0],
            Kind::Utf32Be => &[0, 0, 0xfe, 0xff],
            Kind::Latin1 | Kind::Ascii => &[],
        }
    }

    pub(super) fn encode(self, text: &str, bytes: &mut Vec<u8>) {
        match self.kind {
            Kind::Utf8 => bytes.extend_from_slice(text.as_bytes()),
            Kind::Ascii => bytes.extend(
                text.encode_utf16()
                    .map(|unit| if unit < 128 { unit as u8 } else { b'?' }),
            ),
            Kind::Latin1 => bytes.extend(text.encode_utf16().map(latin1_byte)),
            Kind::Utf16Le => bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes)),
            Kind::Utf16Be => bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes)),
            Kind::Utf32Le => bytes.extend(text.chars().flat_map(|ch| (ch as u32).to_le_bytes())),
            Kind::Utf32Be => bytes.extend(text.chars().flat_map(|ch| (ch as u32).to_be_bytes())),
        }
    }

    pub(super) fn unit_width(self) -> usize {
        match self.kind {
            Kind::Utf16Le | Kind::Utf16Be => 2,
            Kind::Utf32Le | Kind::Utf32Be => 4,
            _ => 1,
        }
    }

    pub(super) fn newline(self, unit: &[u8]) -> Option<u8> {
        let byte = match (self.kind, unit) {
            (_, [delimiter])
            | (Kind::Utf16Le, [delimiter, 0])
            | (Kind::Utf16Be, [0, delimiter])
            | (Kind::Utf32Le, [delimiter, 0, 0, 0])
            | (Kind::Utf32Be, [0, 0, 0, delimiter]) => *delimiter,
            _ => return None,
        };
        matches!(byte, b'\r' | b'\n').then_some(byte)
    }

    pub(super) fn decode(self, bytes: &[u8]) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        match self.kind {
            Kind::Utf8 => String::from_utf8_lossy(bytes),
            Kind::Ascii => Cow::Owned(
                bytes
                    .iter()
                    .map(|&byte| char::from(if byte < 128 { byte } else { b'?' }))
                    .collect(),
            ),
            Kind::Latin1 => Cow::Owned(bytes.iter().map(|&byte| char::from(byte)).collect()),
            Kind::Utf16Le | Kind::Utf16Be => {
                let (pairs, rest) = bytes.as_chunks::<2>();
                let convert = if self.kind == Kind::Utf16Le {
                    u16::from_le_bytes
                } else {
                    u16::from_be_bytes
                };
                let mut text: String = char::decode_utf16(pairs.iter().copied().map(convert))
                    .map(|ch| ch.unwrap_or(char::REPLACEMENT_CHARACTER))
                    .collect();
                if !rest.is_empty() {
                    text.push(char::REPLACEMENT_CHARACTER);
                }
                Cow::Owned(text)
            }
            Kind::Utf32Le | Kind::Utf32Be => {
                let (quads, rest) = bytes.as_chunks::<4>();
                let convert = if self.kind == Kind::Utf32Le {
                    u32::from_le_bytes
                } else {
                    u32::from_be_bytes
                };
                let mut text: String = quads
                    .iter()
                    .copied()
                    .map(|quad| {
                        char::from_u32(convert(quad)).unwrap_or(char::REPLACEMENT_CHARACTER)
                    })
                    .collect();
                if !rest.is_empty() {
                    text.push(char::REPLACEMENT_CHARACTER);
                }
                Cow::Owned(text)
            }
        }
    }
}

pub(super) fn variable_name(name: &str) -> Option<&'static str> {
    ["OutputEncoding", "PSApplicationOutputEncoding"]
        .into_iter()
        .find(|variable| name.eq_ignore_ascii_case(variable))
}

pub(super) fn console() -> Encoding {
    *CONSOLE.lock().expect("copy console encoding")
}

pub(super) fn set_console(value: &Value) -> Result<(), RuntimeError> {
    let encoding = Encoding::from_value(value).ok_or(RuntimeError::Script {
        id: "ExceptionWhenSetting",
        category: "NotSpecified: (:) [], SetValueInvocationException".into(),
        exception_type: "System.Management.Automation.SetValueInvocationException",
        message: if value.is_null_like() { "Exception setting \"OutputEncoding\": \"Value cannot be null. (Parameter 'value')\"".into() }
            else { format!("Exception setting \"OutputEncoding\": \"Cannot convert the \"{value}\" value of type \"{}\" to type \"System.Text.Encoding\".\"", value.type_name()) },
    })?;
    *CONSOLE.lock().expect("set console encoding") = encoding;
    Ok(())
}

pub(super) fn write_host(
    encoding: Encoding,
    out: &mut dyn Write,
    text: fmt::Arguments<'_>,
) -> io::Result<()> {
    if encoding.kind == Kind::Utf8 {
        return out.write_fmt(text);
    }
    let mut bytes = Vec::new();
    encoding.encode(&text.to_string(), &mut bytes);
    out.write_all(&bytes)
}

// The default .NET Latin1 encoder applies a best-fit fallback per UTF-16 unit.
// Mapping from the pinned reference: Encoding.Latin1.GetBytes([char[]](256..65535)).
fn latin1_byte(unit: u16) -> u8 {
    match unit {
        0..=0xff => unit as u8,
        0xff01..=0xff5e => (unit - 0xfee0) as u8,
        0x2000..=0x2006 => 0x20,
        0x02ba | 0x030e | 0x201c | 0x201d | 0x201e => 0x22,
        0x02b9 | 0x02bc | 0x02c8 | 0x2018 | 0x2019 | 0x2032 => 0x27,
        0x201a => 0x2c,
        0x2010 | 0x2011 | 0x2013 | 0x2014 => 0x2d,
        0x2022 | 0x2026 => 0x2e,
        0x2039 => 0x3c,
        0x203a => 0x3e,
        0x0100 | 0x0102 | 0x0104 | 0x01cd | 0x01de => 0x41,
        0x0106 | 0x0108 | 0x010a | 0x010c => 0x43,
        0x010e | 0x0110 | 0x0189 => 0x44,
        0x0112 | 0x0114 | 0x0116 | 0x0118 | 0x011a => 0x45,
        0x0191 => 0x46,
        0x011c | 0x011e | 0x0120 | 0x0122 | 0x01e4 | 0x01e6 => 0x47,
        0x0124 | 0x0126 => 0x48,
        0x0128 | 0x012a | 0x012c | 0x012e | 0x0130 | 0x0197 | 0x01cf => 0x49,
        0x0134 => 0x4a,
        0x0136 | 0x01e8 => 0x4b,
        0x0139 | 0x013b | 0x013d | 0x0141 => 0x4c,
        0x0143 | 0x0145 | 0x0147 => 0x4e,
        0x014c | 0x014e | 0x0150 | 0x0152 | 0x019f | 0x01a0 | 0x01d1 | 0x01ea | 0x01ec => 0x4f,
        0x0154 | 0x0156 | 0x0158 => 0x52,
        0x015a | 0x015c | 0x015e | 0x0160 => 0x53,
        0x0162 | 0x0164 | 0x0166 | 0x01ae | 0x2122 => 0x54,
        0x0168 | 0x016a | 0x016c | 0x016e | 0x0170 | 0x0172 | 0x01af | 0x01d3 | 0x01d5 | 0x01d7
        | 0x01d9 | 0x01db => 0x55,
        0x0174 => 0x57,
        0x0176 | 0x0178 => 0x59,
        0x0179 | 0x017b | 0x017d => 0x5a,
        0x02c4 | 0x02c6 | 0x0302 => 0x5e,
        0x02cd | 0x0331 | 0x0332 => 0x5f,
        0x02cb | 0x0300 | 0x2035 => 0x60,
        0x0101 | 0x0103 | 0x0105 | 0x01ce | 0x01df => 0x61,
        0x0180 => 0x62,
        0x0107 | 0x0109 | 0x010b | 0x010d => 0x63,
        0x010f | 0x0111 => 0x64,
        0x0113 | 0x0115 | 0x0117 | 0x0119 | 0x011b => 0x65,
        0x0192 => 0x66,
        0x011d | 0x011f | 0x0121 | 0x0123 | 0x01e5 | 0x01e7 | 0x0261 => 0x67,
        0x0125 | 0x0127 => 0x68,
        0x0129 | 0x012b | 0x012d | 0x012f | 0x0131 | 0x01d0 => 0x69,
        0x0135 | 0x01f0 => 0x6a,
        0x0137 | 0x01e9 => 0x6b,
        0x013a | 0x013c | 0x013e | 0x0142 | 0x019a => 0x6c,
        0x0144 | 0x0146 | 0x0148 => 0x6e,
        0x014d | 0x014f | 0x0151 | 0x0153 | 0x01a1 | 0x01d2 | 0x01eb | 0x01ed => 0x6f,
        0x0155 | 0x0157 | 0x0159 => 0x72,
        0x015b | 0x015d | 0x015f | 0x0161 => 0x73,
        0x0163 | 0x0165 | 0x0167 | 0x01ab => 0x74,
        0x0169 | 0x016b | 0x016d | 0x016f | 0x0171 | 0x0173 | 0x01b0 | 0x01d4 | 0x01d6 | 0x01d8
        | 0x01da | 0x01dc => 0x75,
        0x0175 => 0x77,
        0x0177 => 0x79,
        0x017a | 0x017c | 0x017e | 0x01b6 => 0x7a,
        0x02dc | 0x0303 => 0x7e,
        _ => b'?',
    }
}
