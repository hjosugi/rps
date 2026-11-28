//! Numeric literal parsing: base, suffix, multiplier and the .NET type.

use std::fmt;

use crate::{ParseError, Span};

#[derive(Debug, Clone)]
pub enum NumberLiteral {
    Int32(i32),
    Int64(i64),
    UInt32(u32),
    UInt64(u64),
    Int16(i16),
    UInt16(u16),
    Byte(u8),
    SByte(i8),
    Decimal(String),
    Double(f64),
    BigInteger(String),
}

impl PartialEq for NumberLiteral {
    fn eq(&self, other: &Self) -> bool {
        use NumberLiteral::*;
        match (self, other) {
            (Int32(a), Int32(b)) => a == b,
            (Int64(a), Int64(b)) => a == b,
            (UInt32(a), UInt32(b)) => a == b,
            (UInt64(a), UInt64(b)) => a == b,
            (Int16(a), Int16(b)) => a == b,
            (UInt16(a), UInt16(b)) => a == b,
            (Byte(a), Byte(b)) => a == b,
            (SByte(a), SByte(b)) => a == b,
            (Decimal(a), Decimal(b)) => a == b,
            (Double(a), Double(b)) => a.to_bits() == b.to_bits(),
            (BigInteger(a), BigInteger(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for NumberLiteral {}

impl NumberLiteral {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Int32(_) => "System.Int32",
            Self::Int64(_) => "System.Int64",
            Self::UInt32(_) => "System.UInt32",
            Self::UInt64(_) => "System.UInt64",
            Self::Int16(_) => "System.Int16",
            Self::UInt16(_) => "System.UInt16",
            Self::Byte(_) => "System.Byte",
            Self::SByte(_) => "System.SByte",
            Self::Decimal(_) => "System.Decimal",
            Self::Double(_) => "System.Double",
            Self::BigInteger(_) => "System.Numerics.BigInteger",
        }
    }

    pub fn is_int32(&self) -> Option<i32> {
        match self {
            Self::Int32(value) => Some(*value),
            _ => None,
        }
    }
}

impl fmt::Display for NumberLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int32(value) => write!(f, "{value}"),
            Self::Int64(value) => write!(f, "{value}"),
            Self::UInt32(value) => write!(f, "{value}"),
            Self::UInt64(value) => write!(f, "{value}"),
            Self::Int16(value) => write!(f, "{value}"),
            Self::UInt16(value) => write!(f, "{value}"),
            Self::Byte(value) => write!(f, "{value}"),
            Self::SByte(value) => write!(f, "{value}"),
            Self::Decimal(value) | Self::BigInteger(value) => f.write_str(value),
            Self::Double(value) => write!(f, "{value}"),
        }
    }
}

