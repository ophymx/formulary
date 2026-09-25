//! SVG emission — the debugging window and golden-test format.
//!
//! Glyphs are outlined to `<path>` data via `ttf-parser`, so the output is
//! self-contained (no font references) and text-diffable.

use core::fmt::Write as _;

use crate::ast::Color;
use crate::font::MathFont;
use crate::layout::{Item, Layout};

/// Padding around the formula in the SVG viewport, in layout units.
const MARGIN: f32 = 2.0;

/// Render a display list as a standalone SVG document.
///
/// The layout's baseline is placed at `y = MARGIN + ascent` in SVG space.
pub fn to_svg(layout: &Layout, font: &MathFont) -> String {
    let width = layout.width + 2.0 * MARGIN;
    let height = layout.ascent + layout.descent + 2.0 * MARGIN;
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" width="{w}" height="{h}">"#,
        fmt(width),
        fmt(height),
        w = fmt(width),
        h = fmt(height),
    );
    let _ = writeln!(
        svg,
        r#"<g transform="translate({} {})" fill="black">"#,
        fmt(MARGIN),
        fmt(MARGIN + layout.ascent)
    );
    // Path data is per-glyph (position and scale live in the transform), so
    // outline each distinct glyph once per document.
    let mut outlined: std::collections::HashMap<crate::GlyphId, Option<String>> =
        std::collections::HashMap::new();
    for item in &layout.items {
        match *item {
            Item::Glyph {
                id,
                x,
                y,
                size,
                advance,
                color,
                mirrored,
            } => {
                let d = outlined.entry(id).or_insert_with(|| {
                    let mut builder = PathBuilder::default();
                    (font.outline(id, &mut builder) && !builder.d.is_empty()).then_some(builder.d)
                });
                let Some(d) = d else {
                    continue; // blank glyph (e.g. space)
                };
                // Outline is in font units, y-up; scale to `size` and flip y.
                // Mirrored glyphs flip about their advance box.
                let s = size / font.units_per_em();
                let (tx, sx) = if mirrored {
                    (x + advance, format!("-{}", fmt(s)))
                } else {
                    (x, fmt(s))
                };
                let _ = writeln!(
                    svg,
                    r#"<path transform="translate({} {}) scale({sx} -{sy})"{f} d="{d}"/>"#,
                    fmt(tx),
                    fmt(y),
                    sx = sx,
                    sy = fmt(s),
                    f = fill(color),
                    d = d,
                );
            }
            Item::Rule { x, y, w, h, color } => {
                let _ = writeln!(
                    svg,
                    r#"<rect x="{}" y="{}" width="{}" height="{}"{}/>"#,
                    fmt(x),
                    fmt(y),
                    fmt(w),
                    fmt(h),
                    fill(color)
                );
            }
            Item::Background { x, y, w, h, color } => {
                let _ = writeln!(
                    svg,
                    r#"<rect x="{}" y="{}" width="{}" height="{}"{}/>"#,
                    fmt(x),
                    fmt(y),
                    fmt(w),
                    fmt(h),
                    fill(Some(color))
                );
            }
        }
    }
    svg.push_str("</g>\n</svg>\n");
    svg
}

/// A `fill` attribute for a colored item; empty for the inherited default,
/// so colorless output is unchanged.
fn fill(color: Option<Color>) -> String {
    match color {
        None => String::new(),
        Some(c) if c.a == 255 => format!(r##" fill="#{:02x}{:02x}{:02x}""##, c.r, c.g, c.b),
        Some(c) => format!(
            r##" fill="#{:02x}{:02x}{:02x}" fill-opacity="{}""##,
            c.r,
            c.g,
            c.b,
            fmt(f32::from(c.a) / 255.0)
        ),
    }
}

/// Compact, stable float formatting for diffable goldens: three decimal
/// places, trailing zeros trimmed.
///
/// Fixed-point via integer math: `v * 1000` is exact in f64 (24 significand
/// bits × 1000 stays under 2^53), so `round_ties_even` reproduces
/// `format!("{v:.3}")`'s correct rounding at a fraction of its cost.
fn fmt(v: f32) -> String {
    let mut s = String::new();
    fmt_to(&mut s, v);
    s
}

/// The straightforward formatting this module's fast path must match:
/// `{v:.3}` with trailing zeros (and a bare sign) trimmed. Kept as the
/// huge/non-finite fallback and as the oracle for the equivalence test.
fn fmt_slow(v: f32) -> String {
    let mut s = format!("{v:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

fn fmt_to(out: &mut String, v: f32) {
    let scaled = f64::from(v) * 1000.0;
    if scaled.is_nan() || scaled.abs() >= 9.0e15 {
        out.push_str(&fmt_slow(v));
        return;
    }
    let n = scaled.round_ties_even() as i64;
    if n == 0 {
        out.push('0');
        return;
    }
    if n < 0 {
        out.push('-');
    }
    let n = n.unsigned_abs();
    let _ = write!(out, "{}", n / 1000);
    let frac = n % 1000;
    if frac != 0 {
        if frac.is_multiple_of(100) {
            let _ = write!(out, ".{}", frac / 100);
        } else if frac.is_multiple_of(10) {
            let _ = write!(out, ".{:02}", frac / 10);
        } else {
            let _ = write!(out, ".{frac:03}");
        }
    }
}

#[derive(Default)]
struct PathBuilder {
    d: String,
}

impl skrifa::outline::OutlinePen for PathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.d.push('M');
        fmt_to(&mut self.d, x);
        self.d.push(' ');
        fmt_to(&mut self.d, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.d.push('L');
        fmt_to(&mut self.d, x);
        self.d.push(' ');
        fmt_to(&mut self.d, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.d.push('Q');
        for (i, v) in [x1, y1, x, y].into_iter().enumerate() {
            if i > 0 {
                self.d.push(' ');
            }
            fmt_to(&mut self.d, v);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.d.push('C');
        for (i, v) in [x1, y1, x2, y2, x, y].into_iter().enumerate() {
            if i > 0 {
                self.d.push(' ');
            }
            fmt_to(&mut self.d, v);
        }
    }
    fn close(&mut self) {
        self.d.push('Z');
    }
}

#[cfg(test)]
mod tests {
    /// The fast fixed-point path must reproduce the `format!("{v:.3}")`
    /// slow path byte-for-byte.
    #[test]
    fn fmt_matches_std_formatting() {
        let mut cases: Vec<f32> = vec![0.0, -0.0, 0.0625, -0.0625, 1.5, -1.5, 0.0005, -0.0001];
        // Ties (multiples of 1/16) and a pseudo-random sweep.
        for i in 0..20000u32 {
            cases.push(i as f32 / 16.0);
            cases.push(-(i as f32) / 16.0);
            let x = f32::from_bits(0x3800_0000u32.wrapping_add(i.wrapping_mul(2_654_435_761)));
            if x.is_finite() {
                cases.push(x % 1.0e6);
            }
        }
        for v in cases {
            assert_eq!(
                super::fmt(v),
                super::fmt_slow(v),
                "for {v} ({:x})",
                v.to_bits()
            );
        }
    }
}
