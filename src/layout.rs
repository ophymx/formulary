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

use crate::ast::{DisplayMode, Form, Length, MathRoot, Node, ScriptLevel};
use crate::font::{GlyphId, MathFont, Stretched};
use crate::mathvariant::to_math_italic;
use crate::opdict;

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
    /// superscripts less.
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

    /// Child context under a radical: same size and style, but cramped.
    fn cramped_child(&self) -> Self {
        Ctx::derive(
            self.font,
            self.base_size,
            self.script_level,
            self.display_style,
            true,
        )
    }

    /// Child context for an `<mroot>` degree: two script levels up,
    /// displaystyle off (TeX's scriptscript style).
    fn root_degree_child(&self) -> Self {
        Ctx::derive(
            self.font,
            self.base_size,
            self.script_level.saturating_add(2),
            false,
            self.cramped,
        )
    }

    /// Child context for an accent script: accents keep their size (no
    /// script-level bump) so the mark stays as wide as its base.
    fn accent_child(&self, cramped: bool) -> Self {
        Ctx::derive(
            self.font,
            self.base_size,
            self.script_level,
            false,
            self.cramped || cramped,
        )
    }

    /// Child context for a subscript or superscript: script level rises,
    /// displaystyle switches off. Subscripts are additionally cramped.
    fn script_child(&self, cramped: bool) -> Self {
        Ctx::derive(
            self.font,
            self.base_size,
            self.script_level.saturating_add(1),
            false,
            self.cramped || cramped,
        )
    }

    /// Child context for `<mstyle>` overrides.
    fn styled_child(&self, display: Option<bool>, level: Option<ScriptLevel>) -> Self {
        let script_level = match level {
            None => self.script_level,
            Some(ScriptLevel::Set(n)) => n,
            Some(ScriptLevel::Add(d)) => {
                (i16::from(self.script_level) + i16::from(d)).clamp(0, 255) as u8
            }
        };
        Ctx::derive(
            self.font,
            self.base_size,
            script_level,
            display.unwrap_or(self.display_style),
            self.cramped,
        )
    }

    /// A MATH constant, converted from design units to output units at this
    /// context's scale.
    fn constant(&self, v: ttf_parser::math::MathValue) -> f32 {
        v.value as f32 * self.scale
    }

    /// Resolve a MathML length to layout units. Percentages resolve against
    /// `percent_ref` (the natural dimension for `mpadded`, zero elsewhere).
    fn resolve(&self, len: Length, percent_ref: f32) -> f32 {
        match len {
            Length::Em(v) => v * self.size,
            Length::Ex(v) => v * self.font.x_height() * self.scale,
            Length::Px(v) => v,
            Length::Pt(v) => v * 96.0 / 72.0,
            Length::Percent(v) => v / 100.0 * percent_ref,
        }
    }
}

/// Intermediate box; same shape as `Layout` but items are in box-local
/// coordinates (baseline at y = 0, left edge at x = 0).
#[derive(Default)]
struct MathBox {
    width: f32,
    ascent: f32,
    descent: f32,
    /// Italic correction of the box's trailing glyph (glyph runs and large
    /// operators; zero for composite constructs), consumed by script
    /// attachment.
    italic_correction: f32,
    items: Vec<Item>,
}

