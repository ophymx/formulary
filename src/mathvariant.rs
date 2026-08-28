//! The automatic italic mapping MathML Core applies to single-character `<mi>`.
//!
//! Single-char identifiers default to `mathvariant="italic"`, realized by
//! substituting the Mathematical Alphanumeric Symbols code point (MathML Core
//! §"New text transform values"). Multi-char identifiers stay upright.

/// Map `c` to its mathematical-italic counterpart, or return it unchanged if
/// no mapping exists (digits, punctuation, already-styled characters).
pub fn to_math_italic(c: char) -> char {
    let mapped = match c {
        // Unicode leaves a hole in the italic block where ℎ predates it.
        'h' => 0x210E,
        'A'..='Z' => 0x1D434 + (c as u32 - 'A' as u32),
        'a'..='z' => 0x1D44E + (c as u32 - 'a' as u32),
        // Greek capitals; the reserved slot U+03A2 lines up with the math
        // block's THETA SYMBOL entry, so a straight offset is correct.
        '\u{0391}'..='\u{03A9}' => 0x1D6E2 + (c as u32 - 0x0391),
        '\u{03B1}'..='\u{03C9}' => 0x1D6FC + (c as u32 - 0x03B1),
        '\u{0131}' => 0x1D6A4, // dotless i
        '\u{0237}' => 0x1D6A5, // dotless j
        '\u{2202}' => 0x1D715, // partial differential
        '\u{2207}' => 0x1D6FB, // nabla
        _ => return c,
    };
    char::from_u32(mapped).unwrap_or(c)
}