/// Parses a number candidate. `None` means the text is not numeric (a command
/// name such as `7z`); `Some(Err)` means a numeric form or range is invalid.
pub fn parse_number_literal(text: &str, span: Span) -> Option<Result<NumberLiteral, ParseError>> {
    let (negative, unsigned) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        (false, rest)
    } else {
        (false, text)
    };
    let lower = unsigned.to_ascii_lowercase();
    let multiplier = ["kb", "mb", "gb", "tb", "pb"]
        .iter()
        .find(|suffix| lower.ends_with(*suffix))
        .copied();
    let mut body = if multiplier.is_some() {
        &unsigned[..unsigned.len() - 2]
    } else {
        unsigned
    };
    let mut suffix = "";
    for candidate in [
        "ul", "lu", "uy", "yu", "us", "su", "l", "d", "u", "y", "s", "n",
    ] {
        if !(candidate == "d" && lower.starts_with("0x"))
            && body.len() > candidate.len()
            && body.as_bytes()[body.len() - candidate.len()..]
                .eq_ignore_ascii_case(candidate.as_bytes())
        {
            suffix = candidate;
            body = &body[..body.len() - candidate.len()];
            break;
        }
    }
    if body.is_empty() {
        return Some(Err(invalid(span)));
    }

    let radix = if body.len() > 2 && (body.starts_with("0x") || body.starts_with("0X")) {
        16
    } else if body.len() > 2 && (body.starts_with("0b") || body.starts_with("0B")) {
        2
    } else {
        10
    };
    let digits = match radix {
        16 | 2 => &body[2..],
        _ => body,
    };
    if digits.is_empty() {
        return Some(Err(invalid(span)));
    }
    if radix == 10
        && !digits
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '_' | '.' | 'e' | 'E' | '+' | '-'))
    {
        return None;
    }
    let real =
        digits.contains('.') || (radix == 10 && (digits.contains('e') || digits.contains('E')));
    let multiplier = multiplier.map_or(1u128, |suffix| match suffix {
        "kb" => 1024,
        "mb" => 1024 * 1024,
        "gb" => 1024 * 1024 * 1024,
        "tb" => 1024u128.pow(4),
        _ => 1024u128.pow(5),
    });

    if real {
        if radix != 10 {
            return Some(Err(invalid(span)));
        }
        let Ok(value) = digits.parse::<f64>() else {
            return Some(Err(invalid(span)));
        };
        let value = if negative { -value } else { value } * multiplier as f64;
        if suffix.eq_ignore_ascii_case("d") {
            let text = if digits.contains('e') || digits.contains('E') {
                format!("{value}")
            } else {
                let digits = digits
                    .strip_prefix('.')
                    .map_or(digits.to_owned(), |rest| format!("0.{rest}"));
                if negative {
                    format!("-{digits}")
                } else {
                    digits
                }
            };
            return Some(Ok(NumberLiteral::Decimal(text)));
        }
        if !suffix.is_empty() {
            return Some(Err(invalid(span)));
        }
        return Some(Ok(NumberLiteral::Double(value)));
    }

    let clean: String = digits.chars().filter(|ch| *ch != '_').collect();
    let Ok(magnitude) = u128::from_str_radix(&clean, radix) else {
        return Some(Err(invalid(span)));
    };
    let Some(value) = magnitude.checked_mul(multiplier) else {
        return Some(Err(invalid(span)));
    };
    let suffix = suffix.to_ascii_lowercase();

    let signed = || -> Option<i128> {
        let value = i128::try_from(value).ok()?;
        Some(if negative { -value } else { value })
    };

    let result = match suffix.as_str() {
        "l" => {
            if radix != 10 {
                let value = sign_extend(value, 64);
                return Some(Ok(NumberLiteral::Int64(if negative {
                    -value
                } else {
                    value
                })));
            }
            match signed().and_then(|v| i64::try_from(v).ok()) {
                Some(v) => NumberLiteral::Int64(v),
                None => return Some(Err(invalid(span))),
            }
        }
        "u" => match u32::try_from(value) {
            Ok(v) if !negative => NumberLiteral::UInt32(v),
            _ => return Some(Err(invalid(span))),
        },
        "ul" | "lu" => match u64::try_from(value) {
            Ok(v) if !negative => NumberLiteral::UInt64(v),
            _ => return Some(Err(invalid(span))),
        },
        "y" => match signed().and_then(|v| i8::try_from(v).ok()) {
            Some(v) => NumberLiteral::SByte(v),
            None => return Some(Err(invalid(span))),
        },
        "uy" | "yu" => match u8::try_from(value) {
            Ok(v) if !negative => NumberLiteral::Byte(v),
            _ => return Some(Err(invalid(span))),
        },
        "s" => match signed().and_then(|v| i16::try_from(v).ok()) {
            Some(v) => NumberLiteral::Int16(v),
            None => return Some(Err(invalid(span))),
        },
        "us" | "su" => match u16::try_from(value) {
            Ok(v) if !negative => NumberLiteral::UInt16(v),
            _ => return Some(Err(invalid(span))),
        },
        "n" => {
            let text = if negative {
                format!("-{value}")
            } else {
                value.to_string()
            };
            NumberLiteral::BigInteger(text)
        }
        "d" => NumberLiteral::Decimal(if negative {
            format!("-{value}")
        } else {
            value.to_string()
        }),
        "" => {
            if radix != 10 {
                let value = if negative {
                    value.wrapping_neg()
                } else {
                    value
                };
                return Some(Ok(if value <= u128::from(u32::MAX) {
                    NumberLiteral::Int32(sign_extend(value, 32) as i32)
                } else if value <= u128::from(u64::MAX) {
                    NumberLiteral::Int64(sign_extend(value, 64))
                } else {
                    NumberLiteral::Decimal(value.to_string())
                }));
            }
            let Some(signed) = signed() else {
                return Some(Err(invalid(span)));
            };
            if let Ok(v) = i32::try_from(signed) {
                NumberLiteral::Int32(v)
            } else if let Ok(v) = i64::try_from(signed) {
                NumberLiteral::Int64(v)
            } else {
                NumberLiteral::Decimal(signed.to_string())
            }
        }
        _ => return Some(Err(invalid(span))),
    };
    Some(Ok(result))
}

