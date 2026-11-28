//! Wrapped fields in PowerShell's complex formatting views.
use crate::command::{cell_width, cells};

/// Write each logical line with the view's indentation. ComplexWriter enables
/// word wrapping for these six language families, and uses display cells otherwise.
pub fn wraps_words(language: &str) -> bool {
    matches!(
        language.split(['-', '_']).next().unwrap_or(""),
        "en" | "fr" | "de" | "it" | "pt" | "es"
    )
}

pub fn write_field(out: &mut String, indent: usize, width: usize, words: bool, text: &str) {
    let padding = " ".repeat(indent);
    if indent >= width {
        out.push_str(&padding);
        out.push('\n');
        return;
    }
    let available = width - indent;
    for line in text.split('\n') {
        out.push_str(&padding);
        let mut used = 0;
        if words {
            let mut word = String::new();
            for ch in line.chars() {
                match ch {
                    ' ' | '\t' => word.push(ch),
                    '\u{ad}' => {
                        if used + cells(&word) + 1 == available {
                            word.push('-');
                        }
                    }
                    '\u{2011}' | '\u{a0}' => {}
                    _ => {
                        word.push(ch);
                        continue;
                    }
                }
                append_piece(out, &padding, available, &mut used, &word, true);
                word.clear();
            }
            append_piece(out, &padding, available, &mut used, &word, true);
        } else {
            append_piece(out, &padding, available, &mut used, line, false);
        }
        out.push('\n');
    }
}

fn append_piece(
    out: &mut String,
    padding: &str,
    width: usize,
    used: &mut usize,
    text: &str,
    word: bool,
) {
    if word {
        let len = cells(text);
        if len <= width {
            if *used + len > width {
                out.push('\n');
                out.push_str(padding);
                *used = 0;
            }
            out.push_str(text);
            *used += len;
            return;
        }
    }
    for ch in text.chars() {
        let len = cell_width(ch);
        let (ch, len) = if len > width { ('?', 1) } else { (ch, len) };
        if *used + len > width {
            out.push('\n');
            out.push_str(padding);
            *used = 0;
        }
        out.push(ch);
        *used += len;
    }
}