impl MathBox {
    fn empty() -> Self {
        Self::default()
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
        // An <mo> reached outside row context (e.g. as a script base) gets no
        // form-dependent spacing; the surrounding row handles spacing.
        Node::Operator { text, .. } => layout_text_run(ctx, text),
        Node::Number(text) | Node::Text(text) => layout_text_run(ctx, text),
        Node::Row(children) => layout_row(ctx, children),
        Node::Frac { num, den } => layout_frac(ctx, num, den),
        Node::Scripts { base, sub, sup } => {
            layout_scripts(ctx, base, sub.as_deref(), sup.as_deref())
        }
        Node::UnderOver {
            base,
            under,
            over,
            accent,
            accent_under,
        } => layout_underover(
            ctx,
            base,
            under.as_deref(),
            over.as_deref(),
            accent.unwrap_or(false),
            accent_under.unwrap_or(false),
        ),
        Node::Sqrt(children) => {
            let content = layout_row(&ctx.cramped_child(), children);
            layout_radical(ctx, content, None)
        }
        Node::Root { base, index } => {
            let content = layout_node(&ctx.cramped_child(), base);
            let degree = layout_node(&ctx.root_degree_child(), index);
            layout_radical(ctx, content, Some(degree))
        }
        Node::Space {
            width,
            height,
            depth,
        } => MathBox {
            width: width.map_or(0.0, |l| ctx.resolve(l, 0.0)).max(0.0),
            ascent: height.map_or(0.0, |l| ctx.resolve(l, 0.0)).max(0.0),
            descent: depth.map_or(0.0, |l| ctx.resolve(l, 0.0)).max(0.0),
            italic_correction: 0.0,
            items: Vec::new(),
        },
        Node::Styled {
            display_style,
            script_level,
            children,
        } => layout_row(&ctx.styled_child(*display_style, *script_level), children),
        Node::Phantom(children) => {
            let mut b = layout_row(ctx, children);
            b.items.clear();
            b
        }
        Node::Padded {
            width,
            height,
            depth,
            lspace,
            voffset,
            children,
        } => {
            let natural = layout_row(ctx, children);
            let dx = lspace.map_or(0.0, |l| ctx.resolve(l, 0.0)).max(0.0);
            // Positive voffset moves the content up.
            let dy = -voffset.map_or(0.0, |l| ctx.resolve(l, 0.0));
            let mut out = MathBox {
                width: width
                    .map_or(natural.width, |l| ctx.resolve(l, natural.width))
                    .max(0.0),
                ascent: height
                    .map_or(natural.ascent, |l| ctx.resolve(l, natural.ascent))
                    .max(0.0),
                descent: depth
                    .map_or(natural.descent, |l| ctx.resolve(l, natural.descent))
                    .max(0.0),
                italic_correction: natural.italic_correction,
                items: natural.items,
            };
            for item in &mut out.items {
                item.translate(dx, dy);
            }
            out
        }
    }
}

const RADICAL_CHAR: char = '\u{221A}';

/// `<msqrt>`/`<mroot>` per the OpenType MATH radical constants: the radical
/// glyph is the vertical variant covering the radicand's height plus the
/// minimum gap and rule; the overbar continues from its top across the
/// radicand, with RadicalExtraAscender white space above.
fn layout_radical(ctx: &Ctx, content: MathBox, degree: Option<MathBox>) -> MathBox {
    let c = ctx.font.constants();
    let gap_min = if ctx.display_style {
        ctx.constant(c.radical_display_style_vertical_gap())
    } else {
        ctx.constant(c.radical_vertical_gap())
    };
    let thickness = ctx.constant(c.radical_rule_thickness());

    // Pick or assemble a radical glyph tall enough for radicand + gap + rule.
    let target = content.ascent + content.descent + gap_min + thickness;
    let stretched = ctx
        .font
        .glyph_index(RADICAL_CHAR)
        .map(|base| ctx.font.stretch_vertical(base, target / ctx.scale));
    let glyph_height = stretched.as_ref().map_or(0.0, |s| {
        let mut probe = Vec::new();
        emit_stretched(ctx, s, 0.0, 0.0, &mut probe).0
    });

    // A taller-than-needed glyph centers its excess: half widens the gap,
    // half hangs below the radicand (TeX rule 11).
    let gap = gap_min + (glyph_height - target).max(0.0) / 2.0;
    // Top edge of the overbar; the radical glyph's ink top aligns with it.
    let bar_top = -(content.ascent + gap + thickness);

    let mut out = MathBox::empty();
    out.ascent = content.ascent + gap + thickness + ctx.constant(c.radical_extra_ascender());
    out.descent = content.descent.max(bar_top + glyph_height);

    // Horizontal assembly: [kern degree kern] glyph, radicand under the bar.
    let mut x = 0.0;
    if let Some(deg) = degree {
        x += ctx.constant(c.radical_kern_before_degree());
        // Degree bottom sits this fraction of the glyph's span above its bottom.
        let raise = f32::from(c.radical_degree_bottom_raise_percent()) / 100.0;
        let glyph_bottom = bar_top + glyph_height;
        let deg_baseline = glyph_bottom - raise * glyph_height - deg.descent;
        out.ascent = out.ascent.max(deg.ascent - deg_baseline);
        out.descent = out.descent.max(deg_baseline + deg.descent);
        for mut item in deg.items {
            item.translate(x, deg_baseline);
            out.items.push(item);
        }
        x += deg.width + ctx.constant(c.radical_kern_after_degree());
        x = x.max(0.0); // a large negative kern must not push the glyph out
    }
    if let Some(s) = &stretched {
        let (_, advance) = emit_stretched(ctx, s, x, bar_top, &mut out.items);
        x += advance;
    }
    out.items.push(Item::Rule {
        x,
        y: bar_top,
        w: content.width,
        h: thickness,
    });
    for mut item in content.items {
        item.translate(x, 0.0);
        out.items.push(item);
    }
    out.width = x + content.width;
    out
}

