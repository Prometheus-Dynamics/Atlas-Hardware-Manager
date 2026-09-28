use std::io::Write;

use serde_json::Value;

/// Writes one line to stdout. A closed pipe is not an error worth reporting.
pub(crate) fn line(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

pub(crate) fn json(value: &Value) {
    line(&value.to_string());
}

pub(crate) fn error(text: &str) {
    let _ = writeln!(std::io::stderr(), "atlas: {text}");
}

/// Renders rows as left-aligned columns separated by two spaces.
pub(crate) fn table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let render = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        padded.join("  ").trim_end().to_string()
    };
    line(&render(headers.to_vec()));
    for row in rows {
        line(&render(row.iter().map(String::as_str).collect()));
    }
}
