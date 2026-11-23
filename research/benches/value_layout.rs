//! #35: the size and clone cost of `rpsh_runtime::Value`.
//!
//! DESIGN.md §3.3 settles a 16-byte value: a tag and an 8-byte payload. Every
//! heap variant is one pointer, and the string handle is thin because ASCII
//! and UTF-16 live behind it. This bench keeps that under measurement; the
//! compile-time assertion in `rpsh-runtime` fails the build if the size grows.

use std::hint::black_box;

use rpsh_research::{Report, iterations, measure};
use rpsh_runtime::{PsString, Value};

fn main() {
    let mut report = Report::new(
        "value_layout",
        "rpsh_runtime::Value size and clone cost (#35)",
    );
    report.header();
    report.fact("size_of Value", std::mem::size_of::<Value>());
    report.fact("size_of PsString", std::mem::size_of::<PsString>());

    let int = Value::Int32(42);
    report.add(measure(
        "clone Value",
        "Int32",
        iterations(1_000_000),
        || black_box(black_box(&int).clone()),
    ));

    let text = Value::String(PsString::from(
        "2026-11-21T10:15:42Z INFO request handled path=/api/v1/items",
    ));
    report.add(measure(
        "clone Value",
        "String (ASCII)",
        iterations(1_000_000),
        || black_box(black_box(&text).clone()),
    ));

    report.save();
}
