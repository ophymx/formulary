//! Layout: element tree + MATH metrics → display list.
//!
//! ## Coordinate system
//!
//! All output values share one unit: the caller's `font_size` (whatever unit
//! that is in — px, pt, …). `x` grows rightward from the layout's left edge;
//! `y` grows **downward** with `y = 0` at the baseline. `ascent` and `descent`
//! are positive distances above/below the baseline, so the layout's total
//! height is `ascent + descent` and its top edge is at `y = -ascent`.
//!
//! Layout is cheap and resolution-independent: re-run it when the target font
//! size changes rather than scaling a previous result.

use crate::ast::{DisplayMode, MathRoot, Node};
use crate::font::{GlyphId, MathFont};
use crate::mathvariant::to_math_italic;

/// Caller-supplied layout parameters.
#[derive(Debug, Clone)]
pub struct LayoutOptions {
    /// Target font size. Every coordinate and length in the resulting
    /// [`Layout`] is expressed in this unit.
    pub font_size: f32,
}

/// A laid-out formula: extents plus a flat display list.
///
/// `ascent`/`descent` are the inline-integration contract: place the layout so
/// its baseline sits on the surrounding text baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub items: Vec<Item>,
}

/// One drawable in the display list.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Item {
    /// A glyph from the math font, positioned at its baseline origin `(x, y)`,
    /// to be rasterized at font size `size` (same unit as everything else;
    /// differs from the top-level font size inside scripts).
    Glyph {
        id: GlyphId,
        x: f32,
        y: f32,
        size: f32,
    },
    /// A filled rectangle (fraction bars, radical rules). `(x, y)` is the top-left
    /// corner; `h` extends downward.
    Rule { x: f32, y: f32, w: f32, h: f32 },
}

impl Item {
    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Item::Glyph { x, y, .. } | Item::Rule { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
        }
    }
}

/// Lay out a parsed formula against `font` at `options.font_size`.
pub fn layout(root: &MathRoot, font: &MathFont, options: &LayoutOptions) -> Layout {
    let ctx = Ctx::new(
        font,
        options.font_size,
        // Per MathML Core, display="block" starts in displaystyle; inline
        // math starts in text style.
        root.display == DisplayMode::Block,
    );
    let b = layout_row(&ctx, &root.children);
    Layout {
        width: b.width,
        ascent: b.ascent,
        descent: b.descent,
        items: b.items,
    }
}

/// Immutable layout state threaded through the tree (MathML Core's
/// math-style / math-depth / cramped trio, plus derived scale factors).
struct Ctx<'a, 'f> {
    font: &'a MathFont<'f>,
    /// The caller's font size; script sizes derive from it.
    base_size: f32,
    /// MathML Core math-depth: 0 at top level, +1 inside scripts and
    /// text-style fraction parts.
    script_level: u8,
    /// Displaystyle (`math-style: normal`): big operators, wider fraction
    /// gaps. Off inside fraction parts and scripts.
    display_style: bool,
    /// Cramped styles (under bars, in subscripts/denominators) raise
    /// superscripts less. Unused until scripts land, threaded now.
    #[allow(dead_code)]
    cramped: bool,
    /// Font size at the current script level, stamped on emitted glyphs.
    size: f32,
    /// Font design units → output units at the current script level.
    scale: f32,
}

impl<'a, 'f> Ctx<'a, 'f> {
    fn new(font: &'a MathFont<'f>, base_size: f32, display_style: bool) -> Self {
        Self::derive(font, base_size, 0, display_style, false)
    }

    fn derive(
        font: &'a MathFont<'f>,
        base_size: f32,
        script_level: u8,
        display_style: bool,
        cramped: bool,
    ) -> Self {
        let percent = |v: i16, fallback: f32| {
            if v > 0 {
                v as f32 / 100.0
            } else {
                fallback
            }
        };
        let consts = font.constants();
        let factor = match script_level {
            0 => 1.0,
            1 => percent(consts.script_percent_scale_down(), 0.71),
            _ => percent(consts.script_script_percent_scale_down(), 0.5041),
        };
        let size = base_size * factor;
        Ctx {
            font,
            base_size,
            script_level,
            display_style,
            cramped,
            size,
            scale: size / font.units_per_em(),
        }
    }

    /// Child context for a fraction part. Displaystyle switches off; the
    /// script level increments only when already in text style.
    fn frac_child(&self, cramped: bool) -> Self {
        let level = if self.display_style {
            self.script_level
        } else {
            self.script_level.saturating_add(1)
        };
        Ctx::derive(self.font, self.base_size, level, false, self.cramped || cramped)
    }

    /// A MATH constant, converted from design units to output units at this
    /// context's scale.
    fn constant(&self, v: ttf_parser::math::MathValue) -> f32 {
        v.value as f32 * self.scale
    }
}