/// `<munder>`/`<mover>`/`<munderover>`.
///
/// Movable limits render as sub/superscripts outside display style. Accented
/// scripts keep full size and hug the base at AccentBaseHeight; limits over
/// large operators use the Upper/LowerLimit constants; everything else uses
/// the over/underbar gaps. Lone stretchy horizontal operators in any slot
/// stretch to the widest slot's natural width.
fn layout_underover(
    ctx: &Ctx,
    base: &Node,
    under: Option<&Node>,
    over: Option<&Node>,
    accent: bool,
    accent_under: bool,
) -> MathBox {
    let base_flags = core_operator(base)
        .map(|(text, _)| dictionary_entry(text, Form::Infix).2)
        .unwrap_or(0);
    if !ctx.display_style {
        let movable = core_operator(base).is_some_and(|(_, attrs)| {
            attrs
                .movablelimits
                .unwrap_or(base_flags & opdict::MOVABLE_LIMITS != 0)
        });
        if movable {
            return layout_scripts(ctx, base, under, over);
        }
    }

    let base_box = layout_operator_base(ctx, base);
    let over_ctx = if accent {
        ctx.accent_child(false)
    } else {
        ctx.script_child(false)
    };
    let under_ctx = if accent_under {
        ctx.accent_child(true)
    } else {
        ctx.script_child(true)
    };
    let over_box = over.map(|n| layout_node(&over_ctx, n));
    let under_box = under.map(|n| layout_node(&under_ctx, n));

    // Horizontal stretching to the widest slot.
    let width = base_box
        .width
        .max(over_box.as_ref().map_or(0.0, |b| b.width))
        .max(under_box.as_ref().map_or(0.0, |b| b.width));
    let stretch_h = |ctx: &Ctx, node: &Node, natural: MathBox| -> MathBox {
        if natural.width < width - 1e-3 {
            if let Node::Operator { text, attrs } = node {
                if let Some(c) = single_char(text) {
                    let flags = dictionary_entry(text, Form::Infix).2;
                    if attrs.stretchy.unwrap_or(flags & opdict::STRETCHY != 0)
                        && flags & opdict::HORIZONTAL != 0
                    {
                        if let Some(g) = ctx.font.glyph_index(c) {
                            let stretched =
                                ctx.font.stretch_horizontal(g, width / ctx.scale);
                            return layout_stretched_horizontal(ctx, &stretched);
                        }
                    }
                }
            }
        }
        natural
    };
    let base_box = stretch_h(ctx, base, base_box);
    let over_box = over.map(|n| stretch_h(&over_ctx, n, over_box.expect("laid out above")));
    let under_box =
        under.map(|n| stretch_h(&under_ctx, n, under_box.expect("laid out above")));

    let c = ctx.font.constants();
    // Operators carrying limits use the limit constants; plain bases the bar
    // constants.
    let limits_base = base_flags & (opdict::LARGEOP | opdict::MOVABLE_LIMITS) != 0;

    let mut out = MathBox::empty();
    out.width = width;
    out.ascent = base_box.ascent;
    out.descent = base_box.descent;
    let dx = (width - base_box.width) / 2.0;
    for mut item in base_box.items {
        item.translate(dx, 0.0);
        out.items.push(item);
    }

    if let Some(ob) = over_box {
        let (shift_up, extra_ascender) = if accent {
            // The accent glyph is drawn for a base of AccentBaseHeight; raise
            // it only by however much the base exceeds that.
            (
                (base_box.ascent - ctx.constant(c.accent_base_height())).max(0.0),
                0.0,
            )
        } else if limits_base {
            (
                base_box.ascent
                    + ctx
                        .constant(c.upper_limit_baseline_rise_min())
                        .max(ctx.constant(c.upper_limit_gap_min()) + ob.descent),
                0.0,
            )
        } else {
            (
                base_box.ascent + ctx.constant(c.overbar_vertical_gap()) + ob.descent,
                ctx.constant(c.overbar_extra_ascender()),
            )
        };
        out.ascent = out.ascent.max(shift_up + ob.ascent + extra_ascender);
        let dx = (width - ob.width) / 2.0;
        for mut item in ob.items {
            item.translate(dx, -shift_up);
            out.items.push(item);
        }
    }

    if let Some(ub) = under_box {
        let (shift_down, extra_descender) = if accent_under {
            ((base_box.descent + ub.ascent).max(0.0), 0.0)
        } else if limits_base {
            (
                base_box.descent
                    + ctx
                        .constant(c.lower_limit_baseline_drop_min())
                        .max(ctx.constant(c.lower_limit_gap_min()) + ub.ascent),
                0.0,
            )
        } else {
            (
                base_box.descent + ctx.constant(c.underbar_vertical_gap()) + ub.ascent,
                ctx.constant(c.underbar_extra_descender()),
            )
        };
        out.descent = out.descent.max(shift_down + ub.descent + extra_descender);
        let dx = (width - ub.width) / 2.0;
        for mut item in ub.items {
            item.translate(dx, shift_down);
            out.items.push(item);
        }
    }
    out
}

