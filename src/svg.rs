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
    for item in &layout.items {
        match *item {
            Item::Glyph {
                id,
                x,
                y,
                size,
                color,
                mirrored,
            } => {
                let mut builder = PathBuilder::default();
                if font.face().outline_glyph(id, &mut builder).is_none() {
                    continue; // blank glyph (e.g. space)
                }
                // Outline is in font units, y-up; scale to `size` and flip y.
                // Mirrored glyphs flip about their advance box.
                let s = size / font.units_per_em();
                let (tx, sx) = if mirrored {
                    let advance = font
                        .face()
                        .glyph_hor_advance(id)
                        .unwrap_or(0);
                    (x + f32::from(advance) * s, format!("-{}", fmt(s)))
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
                    d = builder.d,
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
fn fmt(v: f32) -> String {
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

#[derive(Default)]
struct PathBuilder {
    d: String,
}

impl ttf_parser::OutlineBuilder for PathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "M{} {}", fmt(x), fmt(y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.d, "L{} {}", fmt(x), fmt(y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let _ = write!(self.d, "Q{} {} {} {}", fmt(x1), fmt(y1), fmt(x), fmt(y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let _ = write!(
            self.d,
            "C{} {} {} {} {} {}",
            fmt(x1),
            fmt(y1),
            fmt(x2),
            fmt(y2),
            fmt(x),
            fmt(y)
        );
    }
    fn close(&mut self) {
        self.d.push('Z');
    }
}
