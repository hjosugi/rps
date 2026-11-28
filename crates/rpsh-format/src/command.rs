//! The built-in CommandInfo table from PowerShellCore.format.ps1xml.

const HEADERS: [&str; 4] = ["CommandType", "Name", "Version", "Source"];

/// A command row or a value the host displays between rows of an open table.
#[derive(Debug)]
pub enum CommandTableItem<const N: usize = 4> {
    Command([String; N]),
    Text(String),
}

/// Formats the CommandInfo view in plain text. The first three columns have
/// fixed widths and Source uses the space left in the output device.
pub fn command_table(
    items: &[CommandTableItem],
    width: usize,
    auto_size: bool,
    hide_headers: bool,
) -> String {
    fixed_table(
        items,
        width,
        HEADERS,
        [15, 50, 10, width],
        auto_size,
        hide_headers,
    )
}

/// HelpInfoShort's built-in four-column view. Synopsis uses the remaining columns.
pub fn help_table(items: &[CommandTableItem], width: usize) -> String {
    fixed_table(
        items,
        width,
        ["Name", "Category", "Module", "Synopsis"],
        [33, 9, 25, width],
        false,
        false,
    )
}

/// The built-in PSVariable view gives its Name column 30 character cells.
pub fn variable_table(
    items: &[CommandTableItem<2>],
    width: usize,
    auto_size: bool,
    hide_headers: bool,
) -> String {
    fixed_table(
        items,
        width,
        ["Name", "Value"],
        [30, width],
        auto_size,
        hide_headers,
    )
}

fn fixed_table<const N: usize>(
    items: &[CommandTableItem<N>],
    width: usize,
    headers: [&str; N],
    mut widths: [usize; N],
    auto_size: bool,
    hide_headers: bool,
) -> String {
    if auto_size {
        widths = headers.map(str::len);
        for item in items {
            if let CommandTableItem::Command(fields) = item {
                for (width, field) in widths.iter_mut().zip(fields) {
                    *width = (*width).max(cells(field));
                }
            }
        }
    }

    let mut remaining = width;
    for width in &mut widths {
        *width = remaining.min(*width);
        remaining = remaining.saturating_sub(*width).saturating_sub(1);
    }

    let mut out = String::from("\n");
    if width >= 5 && !hide_headers {
        // Header labels wrap. Their last visible column has no right padding.
        let lines = headers
            .iter()
            .zip(widths)
            .filter(|(_, width)| *width != 0)
            .map(|(label, width)| label.len().div_ceil(width))
            .max()
            .unwrap_or(0);
        for line in 0..lines {
            let mut header = String::new();
            for (index, (&label, &width)) in headers.iter().zip(&widths).enumerate() {
                if width == 0 {
                    break;
                }
                let start = label.len().min(line.saturating_mul(width));
                let end = label.len().min(start + width);
                let field = &label[start..end];
                if index > 0 {
                    header.push(' ');
                }
                header.push_str(field);
                if index + 1 < widths.len() && widths[index + 1] > 0 {
                    header.extend(std::iter::repeat_n(' ', width - field.len()));
                }
            }
            out.push_str(header.trim_end());
            out.push('\n');
        }
        let rules = std::array::from_fn(|i| "-".repeat(widths[i].min(headers[i].len())));
        write_row(&mut out, &rules, &widths);
    }
    for item in items {
        match item {
            CommandTableItem::Command(fields) if width >= 5 => write_row(&mut out, fields, &widths),
            CommandTableItem::Command(_) => {}
            CommandTableItem::Text(text) => {
                out.push_str(text);
                out.push('\n');
            }
        }
    }
    out.push('\n');
    out
}

fn write_row<const N: usize>(out: &mut String, fields: &[String; N], widths: &[usize; N]) {
    for (index, (field, &width)) in fields.iter().zip(widths).enumerate() {
        if width == 0 {
            break;
        }
        if index > 0 {
            out.push(' ');
        }
        let field = field.split(['\r', '\n']).next().unwrap_or_default();
        let mut used = cells(field);
        if used <= width {
            out.push_str(field);
        } else {
            used = 0;
            let target = if width > 1 { width - 1 } else { width };
            for ch in field.chars() {
                let advance = cell_width(ch);
                if used + advance > target {
                    break;
                }
                out.push(ch);
                used += advance;
            }
            if width > 1 {
                out.push('…');
                used += 1;
            }
        }
        // TableWriter pads even the last visible field when columns to its
        // right were hidden. Only the original last field is never padded.
        if index + 1 < widths.len() {
            out.extend(std::iter::repeat_n(' ', width - used));
        }
    }
    out.push('\n');
}

pub(crate) fn cells(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

// ConsoleControl.LengthInBufferCells counts UTF-16 units and doubles these
// BMP ranges for a wide terminal cell. A supplementary scalar uses two units.
pub(crate) fn cell_width(ch: char) -> usize {
    match ch as u32 {
        0x1100..=0x115f
        | 0x2329..=0x232a
        | 0x2e80..=0x303e
        | 0x3040..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe10..=0xfe19
        | 0xfe30..=0xfe6f
        | 0xff00..=0xff60
        | 0xffe0..=0xffe6
        | 0x10000.. => 2,
        _ => 1,
    }
}