/// `<msub>`/`<msup>`/`<msubsup>` per the OpenType MATH script constants
/// (the TeX Appendix G rules 18a–f recast in font terms).
///
/// Italic correction of the base is not yet applied to the superscript
/// offset; that lands together with per-glyph MathGlyphInfo access.
fn layout_scripts(ctx: &Ctx, base: &Node, sub: Option<&Node>, sup: Option<&Node>) -> MathBox {
    let base_box = layout_operator_base(ctx, base);
    layout_scripts_on(ctx, base_box, sub, sup)
}

/// Script attachment to an already-laid base (shared with embellished
/// stretchy operators, whose base is stretched before scripts attach).
fn layout_scripts_on(
    ctx: &Ctx,
    base_box: MathBox,
    sub: Option<&Node>,
    sup: Option<&Node>,
) -> MathBox {
    let sub_box = sub.map(|n| layout_node(&ctx.script_child(true), n));
    let sup_box = sup.map(|n| layout_node(&ctx.script_child(false), n));

    let c = ctx.font.constants();

    // Superscript shift above the baseline (u in TeX terms).
    let mut sup_shift = 0.0_f32;
    if let Some(s) = &sup_box {
        let preferred = if ctx.cramped {
            ctx.constant(c.superscript_shift_up_cramped())
        } else {
            ctx.constant(c.superscript_shift_up())
        };
        sup_shift = preferred
            // Don't drop the script baseline too far below the base's top.
            .max(base_box.ascent - ctx.constant(c.superscript_baseline_drop_max()))
            // Keep the superscript's bottom ink above SuperscriptBottomMin.
            .max(ctx.constant(c.superscript_bottom_min()) + s.descent);
    }

    // Subscript shift below the baseline (v in TeX terms).
    let mut sub_shift = 0.0_f32;
    if let Some(s) = &sub_box {
        sub_shift = ctx
            .constant(c.subscript_shift_down())
            // Hang the script baseline at least this far below the base's bottom.
            .max(base_box.descent + ctx.constant(c.subscript_baseline_drop_min()))
            // Keep the subscript's top ink below SubscriptTopMax.
            .max(s.ascent - ctx.constant(c.subscript_top_max()));
    }

    // With both scripts, keep them apart: grow the gap first by raising the
    // superscript (up to SuperscriptBottomMaxWithSubscript), then by pushing
    // the subscript down.
    if let (Some(sb), Some(sp)) = (&sub_box, &sup_box) {
        let gap = (sup_shift - sp.descent) + (sub_shift - sb.ascent);
        let mut deficit = ctx.constant(c.sub_superscript_gap_min()) - gap;
        if deficit > 0.0 {
            let headroom = ctx.constant(c.superscript_bottom_max_with_subscript())
                - (sup_shift - sp.descent);
            if headroom > 0.0 {
                let up = deficit.min(headroom);
                sup_shift += up;
                deficit -= up;
            }
            sub_shift += deficit.max(0.0);
        }
    }

    // Superscripts attach at the full advance; subscripts tuck left by the
    // base's italic correction (the classic ∫ lower-limit tuck).
    let sup_x = base_box.width;
    let sub_x = (base_box.width - base_box.italic_correction).max(0.0);
    let end = sub_box
        .as_ref()
        .map_or(0.0, |b| sub_x + b.width)
        .max(sup_box.as_ref().map_or(0.0, |b| sup_x + b.width))
        .max(base_box.width);

    let mut out = MathBox {
        width: end + ctx.constant(c.space_after_script()),
        ascent: base_box.ascent,
        descent: base_box.descent,
        italic_correction: 0.0,
        items: base_box.items,
    };
    if let Some(s) = sup_box {
        out.ascent = out.ascent.max(sup_shift + s.ascent);
        out.descent = out.descent.max(s.descent - sup_shift);
        for mut item in s.items {
            item.translate(sup_x, -sup_shift);
            out.items.push(item);
        }
    }
    if let Some(s) = sub_box {
        out.ascent = out.ascent.max(s.ascent - sub_shift);
        out.descent = out.descent.max(sub_shift + s.descent);
        for mut item in s.items {
            item.translate(sub_x, sub_shift);
            out.items.push(item);
        }
    }
    out
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
        italic_correction: 0.0,
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

/// Horizontal concatenation on a shared baseline, with operator-dictionary
/// spacing around `<mo>` children and vertical stretching of stretchy ones.
fn layout_row(ctx: &Ctx, children: &[Node]) -> MathBox {
    // Positions among non-space-like siblings decide each operator's form.
    let significant: Vec<usize> = children
        .iter()
        .enumerate()
        .filter(|(_, c)| !is_space_like(c))
        .map(|(i, _)| i)
        .collect();

    // Pass 1: lay out everything except vertically-stretchy (possibly
    // embellished) operators, which must wait until the extent of their
    // siblings is known. Spacing and stretchiness come from the embellished
    // operator's core `<mo>`, so `<msup><mo>)</mo>…</msup>` spaces and
    // stretches like the fence it wraps.
    enum Slot<'n> {
        Fixed(MathBox),
        Stretchy { node: &'n Node, symmetric: bool },
    }
    let mut slots: Vec<(f32, f32, Slot)> = Vec::with_capacity(children.len());
    let (mut max_ascent, mut max_descent) = (0.0_f32, 0.0_f32);
    for (i, child) in children.iter().enumerate() {
        let slot = if let Some((text, attrs)) = core_operator(child) {
            let form = attrs.form.unwrap_or_else(|| infer_form(i, &significant));
            let (dict_l, dict_r, flags) = dictionary_entry(text, form);
            let em = ctx.size / 18.0;
            let lspace = attrs
                .lspace
                .map_or(f32::from(dict_l) * em, |l| ctx.resolve(l, 0.0).max(0.0));
            let rspace = attrs
                .rspace
                .map_or(f32::from(dict_r) * em, |l| ctx.resolve(l, 0.0).max(0.0));
            let stretchy = attrs.stretchy.unwrap_or(flags & opdict::STRETCHY != 0)
                && flags & opdict::HORIZONTAL == 0
                && single_char(text).and_then(|c| ctx.font.glyph_index(c)).is_some();
            if stretchy {
                let symmetric = attrs.symmetric.unwrap_or(flags & opdict::SYMMETRIC != 0);
                (lspace, rspace, Slot::Stretchy { node: child, symmetric })
            } else if let Node::Operator { text, attrs } = child {
                // Direct <mo>: large-operator treatment happens here; for
                // embellished wrappers layout_operator_base handles it.
                match single_char(text) {
                    Some(c)
                        if ctx.display_style
                            && attrs.largeop.unwrap_or(flags & opdict::LARGEOP != 0) =>
                    {
                        (lspace, rspace, Slot::Fixed(layout_large_operator(ctx, c)))
                    }
                    _ => (lspace, rspace, Slot::Fixed(layout_text_run(ctx, text))),
                }
            } else {
                (lspace, rspace, Slot::Fixed(layout_node(ctx, child)))
            }
        } else {
            (0.0, 0.0, Slot::Fixed(layout_node(ctx, child)))
        };
        if let (_, _, Slot::Fixed(b)) = &slot {
            max_ascent = max_ascent.max(b.ascent);
            max_descent = max_descent.max(b.descent);
        }
        slots.push(slot);
    }

    // Pass 2: stretch deferred operators to the row's extent, then assemble.
    let mut out = MathBox::empty();
    for (lspace, rspace, slot) in slots {
        let mut b = match slot {
            Slot::Fixed(b) => b,
            Slot::Stretchy { node, symmetric } => {
                layout_embellished_stretchy(ctx, node, symmetric, max_ascent, max_descent)
            }
        };
        for item in &mut b.items {
            item.translate(out.width + lspace, 0.0);
        }
        out.items.append(&mut b.items);
        out.width += lspace + b.width + rspace;
        out.ascent = out.ascent.max(b.ascent);
        out.descent = out.descent.max(b.descent);
        out.italic_correction = b.italic_correction;
    }
    out
}

/// Stretch the core of an embellished operator to the row extent, then
/// re-wrap it in its script attachments. Under/over embellishments lay out
/// unstretched (they'd need their own re-wrapping pass).
fn layout_embellished_stretchy(
    ctx: &Ctx,
    node: &Node,
    symmetric: bool,
    max_ascent: f32,
    max_descent: f32,
) -> MathBox {
    match node {
        Node::Operator { text, .. } => match single_char(text) {
            Some(c) => layout_stretchy_operator(ctx, c, symmetric, max_ascent, max_descent),
            None => layout_node(ctx, node),
        },
        Node::Scripts { base, sub, sup } => {
            let base_box =
                layout_embellished_stretchy(ctx, base, symmetric, max_ascent, max_descent);
            layout_scripts_on(ctx, base_box, sub.as_deref(), sup.as_deref())
        }
        _ => layout_node(ctx, node),
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// Emit a [`Stretched`] glyph with its ink top at `top` (layout units),
/// returning `(ink height, advance width)` in layout units.
fn emit_stretched(ctx: &Ctx, stretched: &Stretched, x: f32, top: f32, items: &mut Vec<Item>) -> (f32, f32) {
    match stretched {
        Stretched::Glyph(g) => {
            let Some(ink) = ctx.font.ink_box(*g) else {
                return (0.0, ctx.font.advance(*g) * ctx.scale);
            };
            items.push(Item::Glyph {
                id: *g,
                x,
                y: top + f32::from(ink.y_max) * ctx.scale,
                size: ctx.size,
            });
            (
                f32::from(ink.y_max - ink.y_min) * ctx.scale,
                ctx.font.advance(*g) * ctx.scale,
            )
        }
        Stretched::Assembly { parts, extent } => {
            let height = extent * ctx.scale;
            let bottom = top + height;
            let mut advance = 0.0_f32;
            for &(g, offset) in parts {
                advance = advance.max(ctx.font.advance(g) * ctx.scale);
                let Some(ink) = ctx.font.ink_box(g) else { continue };
                // Part's ink bottom sits `offset` above the assembly bottom.
                items.push(Item::Glyph {
                    id: g,
                    x,
                    y: bottom - offset * ctx.scale + f32::from(ink.y_min) * ctx.scale,
                    size: ctx.size,
                });
            }
            (height, advance)
        }
    }
}

/// Emit a horizontally-stretched glyph starting at `x` on the baseline,
/// returning its box (width from the stretch axis, ascent/descent from ink).
fn layout_stretched_horizontal(ctx: &Ctx, stretched: &Stretched) -> MathBox {
    let mut out = MathBox::empty();
    match stretched {
        Stretched::Glyph(g) => {
            out.items.push(Item::Glyph {
                id: *g,
                x: 0.0,
                y: 0.0,
                size: ctx.size,
            });
            out.width = ctx.font.advance(*g) * ctx.scale;
            if let Some(ink) = ctx.font.ink_box(*g) {
                out.ascent = f32::from(ink.y_max) * ctx.scale;
                out.descent = -f32::from(ink.y_min) * ctx.scale;
            }
        }
        Stretched::Assembly { parts, extent } => {
            out.width = extent * ctx.scale;
            for &(g, offset) in parts {
                out.items.push(Item::Glyph {
                    id: g,
                    x: offset * ctx.scale,
                    y: 0.0,
                    size: ctx.size,
                });
                if let Some(ink) = ctx.font.ink_box(g) {
                    out.ascent = out.ascent.max(f32::from(ink.y_max) * ctx.scale);
                    out.descent = out.descent.max(-f32::from(ink.y_min) * ctx.scale);
                }
            }
        }
    }
    out
}

/// The core `<mo>` of an embellished operator: the node itself, or the base
/// of scripts/under-over wrappers (MathML Core's embellished-operator
/// definition, without the space-like-row cases).
fn core_operator(node: &Node) -> Option<(&str, &crate::ast::OperatorAttrs)> {
    match node {
        Node::Operator { text, attrs } => Some((text, attrs)),
        Node::Scripts { base, .. } | Node::UnderOver { base, .. } => core_operator(base),
        _ => None,
    }
}

/// Lay out a node that may be a lone large operator: in display style the
/// base of scripts/limits picks its DisplayOperatorMinHeight variant.
fn layout_operator_base(ctx: &Ctx, node: &Node) -> MathBox {
    if let Node::Operator { text, attrs } = node {
        if ctx.display_style {
            if let Some(c) = single_char(text) {
                let flags = dictionary_entry(text, Form::Infix).2;
                if attrs.largeop.unwrap_or(flags & opdict::LARGEOP != 0) {
                    return layout_large_operator(ctx, c);
                }
            }
        }
    }
    layout_node(ctx, node)
}

/// A vertically-stretchy operator covering the row's extent: symmetric ones
/// (fences) grow equally about the math axis, others cover ascent + descent
/// directly. Excess from a too-tall variant is centered over the target.
fn layout_stretchy_operator(
    ctx: &Ctx,
    c: char,
    symmetric: bool,
    max_ascent: f32,
    max_descent: f32,
) -> MathBox {
    let glyph = ctx.font.glyph_index(c).expect("checked by caller");
    let axis = ctx.constant(ctx.font.constants().axis_height());
    let (target, target_ascent) = if symmetric {
        let above = (max_ascent - axis).max(max_descent + axis).max(0.0);
        (2.0 * above, axis + above)
    } else {
        ((max_ascent + max_descent).max(0.0), max_ascent)
    };
    if target <= 0.0 {
        // Nothing to cover (row of only stretchy operators): natural glyph.
        return layout_text_run(ctx, &c.to_string());
    }
    let stretched = ctx.font.stretch_vertical(glyph, target / ctx.scale);
    let mut out = MathBox::empty();
    // Probe the actual ink height first to center any excess.
    let mut probe = Vec::new();
    let (height, _) = emit_stretched(ctx, &stretched, 0.0, 0.0, &mut probe);
    let ascent = target_ascent + (height - target).max(0.0) / 2.0;
    let (height, advance) = emit_stretched(ctx, &stretched, 0.0, -ascent, &mut out.items);
    out.width = advance;
    out.ascent = ascent;
    out.descent = height - ascent;
    out
}

/// A large operator (∫, ∑, …) in display style: the variant reaching
/// DisplayOperatorMinHeight, vertically centered on the math axis.
fn layout_large_operator(ctx: &Ctx, c: char) -> MathBox {
    let Some(glyph) = ctx.font.glyph_index(c) else {
        return layout_text_run(ctx, &c.to_string());
    };
    let min_height = f32::from(ctx.font.constants().display_operator_min_height());
    let stretched = ctx.font.stretch_vertical(glyph, min_height);
    let axis = ctx.constant(ctx.font.constants().axis_height());
    let mut probe = Vec::new();
    let (height, _) = emit_stretched(ctx, &stretched, 0.0, 0.0, &mut probe);
    let ascent = axis + height / 2.0;
    let mut out = MathBox::empty();
    let (height, advance) = emit_stretched(ctx, &stretched, 0.0, -ascent, &mut out.items);
    out.width = advance;
    out.ascent = ascent;
    out.descent = height - ascent;
    if let Stretched::Glyph(g) = stretched {
        out.italic_correction = ctx.font.italic_correction(g) * ctx.scale;
    }
    out
}

/// Elements that don't count as operands when inferring operator forms
/// (MathML Core's space-like definition).
fn is_space_like(node: &Node) -> bool {
    match node {
        Node::Space { .. } | Node::Text(_) => true,
        Node::Row(children) | Node::Phantom(children) => {
            children.iter().all(is_space_like)
        }
        Node::Styled { children, .. } | Node::Padded { children, .. } => {
            children.iter().all(is_space_like)
        }
        _ => false,
    }
}

/// Form by position: first of several operands → prefix, last → postfix,
/// otherwise (including a lone child) infix.
fn infer_form(index: usize, significant: &[usize]) -> Form {
    if significant.len() > 1 {
        if significant.first() == Some(&index) {
            return Form::Prefix;
        }
        if significant.last() == Some(&index) {
            return Form::Postfix;
        }
    }
    Form::Infix
}

/// Dictionary lookup with the spec's fallback chain (requested form, then
/// infix → postfix → prefix), ending at the default entry (5/18 em each
/// side, no properties). Multi-character operators aren't in the dictionary
/// and get the default.
fn dictionary_entry(text: &str, form: Form) -> (u8, u8, u8) {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return (5, 5, 0);
    };
    let requested = match form {
        Form::Infix => opdict::FORM_INFIX,
        Form::Prefix => opdict::FORM_PREFIX,
        Form::Postfix => opdict::FORM_POSTFIX,
    };
    for f in [
        requested,
        opdict::FORM_INFIX,
        opdict::FORM_POSTFIX,
        opdict::FORM_PREFIX,
    ] {
        if let Some(entry) = opdict::lookup(c, f) {
            return entry;
        }
    }
    (5, 5, 0)
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
        out.italic_correction = ctx.font.italic_correction(gid) * ctx.scale;
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