/// Intermediate box; same shape as `Layout` but items are in box-local
/// coordinates (baseline at y = 0, left edge at x = 0).
struct MathBox {
    width: f32,
    ascent: f32,
    descent: f32,
    items: Vec<Item>,
}

impl MathBox {
    fn empty() -> Self {
        MathBox {
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
            items: Vec::new(),
        }
    }
}

fn layout_node(ctx: &Ctx, node: &Node) -> MathBox {
    match node {
        Node::Identifier(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                // Single-char <mi> defaults to math italic.
                (Some(c), None) => layout_text_run(ctx, &to_math_italic(c).to_string()),
                _ => layout_text_run(ctx, text),
            }
        }
        Node::Number(text) | Node::Operator(text) | Node::Text(text) => {
            layout_text_run(ctx, text)
        }
        Node::Row(children) => layout_row(ctx, children),
        Node::Frac { num, den } => layout_frac(ctx, num, den),
    }
}

/// `<mfrac>` per MathML Core §3.3.2 / the OpenType MATH fraction constants:
/// numerator shifted up and denominator down by at least the font's preferred
/// shifts, pushed further apart if the min gaps to the rule demand it; the
/// rule is centered on the math axis.
fn layout_frac(ctx: &Ctx, num: &Node, den: &Node) -> MathBox {
    let num_box = layout_node(&ctx.frac_child(false), num);
    let den_box = layout_node(&ctx.frac_child(true), den);

    let c = ctx.font.constants();
    let axis = ctx.constant(c.axis_height());
    let thickness = ctx.constant(c.fraction_rule_thickness());
    let (shift_up, shift_down, gap_above, gap_below) = if ctx.display_style {
        (
            ctx.constant(c.fraction_numerator_display_style_shift_up()),
            ctx.constant(c.fraction_denominator_display_style_shift_down()),
            ctx.constant(c.fraction_num_display_style_gap_min()),
            ctx.constant(c.fraction_denom_display_style_gap_min()),
        )
    } else {
        (
            ctx.constant(c.fraction_numerator_shift_up()),
            ctx.constant(c.fraction_denominator_shift_down()),
            ctx.constant(c.fraction_numerator_gap_min()),
            ctx.constant(c.fraction_denominator_gap_min()),
        )
    };

    // Enforce the minimum clearance between each part's ink and the rule.
    let num_shift = shift_up.max(axis + thickness / 2.0 + gap_above + num_box.descent);
    let den_shift = shift_down.max(gap_below + thickness / 2.0 - axis + den_box.ascent);

    let width = num_box.width.max(den_box.width);
    let mut out = MathBox {
        width,
        ascent: num_shift + num_box.ascent,
        descent: den_shift + den_box.descent,
        items: Vec::with_capacity(num_box.items.len() + den_box.items.len() + 1),
    };
    for (bx, dy) in [(num_box, -num_shift), (den_box, den_shift)] {
        let dx = (width - bx.width) / 2.0;
        for mut item in bx.items {
            item.translate(dx, dy);
            out.items.push(item);
        }
    }
    out.items.push(Item::Rule {
        x: 0.0,
        y: -(axis + thickness / 2.0),
        w: width,
        h: thickness,
    });
    out
}

/// Horizontal concatenation on a shared baseline.
fn layout_row(ctx: &Ctx, children: &[Node]) -> MathBox {
    let mut out = MathBox::empty();
    for child in children {
        let mut b = layout_node(ctx, child);
        for item in &mut b.items {
            item.translate(out.width, 0.0);
        }
        out.items.append(&mut b.items);
        out.width += b.width;
        out.ascent = out.ascent.max(b.ascent);
        out.descent = out.descent.max(b.descent);
    }
    out
}

/// Per-character glyph mapping and advance placement.
///
/// Placeholder for real shaping (rustybuzz, with `ssty` in script styles);
/// adequate for isolated math glyphs, which don't form clusters or ligate.
fn layout_text_run(ctx: &Ctx, text: &str) -> MathBox {
    let mut out = MathBox::empty();
    for c in text.chars() {
        let Some(gid) = ctx.font.glyph_index(c) else {
            // No .notdef rendering yet: skip unmapped characters.
            continue;
        };
        out.items.push(Item::Glyph {
            id: gid,
            x: out.width,
            y: 0.0,
            size: ctx.size,
        });
        out.width += ctx.font.advance(gid) * ctx.scale;
        if let Some(ink) = ctx.font.ink_box(gid) {
            out.ascent = out.ascent.max(ink.y_max as f32 * ctx.scale);
            out.descent = out.descent.max(-(ink.y_min as f32) * ctx.scale);
        }
    }
    // A run with no ink (all spaces) still occupies the line: fall back to
    // font-wide metrics so an empty-looking box doesn't collapse vertically.
    if !out.items.is_empty() && out.ascent == 0.0 && out.descent == 0.0 {
        let (asc, desc) = ctx.font.line_metrics();
        out.ascent = asc * ctx.scale;
        out.descent = desc * ctx.scale;
    }
    out
}
