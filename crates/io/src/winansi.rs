// Ported from PdfCraft (https://github.com/storytold/pdfcraft), commit
// c944f106dd63eb9a33c7348eed1f03548cbd6468: `win_ansi_byte`, `win_ansi`, `literal` and the
// 0x80–0x9F part of the WinAnsi table from `crates/fonts/src/lib.rs` and `encodings.rs`;
// `helvetica_width` from `crates/fonts/src/std14.rs`.
//
// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option; see
// LICENSE-MIT, LICENSE-APACHE and NOTICE in this repository.

//! WinAnsiEncoding (ISO 32000-2 Annex D), PDF literal strings and the standard Helvetica's
//! advance widths: what the text layer needs to set a word in `/Helvetica`.

use crate::helvetica::HELVETICA;

/// Unicode for codes 0x80–0x9F of WinAnsiEncoding (0 = undefined).
static WIN_ANSI_HIGH: [u32; 32] = [
    8364, 0, 8218, 402, 8222, 8230, 8224, 8225, 710, 8240, 352, 8249, 338, 0, 381, 0, 0, 8216, 8217, 8220, 8221, 8226, 8211, 8212, 732, 8482, 353,
    8250, 339, 0, 382, 376,
];

/// The WinAnsiEncoding code for `c`, if the encoding has one.
pub(crate) fn win_ansi_byte(c: char) -> Option<u8> {
    match c {
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => u8::try_from(u32::from(c)).ok(),
        _ => {
            let i = WIN_ANSI_HIGH.iter().position(|&u| u != 0 && u == u32::from(c))?;
            u8::try_from(0x80 + i).ok()
        }
    }
}

/// The code of `c` in a word set in Helvetica: a tab is a space.
pub(crate) fn win_ansi_char(c: char) -> Option<u8> {
    if c == '\t' { Some(b' ') } else { win_ansi_byte(c) }
}

/// Whether every character of `s` has a WinAnsi code.
pub(crate) fn fits_win_ansi(s: &str) -> bool {
    s.chars().all(|c| win_ansi_char(c).is_some())
}

/// Text in WinAnsiEncoding; tabs become spaces and unmappable characters become `?`.
pub(crate) fn win_ansi(s: &str) -> Vec<u8> {
    s.chars().map(|c| win_ansi_char(c).unwrap_or(b'?')).collect()
}

/// Bytes as a PDF literal string, `(` … `)`, with delimiters escaped.
pub(crate) fn literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'(');
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\n' => out.extend_from_slice(b"\\n"),
            _ => out.push(b),
        }
    }
    out.push(b')');
    out
}

/// The advance of `s` in Helvetica at `size`, from the standard metrics, in the same units as `size`.
pub(crate) fn helvetica_width(s: &str, size: f64) -> f64 {
    win_ansi(s).iter().map(|&c| f64::from(HELVETICA.get(usize::from(c)).copied().unwrap_or(0))).sum::<f64>() * size / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{fits_win_ansi, helvetica_width, literal, win_ansi, win_ansi_byte};

    #[test]
    fn win_ansi_maps_ascii_latin_1_and_the_high_block() {
        assert_eq!(win_ansi("Hello, World"), b"Hello, World");
        assert_eq!(win_ansi_byte('é'), Some(0xe9));
        assert_eq!(win_ansi_byte('€'), Some(0x80));
        assert_eq!(win_ansi_byte('“'), Some(0x93));
        assert_eq!(win_ansi_byte('—'), Some(0x97));
        assert_eq!(win_ansi_byte('ž'), Some(0x9e));
        assert_eq!(win_ansi_byte('Ж'), None);
        assert_eq!(win_ansi_byte('\u{1f600}'), None);
        assert_eq!(win_ansi("a\tb"), b"a b");
        assert_eq!(win_ansi("zЖ"), b"z?");
        assert!(fits_win_ansi("naïve café — “quoted”") && !fits_win_ansi("Привет"));
    }

    #[test]
    fn literal_strings_escape_their_delimiters() {
        assert_eq!(literal(b"a(b)c\\d"), b"(a\\(b\\)c\\\\d)");
        assert_eq!(literal(b"line\r\nbreak"), b"(line\\r\\nbreak)");
        assert_eq!(literal(b""), b"()");
    }

    #[test]
    fn helvetica_widths_are_the_published_ones() {
        // "AW" is 667 + 944 thousandths of an em (the Core 14 metrics).
        assert!((helvetica_width("AW", 1000.0) - 1611.0).abs() < 1e-9);
        assert!((helvetica_width(" ", 1.0) - 0.278).abs() < 1e-12);
        assert!((helvetica_width("i", 1.0) - 0.222).abs() < 1e-12);
        assert_eq!(helvetica_width("", 1.0), 0.0);
        assert!(helvetica_width(&"W".repeat(10_000), 1.0).is_finite());
    }
}
