//! Mathematical Alphanumeric Symbols mappings.
//!
//! Two users: the automatic italic MathML Core applies to single-character
//! `<mi>`, and the legacy `mathvariant` attribute values (still ubiquitous in
//! real markup for bold vectors, ℝ-style double-struck sets, …). Characters
//! are mapped at parse time, so layout only ever sees final code points.

/// A `mathvariant` attribute value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathVariant {
    Normal,
    Bold,
    Italic,
    BoldItalic,
    DoubleStruck,
    Script,
    BoldScript,
    Fraktur,
    BoldFraktur,
    SansSerif,
    BoldSansSerif,
    SansSerifItalic,
    SansSerifBoldItalic,
    Monospace,
}

impl MathVariant {
    pub fn from_attr(s: &str) -> Option<Self> {
        Some(match s {
            "normal" => Self::Normal,
            "bold" => Self::Bold,
            "italic" => Self::Italic,
            "bold-italic" => Self::BoldItalic,
            "double-struck" => Self::DoubleStruck,
            "script" => Self::Script,
            "bold-script" => Self::BoldScript,
            "fraktur" => Self::Fraktur,
            "bold-fraktur" => Self::BoldFraktur,
            "sans-serif" => Self::SansSerif,
            "bold-sans-serif" => Self::BoldSansSerif,
            "sans-serif-italic" => Self::SansSerifItalic,
            "sans-serif-bold-italic" => Self::SansSerifBoldItalic,
            "monospace" => Self::Monospace,
            _ => return None,
        })
    }
}

/// Block starts (`None` = the variant has no glyphs for that class, keep the
/// character) and the Letterlike Symbols holes that predate the styled
/// blocks.
struct Plane {
    latin_cap: Option<u32>,
    latin_low: Option<u32>,
    digit: Option<u32>,
    greek_cap: Option<u32>,
    greek_low: Option<u32>,
    exceptions: &'static [(char, u32)],
}

fn plane(v: MathVariant) -> Plane {
    use MathVariant::*;
    let p = |latin_cap, latin_low, digit, greek_cap, greek_low, exceptions| Plane {
        latin_cap,
        latin_low,
        digit,
        greek_cap,
        greek_low,
        exceptions,
    };
    match v {
        Normal => p(None, None, None, None, None, &[]),
        Bold => p(
            Some(0x1D400),
            Some(0x1D41A),
            Some(0x1D7CE),
            Some(0x1D6A8),
            Some(0x1D6C2),
            &[],
        ),
        Italic => p(
            Some(0x1D434),
            Some(0x1D44E),
            None,
            Some(0x1D6E2),
            Some(0x1D6FC),
            &[
                ('h', 0x210E),
                ('\u{0131}', 0x1D6A4),
                ('\u{0237}', 0x1D6A5),
                ('\u{2202}', 0x1D715),
                ('\u{2207}', 0x1D6FB),
            ],
        ),
        BoldItalic => p(
            Some(0x1D468),
            Some(0x1D482),
            None,
            Some(0x1D71C),
            Some(0x1D736),
            &[],
        ),
        DoubleStruck => p(
            Some(0x1D538),
            Some(0x1D552),
            Some(0x1D7D8),
            None,
            None,
            &[
                ('C', 0x2102),
                ('H', 0x210D),
                ('N', 0x2115),
                ('P', 0x2119),
                ('Q', 0x211A),
                ('R', 0x211D),
                ('Z', 0x2124),
            ],
        ),
        Script => p(
            Some(0x1D49C),
            Some(0x1D4B6),
            None,
            None,
            None,
            &[
                ('B', 0x212C),
                ('E', 0x2130),
                ('F', 0x2131),
                ('H', 0x210B),
                ('I', 0x2110),
                ('L', 0x2112),
                ('M', 0x2133),
                ('R', 0x211B),
                ('e', 0x212F),
                ('g', 0x210A),
                ('o', 0x2134),
            ],
        ),
        BoldScript => p(Some(0x1D4D0), Some(0x1D4EA), None, None, None, &[]),
        Fraktur => p(
            Some(0x1D504),
            Some(0x1D51E),
            None,
            None,
            None,
            &[
                ('C', 0x212D),
                ('H', 0x210C),
                ('I', 0x2111),
                ('R', 0x211C),
                ('Z', 0x2128),
            ],
        ),
        BoldFraktur => p(Some(0x1D56C), Some(0x1D586), None, None, None, &[]),
        SansSerif => p(Some(0x1D5A0), Some(0x1D5BA), Some(0x1D7E2), None, None, &[]),
        BoldSansSerif => p(
            Some(0x1D5D4),
            Some(0x1D5EE),
            Some(0x1D7EC),
            Some(0x1D756),
            Some(0x1D770),
            &[],
        ),
        SansSerifItalic => p(Some(0x1D608), Some(0x1D622), None, None, None, &[]),
        SansSerifBoldItalic => p(
            Some(0x1D63C),
            Some(0x1D656),
            None,
            Some(0x1D790),
            Some(0x1D7AA),
            &[],
        ),
        Monospace => p(Some(0x1D670), Some(0x1D68A), Some(0x1D7F6), None, None, &[]),
    }
}

/// Map `c` into the variant's styled block, or return it unchanged if the
/// variant has no counterpart for it.
pub fn apply_variant(c: char, v: MathVariant) -> char {
    let plane = plane(v);
    if let Some(&(_, to)) = plane.exceptions.iter().find(|(from, _)| *from == c) {
        return char::from_u32(to).unwrap_or(c);
    }
    let mapped = match c {
        'A'..='Z' => plane.latin_cap.map(|b| b + (c as u32 - 'A' as u32)),
        'a'..='z' => plane.latin_low.map(|b| b + (c as u32 - 'a' as u32)),
        '0'..='9' => plane.digit.map(|b| b + (c as u32 - '0' as u32)),
        // Greek capitals; the reserved slot U+03A2 lines up with the math
        // blocks' THETA SYMBOL entry, so a straight offset is correct.
        '\u{0391}'..='\u{03A9}' => plane.greek_cap.map(|b| b + (c as u32 - 0x0391)),
        '\u{03B1}'..='\u{03C9}' => plane.greek_low.map(|b| b + (c as u32 - 0x03B1)),
        _ => None,
    };
    mapped
        .and_then(char::from_u32)
        .unwrap_or(c)
}

/// Map `c` to its mathematical-italic counterpart (the single-char `<mi>`
/// default), or return it unchanged if no mapping exists.
pub fn to_math_italic(c: char) -> char {
    apply_variant(c, MathVariant::Italic)
}