fn sign_extend(value: u128, bits: u32) -> i64 {
    let mask = (1u128 << bits) - 1;
    let value = value & mask;
    let sign = 1u128 << (bits - 1);
    if value & sign != 0 {
        (value | !mask) as i64
    } else {
        value as i64
    }
}

fn invalid(span: Span) -> ParseError {
    ParseError::not_implemented("invalid numeric constant", span)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(text: &str) -> NumberLiteral {
        parse_number_literal(text, Span::default())
            .unwrap()
            .unwrap()
    }

    #[test]
    fn picks_the_dotnet_type_by_value_and_suffix() {
        assert_eq!(literal("1").type_name(), "System.Int32");
        assert_eq!(literal("2147483648").type_name(), "System.Int64");
        assert_eq!(literal("9223372036854775808").type_name(), "System.Decimal");
        assert_eq!(literal("0xFFFFFFFF").to_string(), "-1");
        assert_eq!(literal("0xFFFFFFFFFFFFFFFF").to_string(), "-1");
        assert_eq!(literal("0b101").to_string(), "5");
        assert_eq!(literal("0xD83D"), NumberLiteral::Int32(0xD83D));
        assert_eq!(literal("0x1dkb"), NumberLiteral::Int32(29 * 1024));
        assert_eq!(literal("0b11D"), NumberLiteral::Decimal("3".into()));
        assert_eq!(literal("1kb").to_string(), "1024");
        assert_eq!(literal("1tb").type_name(), "System.Int64");
        assert_eq!(literal("10l").type_name(), "System.Int64");
        assert_eq!(literal("1d").type_name(), "System.Decimal");
        assert_eq!(literal("1.5").type_name(), "System.Double");
        assert_eq!(literal("1.50d").to_string(), "1.50");
        assert_eq!(literal("1e3").to_string(), "1000");
        assert_eq!(literal(".5").to_string(), "0.5");
        assert_eq!(literal("255uy").type_name(), "System.Byte");
        assert_eq!(literal("1u").type_name(), "System.UInt32");
        assert_eq!(literal("1ul").type_name(), "System.UInt64");
        assert_eq!(literal("1s").type_name(), "System.Int16");
        assert_eq!(literal("1us").type_name(), "System.UInt16");
        assert_eq!(literal("1n").type_name(), "System.Numerics.BigInteger");
    }

    #[test]
    fn rejects_known_numeric_forms_that_do_not_fit() {
        assert!(
            parse_number_literal("255y", Span::default())
                .unwrap()
                .is_err()
        );
        assert!(
            parse_number_literal("1.5u", Span::default())
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn leaves_command_names_alone() {
        assert!(parse_number_literal("7z", Span::default()).is_none());
    }
}
