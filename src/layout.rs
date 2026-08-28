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

use crate::ast::{
    Color, ColumnAlign, Direction, DisplayMode, Form, Length, MathRoot, Node, ScriptLevel,
    StyleOverrides, TableCell,
};
use crate::font::{GlyphId, KernCorner, MathFont, Stretched};
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
    /// differs from the top-level font size inside scripts). `color: None`
    /// means the consumer's text color, so unstyled math matches the
    /// surrounding text.
    Glyph {
        id: GlyphId,
        x: f32,
        y: f32,
        size: f32,
        color: Option<Color>,
        /// Draw the glyph flipped horizontally about its advance box (used
        /// for the radical in right-to-left math, where fonts rarely ship
        /// pre-mirrored forms).
        mirrored: bool,
    },
    /// A filled rectangle (fraction bars, radical rules, merror borders).
    /// `(x, y)` is the top-left corner; `h` extends downward. `color: None`
    /// means the consumer's text color.
    Rule {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Option<Color>,
    },
    /// A `mathbackground` fill behind part of the formula. Emitted before
    /// the items it sits behind, so drawing in list order is correct.
    Background {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
    },
}

impl Item {
    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Item::Glyph { x, y, .. } | Item::Rule { x, y, .. } | Item::Background { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
        }
    }
}

/// Size multiplier for a script level, from the font's scale-down percents.
fn script_factor(font: &MathFont, script_level: u8) -> f32 {
    let percent = |v: i16, fallback: f32| {
        if v > 0 {
            v as f32 / 100.0
        } else {
            fallback
        }
    };
    let consts = font.constants();
    match script_level {
        0 => 1.0,
        1 => percent(consts.script_percent_scale_down(), 0.71),
        _ => percent(consts.script_script_percent_scale_down(), 0.5041),
    }
}

/// Lay out a parsed formula against `font` at `options.font_size`.
pub fn layout(root: &MathRoot, font: &MathFont, options: &LayoutOptions) -> Layout {
    let ctx = Ctx::new(
        font,
        options.font_size,
        // Per MathML Core, display="block" starts in displaystyle and inline
        // math in text style, unless the displaystyle attribute overrides.
        root.displaystyle
            .unwrap_or(root.display == DisplayMode::Block),
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
    /// Inherited `mathcolor`; `None` is the consumer's text color.
    color: Option<Color>,
    /// Right-to-left layout (`dir="rtl"`).
    rtl: bool,
    /// Font size at the current script level, stamped on emitted glyphs.
    size: f32,
    /// Font design units → output units at the current script level.
    scale: f32,
}

impl<'a, 'f> Ctx<'a, 'f> {
    fn new(font: &'a MathFont<'f>, base_size: f32, display_style: bool) -> Self {
        Self::derive(font, base_size, 0, display_style, false, None, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn derive(
        font: &'a MathFont<'f>,
        base_size: f32,
        script_level: u8,
        display_style: bool,
        cramped: bool,
        color: Option<Color>,
        rtl: bool,
    ) -> Self {
        let size = base_size * script_factor(font, script_level);
        Ctx {
            font,
            base_size,
            script_level,
            display_style,
            cramped,
            color,
            rtl,
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
        Ctx::derive(
            self.font,
            self.base_size,
            level,
            false,
            self.cramped || cramped,
            self.color,
            self.rtl,
        )
    }

    /// Child context under a radical: same size and style, but cramped.
    fn cramped_child(&self) -> Self {
        Ctx::derive(
            self.font,
            self.base_size,
            self.script_level,
            self.display_style,
            true,
            self.color,
            self.rtl,
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
            self.color,
            self.rtl,
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
            self.color,
            self.rtl,
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
            self.color,
            self.rtl,
        )
    }

    /// Child context under a style scope (`mstyle` or any element's global
    /// style attributes).
    fn styled_child(&self, styles: &StyleOverrides) -> Self {
        let script_level = match styles.script_level {
            None => self.script_level,
            Some(ScriptLevel::Set(n)) => n,
            Some(ScriptLevel::Add(d)) => {
                (i16::from(self.script_level) + i16::from(d)).clamp(0, 255) as u8
            }
        };
        // mathsize sets the size at this node; scripts below still scale
        // relative to it, so back out the script factor from the base size.
        let base_size = match styles.math_size {
            Some(len) => {
                let target = self.resolve(len, self.size).max(0.0);
                target / script_factor(self.font, script_level)
            }
            None => self.base_size,
        };
        Ctx::derive(
            self.font,
            base_size,
            script_level,
            styles.display_style.unwrap_or(self.display_style),
            self.cramped,
            styles.color.or(self.color),
            match styles.dir {
                Some(d) => d == Direction::Rtl,
                None => self.rtl,
            },
        )
    }

    /// A MATH constant, converted from design units to output units at this
    /// context's scale.
    fn constant(&self, v: ttf_parser::math::MathValue) -> f32 {
        v.value as f32 * self.scale
    }

    /// Swap in the font's `ssty` alternate in script styles.
    fn script_glyph(&self, glyph: GlyphId) -> GlyphId {
        if self.script_level == 0 {
            return glyph;
        }
        self.font
            .script_alternate(glyph, u16::from(self.script_level.min(2)))
            .unwrap_or(glyph)
    }

    /// Resolve a MathML length to layout units. Percentages resolve against
    /// `percent_ref` — whatever natural dimension the attribute is defined
    /// against (an `mpadded` box dimension, the default rule thickness for
    /// `linethickness`, …); callers with no meaningful reference pass zero.
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
    /// The glyph, when this box is exactly one glyph — the case where the
    /// font's per-glyph MathKern and top-accent-attachment data applies.
    lone_glyph: Option<GlyphId>,
    items: Vec<Item>,
}

impl MathBox {
    fn empty() -> Self {
        Self::default()
    }
}

fn layout_node(ctx: &Ctx, node: &Node) -> MathBox {
    match node {
        // The parser already applied mathvariant / auto-italic mappings.
        Node::Identifier(text) => layout_text_run(ctx, text),
        // An <mo> reached outside row context (e.g. as a script base) gets no
        // form-dependent spacing; the surrounding row handles spacing.
        Node::Operator { text, .. } => layout_text_run(ctx, text),
        Node::Number(text) | Node::Text(text) => layout_text_run(ctx, text),
        // A single-child mrow is transparent: its child's operator spacing
        // and embellishments belong to the enclosing row, so no inner row is
        // formed (which would apply operator spacing a second time).
        Node::Row(children) if children.len() == 1 => layout_node(ctx, &children[0]),
        Node::Row(children) => layout_row(ctx, children),
        Node::Frac {
            num,
            den,
            line_thickness,
        } => layout_frac(ctx, num, den, *line_thickness),
        Node::Scripts { base, sub, sup } => {
            layout_scripts(ctx, base, sub.as_deref(), sup.as_deref())
        }
        // In RTL the post/pre script sides mirror.
        Node::MultiScripts { base, post, pre } if ctx.rtl => {
            layout_multiscripts(ctx, base, pre, post)
        }
        Node::MultiScripts { base, post, pre } => layout_multiscripts(ctx, base, post, pre),
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
        Node::Table { rows, column_align } => layout_table(ctx, rows, column_align),
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
            lone_glyph: None,
            items: Vec::new(),
        },
        Node::Styled { styles, children } => layout_styled(ctx, styles, children),
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
                lone_glyph: natural.lone_glyph,
                items: natural.items,
            };
            for item in &mut out.items {
                item.translate(dx, dy);
            }
            out
        }
    }
}

/// A style scope: lay the children out under the overridden context, then
/// paint `mathbackground` behind them and the merror border around them.
fn layout_styled(ctx: &Ctx, styles: &StyleOverrides, children: &[Node]) -> MathBox {
    let inner = ctx.styled_child(styles);
    // Style wrappers around a single element are transparent like
    // single-child mrows: the enclosing row owns the child's operator
    // spacing.
    let b = match children {
        [only] => layout_node(&inner, only),
        _ => layout_row(&inner, children),
    };
    decorate(styles, b)
}

/// Prepend the background fill and append the border rules for a style
/// scope's box.
fn decorate(styles: &StyleOverrides, mut b: MathBox) -> MathBox {
    if styles.background.is_none() && styles.border.is_none() {
        return b;
    }
    let (x, y, w, h) = (0.0, -b.ascent, b.width, b.ascent + b.descent);
    if let Some(color) = styles.background {
        b.items.insert(0, Item::Background { x, y, w, h, color });
    }
    if let Some(color) = styles.border {
        let t = 1.0; // 1px, per the merror user-agent styling
        let edge = |x, y, w, h| Item::Rule {
            x,
            y,
            w,
            h,
            color: Some(color),
        };
        b.items.push(edge(x, y, w, t));
        b.items.push(edge(x, y + h - t, w, t));
        b.items.push(edge(x, y, t, h));
        b.items.push(edge(x + w - t, y, t, h));
    }
    b
}

/// `<mtable>`: cells baseline-aligned within each row, columns sized to
/// their widest cell, and the whole table vertically centered on the math
/// axis. Cells lay out in text style (displaystyle off), per MathML Core,
/// and carry Core's UA-stylesheet cell padding (0.4 em / 0.5 ex per side).
/// `columnalign` sets per-column alignment (default center; last entry
/// repeats).
fn layout_table(ctx: &Ctx, rows: &[Vec<TableCell>], column_align: &[ColumnAlign]) -> MathBox {
    let cell_ctx = ctx.styled_child(&StyleOverrides {
        display_style: Some(false),
        ..StyleOverrides::default()
    });

    // Grid placement with occupancy, as in HTML tables: cells slide right
    // past slots claimed by earlier row/column spans.
    struct Placed {
        row: usize,
        col: usize,
        row_span: usize,
        col_span: usize,
        content: MathBox,
    }
    let n_rows = rows.len();
    let mut occupied: Vec<Vec<bool>> = vec![Vec::new(); n_rows];
    let mut placed: Vec<Placed> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut c = 0usize;
        for cell in row {
            while occupied[r].get(c).copied().unwrap_or(false) {
                c += 1;
            }
            let row_span = (cell.row_span as usize).clamp(1, n_rows - r);
            let col_span = (cell.col_span as usize).max(1);
            for occ_row in occupied.iter_mut().skip(r).take(row_span) {
                if occ_row.len() < c + col_span {
                    occ_row.resize(c + col_span, false);
                }
                for slot in occ_row.iter_mut().skip(c).take(col_span) {
                    *slot = true;
                }
            }
            placed.push(Placed {
                row: r,
                col: c,
                row_span,
                col_span,
                content: layout_node(&cell_ctx, &cell.content),
            });
            c += col_span;
        }
    }
    let n_cols = occupied.iter().map(Vec::len).max().unwrap_or(0);

    let hpad = 0.4 * ctx.size;
    let vpad = 0.5 * ctx.font.x_height() * ctx.scale;

    // Column widths from span-1 cells first, then widen spanned columns
    // evenly when a spanning cell needs more room (a span absorbs the
    // inter-column padding it crosses).
    let mut col_widths = vec![0.0_f32; n_cols];
    for p in placed.iter().filter(|p| p.col_span == 1) {
        col_widths[p.col] = col_widths[p.col].max(p.content.width);
    }
    let mut spanning: Vec<&Placed> = placed.iter().filter(|p| p.col_span > 1).collect();
    spanning.sort_by_key(|p| p.col_span);
    for p in spanning {
        let cols = &mut col_widths[p.col..p.col + p.col_span];
        let available = cols.iter().sum::<f32>() + 2.0 * hpad * (p.col_span - 1) as f32;
        let deficit = p.content.width - available;
        if deficit > 0.0 {
            let share = deficit / p.col_span as f32;
            for w in cols {
                *w += share;
            }
        }
    }

    // Row extents likewise: span-1 cells set each row's ascent/descent
    // (a rowspan cell's ascent still belongs to its first row, where its
    // baseline sits); deeper cells then grow the descents of the rows they
    // span.
    let mut row_ascent = vec![0.0_f32; n_rows];
    let mut row_descent = vec![0.0_f32; n_rows];
    for p in &placed {
        row_ascent[p.row] = row_ascent[p.row].max(p.content.ascent);
        if p.row_span == 1 {
            row_descent[p.row] = row_descent[p.row].max(p.content.descent);
        }
    }
    let mut row_spanning: Vec<&Placed> = placed.iter().filter(|p| p.row_span > 1).collect();
    row_spanning.sort_by_key(|p| p.row_span);
    for p in row_spanning {
        let below: f32 = (p.row + 1..p.row + p.row_span)
            .map(|rr| row_ascent[rr] + row_descent[rr] + 2.0 * vpad)
            .sum();
        let deficit = p.content.descent - (row_descent[p.row] + below);
        if deficit > 0.0 {
            let share = deficit / p.row_span as f32;
            for d in row_descent.iter_mut().skip(p.row).take(p.row_span) {
                *d += share;
            }
        }
    }

    let total_width = col_widths.iter().sum::<f32>() + 2.0 * hpad * n_cols as f32;
    let total_height = row_ascent
        .iter()
        .zip(&row_descent)
        .map(|(a, d)| a + d)
        .sum::<f32>()
        + 2.0 * vpad * n_rows as f32;

    // Center the table vertically on the math axis; a table shorter than
    // twice the axis height sits on the baseline instead of dipping below.
    let axis = ctx.constant(ctx.font.constants().axis_height());
    let descent = (total_height / 2.0 - axis).max(0.0);
    let mut out = MathBox {
        width: total_width,
        ascent: total_height - descent,
        descent,
        ..MathBox::empty()
    };

    let align_of = |j: usize| -> ColumnAlign {
        column_align
            .get(j)
            .or(column_align.last())
            .copied()
            .unwrap_or_default()
    };
    // Visual column position: logical column j counts from the right in RTL.
    let col_start = |j: usize| -> f32 {
        let logical: f32 = col_widths[..j].iter().map(|w| w + 2.0 * hpad).sum();
        if ctx.rtl {
            total_width - logical - (col_widths[j] + 2.0 * hpad)
        } else {
            logical
        }
    };
    // Baseline of each row.
    let mut baselines = Vec::with_capacity(n_rows);
    let mut y = -out.ascent;
    for r in 0..n_rows {
        let baseline = y + vpad + row_ascent[r];
        baselines.push(baseline);
        y = baseline + row_descent[r] + vpad;
    }

    for p in placed {
        let last_col = p.col + p.col_span - 1;
        let region_start = if ctx.rtl {
            col_start(last_col)
        } else {
            col_start(p.col)
        };
        let region_width =
            col_widths[p.col..=last_col].iter().sum::<f32>() + 2.0 * hpad * (p.col_span - 1) as f32;
        let slack = region_width - p.content.width;
        let dx = region_start
            + hpad
            + match align_of(p.col) {
                ColumnAlign::Left => 0.0,
                ColumnAlign::Center => slack / 2.0,
                ColumnAlign::Right => slack,
            };
        let baseline = baselines[p.row];
        for mut item in p.content.items {
            item.translate(dx, baseline);
            out.items.push(item);
        }
    }
    out
}

const RADICAL_CHAR: char = '\u{221A}';

/// `<msqrt>`/`<mroot>` per the OpenType MATH radical constants: the radical
/// glyph is stretched (pre-drawn variant or extender assembly) to cover the
/// radicand's height plus the minimum gap and rule; the overbar continues
/// from its top across the radicand, with RadicalExtraAscender white space
/// above.
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
        emit_stretched(ctx, s, 0.0, 0.0, false, &mut probe).0
    });

    // A taller-than-needed glyph centers its excess: half widens the gap,
    // half hangs below the radicand (TeX rule 11).
    let gap = gap_min + (glyph_height - target).max(0.0) / 2.0;
    // Top edge of the overbar; the radical glyph's ink top aligns with it.
    let bar_top = -(content.ascent + gap + thickness);

    let mut out = MathBox::empty();
    out.ascent = content.ascent + gap + thickness + ctx.constant(c.radical_extra_ascender());
    out.descent = content.descent.max(bar_top + glyph_height);

    // Horizontal assembly, mirrored under RTL:
    // LTR: [kern degree kern][surd][radicand];  RTL: [radicand][surd][degree].
    let place_degree = |out: &mut MathBox, deg: MathBox, x: f32| -> f32 {
        // Degree bottom sits this fraction of the glyph's span above its bottom.
        let raise = f32::from(c.radical_degree_bottom_raise_percent()) / 100.0;
        let glyph_bottom = bar_top + glyph_height;
        let deg_baseline = glyph_bottom - raise * glyph_height - deg.descent;
        out.ascent = out.ascent.max(deg.ascent - deg_baseline);
        out.descent = out.descent.max(deg_baseline + deg.descent);
        let width = deg.width;
        for mut item in deg.items {
            item.translate(x, deg_baseline);
            out.items.push(item);
        }
        width
    };
    if ctx.rtl {
        out.items.push(Item::Rule {
            x: 0.0,
            y: bar_top,
            w: content.width,
            h: thickness,
            color: ctx.color,
        });
        for item in content.items {
            out.items.push(item);
        }
        let mut x = content.width;
        if let Some(s) = &stretched {
            let (_, advance) = emit_stretched(ctx, s, x, bar_top, true, &mut out.items);
            x += advance;
        }
        if let Some(deg) = degree {
            x += ctx.constant(c.radical_kern_after_degree()).max(0.0);
            x += place_degree(&mut out, deg, x);
            x += ctx.constant(c.radical_kern_before_degree());
        }
        out.width = x.max(content.width);
    } else {
        let mut x = 0.0;
        if let Some(deg) = degree {
            x += ctx.constant(c.radical_kern_before_degree());
            let w = place_degree(&mut out, deg, x);
            x += w + ctx.constant(c.radical_kern_after_degree());
            x = x.max(0.0); // a large negative kern must not push the glyph out
        }
        if let Some(s) = &stretched {
            let (_, advance) = emit_stretched(ctx, s, x, bar_top, false, &mut out.items);
            x += advance;
        }
        out.items.push(Item::Rule {
            x,
            y: bar_top,
            w: content.width,
            h: thickness,
            color: ctx.color,
        });
        for mut item in content.items {
            item.translate(x, 0.0);
            out.items.push(item);
        }
        out.width = x + content.width;
    }
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
                            let stretched = ctx.font.stretch_horizontal(g, width / ctx.scale);
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
    let under_box = under.map(|n| stretch_h(&under_ctx, n, under_box.expect("laid out above")));

    let c = ctx.font.constants();
    // Operators carrying limits use the limit constants; plain bases the bar
    // constants.
    let limits_base = base_flags & (opdict::LARGEOP | opdict::MOVABLE_LIMITS) != 0;

    let mut out = MathBox::empty();
    out.width = width;
    out.ascent = base_box.ascent;
    out.descent = base_box.descent;
    let base_dx = (width - base_box.width) / 2.0;
    // Where an accent should attach horizontally: the font's per-glyph
    // attachment point when the box is a single glyph, else its center.
    let attach = |b: &MathBox| {
        b.lone_glyph
            .and_then(|g| ctx.font.top_accent_attachment(g))
            .map_or(b.width / 2.0, |v| v * ctx.scale)
    };
    let base_attach = base_dx + attach(&base_box);
    for mut item in base_box.items {
        item.translate(base_dx, 0.0);
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
        // Accents align attachment points; other overscripts center.
        let dx = if accent {
            base_attach - attach(&ob)
        } else {
            (width - ob.width) / 2.0
        };
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

/// Bar-less numerator-over-denominator (`linethickness="0"`), per the
/// OpenType MATH stack constants: preferred shifts, then both parts pushed
/// apart symmetrically until StackGapMin holds.
fn layout_stack(ctx: &Ctx, num_box: MathBox, den_box: MathBox) -> MathBox {
    let c = ctx.font.constants();
    let (mut num_shift, mut den_shift, gap_min) = if ctx.display_style {
        (
            ctx.constant(c.stack_top_display_style_shift_up()),
            ctx.constant(c.stack_bottom_display_style_shift_down()),
            ctx.constant(c.stack_display_style_gap_min()),
        )
    } else {
        (
            ctx.constant(c.stack_top_shift_up()),
            ctx.constant(c.stack_bottom_shift_down()),
            ctx.constant(c.stack_gap_min()),
        )
    };
    let gap = (num_shift - num_box.descent) + (den_shift - den_box.ascent);
    if gap < gap_min {
        let bump = (gap_min - gap) / 2.0;
        num_shift += bump;
        den_shift += bump;
    }

    let width = num_box.width.max(den_box.width);
    let mut out = MathBox {
        width,
        ascent: num_shift + num_box.ascent,
        descent: den_shift + den_box.descent,
        italic_correction: 0.0,
        lone_glyph: None,
        items: Vec::with_capacity(num_box.items.len() + den_box.items.len()),
    };
    for (bx, dy) in [(num_box, -num_shift), (den_box, den_shift)] {
        let dx = (width - bx.width) / 2.0;
        for mut item in bx.items {
            item.translate(dx, dy);
            out.items.push(item);
        }
    }
    out
}

/// `<msub>`/`<msup>`/`<msubsup>` per the OpenType MATH script constants
/// (the TeX Appendix G rules 18a–f recast in font terms). Superscripts
/// attach at the base's full advance; subscripts tuck left by its italic
/// correction.
fn layout_scripts(ctx: &Ctx, base: &Node, sub: Option<&Node>, sup: Option<&Node>) -> MathBox {
    let base_box = layout_operator_base(ctx, base);
    layout_scripts_on(ctx, base_box, sub, sup)
}

/// Combined MathKern cut-in for one script against its base: the base's
/// corner evaluated at the script's near edge, plus the script's corner
/// evaluated at the base's near edge (both heights relative to the
/// respective glyph's baseline). Zero unless both sides are single glyphs
/// with kern data is fine — absent tables contribute nothing.
#[allow(clippy::too_many_arguments)]
fn script_kern(
    ctx: &Ctx,
    base: &MathBox,
    script: &MathBox,
    base_corner: KernCorner,
    script_corner: KernCorner,
    base_height: f32,
    script_height: f32,
) -> f32 {
    let mut kern = 0.0;
    if let Some(g) = base.lone_glyph {
        kern += ctx.font.math_kern(g, base_corner, base_height / ctx.scale) * ctx.scale;
    }
    if let Some(g) = script.lone_glyph {
        // The script glyph was laid out at script scale.
        let script_scale = script.items.first().map_or(ctx.scale, |i| match i {
            Item::Glyph { size, .. } => size / ctx.font.units_per_em(),
            _ => ctx.scale,
        });
        kern += ctx
            .font
            .math_kern(g, script_corner, script_height / script_scale)
            * script_scale;
    }
    kern
}

/// Shared vertical shifts for any number of sub/superscripts on one base
/// (the TeX u/v computation, taken as maxima over all scripts so multiple
/// pairs align on common baselines).
fn script_shifts(ctx: &Ctx, base: &MathBox, subs: &[&MathBox], sups: &[&MathBox]) -> (f32, f32) {
    let c = ctx.font.constants();

    // Superscript shift above the baseline (u in TeX terms).
    let mut sup_shift = 0.0_f32;
    if !sups.is_empty() {
        let preferred = if ctx.cramped {
            ctx.constant(c.superscript_shift_up_cramped())
        } else {
            ctx.constant(c.superscript_shift_up())
        };
        sup_shift = preferred
            // Don't drop the script baseline too far below the base's top.
            .max(base.ascent - ctx.constant(c.superscript_baseline_drop_max()));
        for s in sups {
            // Keep the superscript's bottom ink above SuperscriptBottomMin.
            sup_shift = sup_shift.max(ctx.constant(c.superscript_bottom_min()) + s.descent);
        }
    }

    // Subscript shift below the baseline (v in TeX terms).
    let mut sub_shift = 0.0_f32;
    if !subs.is_empty() {
        sub_shift = ctx
            .constant(c.subscript_shift_down())
            // Hang the script baseline at least this far below the base's bottom.
            .max(base.descent + ctx.constant(c.subscript_baseline_drop_min()));
        for s in subs {
            // Keep the subscript's top ink below SubscriptTopMax.
            sub_shift = sub_shift.max(s.ascent - ctx.constant(c.subscript_top_max()));
        }
    }

    // With both, keep them apart: grow the gap first by raising the
    // superscript (up to SuperscriptBottomMaxWithSubscript), then by pushing
    // the subscript down.
    if !subs.is_empty() && !sups.is_empty() {
        let sup_descent = sups.iter().map(|s| s.descent).fold(f32::MIN, f32::max);
        let sub_ascent = subs.iter().map(|s| s.ascent).fold(f32::MIN, f32::max);
        let gap = (sup_shift - sup_descent) + (sub_shift - sub_ascent);
        let mut deficit = ctx.constant(c.sub_superscript_gap_min()) - gap;
        if deficit > 0.0 {
            let headroom =
                ctx.constant(c.superscript_bottom_max_with_subscript()) - (sup_shift - sup_descent);
            if headroom > 0.0 {
                let up = deficit.min(headroom);
                sup_shift += up;
                deficit -= up;
            }
            sub_shift += deficit.max(0.0);
        }
    }
    (sup_shift, sub_shift)
}

/// `<mmultiscripts>`: prescript columns (scripts right-aligned per column)
/// before the base, then postscript columns after it, all sharing the
/// vertical shifts so script baselines align across columns.
fn layout_multiscripts(
    ctx: &Ctx,
    base: &Node,
    post: &[(Option<Node>, Option<Node>)],
    pre: &[(Option<Node>, Option<Node>)],
) -> MathBox {
    let base_box = layout_operator_base(ctx, base);
    let lay = |pairs: &[(Option<Node>, Option<Node>)]| -> Vec<(Option<MathBox>, Option<MathBox>)> {
        pairs
            .iter()
            .map(|(sub, sup)| {
                (
                    sub.as_ref()
                        .map(|n| layout_node(&ctx.script_child(true), n)),
                    sup.as_ref()
                        .map(|n| layout_node(&ctx.script_child(false), n)),
                )
            })
            .collect()
    };
    let post_boxes = lay(post);
    let pre_boxes = lay(pre);

    let all_subs: Vec<&MathBox> = post_boxes
        .iter()
        .chain(&pre_boxes)
        .filter_map(|(s, _)| s.as_ref())
        .collect();
    let all_sups: Vec<&MathBox> = post_boxes
        .iter()
        .chain(&pre_boxes)
        .filter_map(|(_, s)| s.as_ref())
        .collect();
    let (sup_shift, sub_shift) = script_shifts(ctx, &base_box, &all_subs, &all_sups);

    let mut out = MathBox::empty();
    let mut x = 0.0_f32;
    let place = |out: &mut MathBox, b: MathBox, dx: f32, dy: f32| {
        out.ascent = out.ascent.max(b.ascent - dy);
        out.descent = out.descent.max(b.descent + dy);
        for mut item in b.items {
            item.translate(dx, dy);
            out.items.push(item);
        }
    };

    // Prescript columns: right-aligned within each column.
    for (sub, sup) in pre_boxes {
        let col = sub
            .as_ref()
            .map_or(0.0, |b| b.width)
            .max(sup.as_ref().map_or(0.0, |b| b.width));
        if let Some(b) = sub {
            let dx = x + col - b.width;
            place(&mut out, b, dx, sub_shift);
        }
        if let Some(b) = sup {
            let dx = x + col - b.width;
            place(&mut out, b, dx, -sup_shift);
        }
        x += col;
    }

    let base_width = base_box.width;
    let base_ic = base_box.italic_correction;
    place(&mut out, base_box, x, 0.0);
    x += base_width;

    // Postscript columns: the first column's subscript tucks by the base's
    // italic correction, as in msubsup.
    for (i, (sub, sup)) in post_boxes.into_iter().enumerate() {
        let ic = if i == 0 { base_ic } else { 0.0 };
        let sub_x = (x - ic).max(0.0);
        let mut col_end = x;
        if let Some(b) = sub {
            col_end = col_end.max(sub_x + b.width);
            place(&mut out, b, sub_x, sub_shift);
        }
        if let Some(b) = sup {
            col_end = col_end.max(x + b.width);
            place(&mut out, b, x, -sup_shift);
        }
        x = col_end;
    }
    out.width = x + ctx.constant(ctx.font.constants().space_after_script());
    out
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
    let subs: Vec<&MathBox> = sub_box.iter().collect();
    let sups: Vec<&MathBox> = sup_box.iter().collect();
    let (sup_shift, sub_shift) = script_shifts(ctx, &base_box, &subs, &sups);
    let c = ctx.font.constants();

    // Superscripts attach at the full advance; subscripts tuck left by the
    // base's italic correction (the classic ∫ lower-limit tuck). On top of
    // that, the font's MathKern staircases cut scripts into the base's
    // corner whitespace at the heights where they actually sit.
    let sup_kern = sup_box.as_ref().map_or(0.0, |s| {
        script_kern(
            ctx,
            &base_box,
            s,
            KernCorner::TopRight,
            KernCorner::BottomLeft,
            sup_shift - s.descent,
            base_box.ascent - sup_shift,
        )
    });
    let sub_kern = sub_box.as_ref().map_or(0.0, |s| {
        script_kern(
            ctx,
            &base_box,
            s,
            KernCorner::BottomRight,
            KernCorner::TopLeft,
            s.ascent - sub_shift,
            sub_shift - base_box.descent,
        )
    });
    // In RTL, scripts sit to the left of the base (no kern/italic
    // correction refinement on the mirrored side yet).
    let (base_x, sup_x, sub_x, total_width);
    if ctx.rtl {
        let extent = sub_box
            .as_ref()
            .map_or(0.0, |b| b.width)
            .max(sup_box.as_ref().map_or(0.0, |b| b.width));
        base_x = ctx.constant(c.space_after_script()) + extent;
        sup_x = base_x - sup_box.as_ref().map_or(0.0, |b| b.width);
        sub_x = base_x - sub_box.as_ref().map_or(0.0, |b| b.width);
        total_width = base_x + base_box.width;
    } else {
        base_x = 0.0;
        sup_x = (base_box.width + sup_kern).max(0.0);
        sub_x = (base_box.width - base_box.italic_correction + sub_kern).max(0.0);
        let end = sub_box
            .as_ref()
            .map_or(0.0, |b| sub_x + b.width)
            .max(sup_box.as_ref().map_or(0.0, |b| sup_x + b.width))
            .max(base_box.width);
        total_width = end + ctx.constant(c.space_after_script());
    }

    let mut base_items = base_box.items;
    for item in &mut base_items {
        item.translate(base_x, 0.0);
    }
    let mut out = MathBox {
        width: total_width,
        ascent: base_box.ascent,
        descent: base_box.descent,
        italic_correction: 0.0,
        lone_glyph: None,
        items: base_items,
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
/// rule is centered on the math axis. `linethickness="0"` switches to the
/// bar-less stack layout (binomial coefficients), using the Stack constants.
fn layout_frac(ctx: &Ctx, num: &Node, den: &Node, line_thickness: Option<Length>) -> MathBox {
    let num_box = layout_node(&ctx.frac_child(false), num);
    let den_box = layout_node(&ctx.frac_child(true), den);

    let c = ctx.font.constants();
    let axis = ctx.constant(c.axis_height());
    let default_thickness = ctx.constant(c.fraction_rule_thickness());
    let thickness = line_thickness
        .map(|l| ctx.resolve(l, default_thickness).max(0.0))
        .unwrap_or(default_thickness);
    if thickness == 0.0 {
        return layout_stack(ctx, num_box, den_box);
    }
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
        lone_glyph: None,
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
        color: ctx.color,
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
                && single_char(text)
                    .and_then(|c| ctx.font.glyph_index(c))
                    .is_some();
            if stretchy {
                let symmetric = attrs.symmetric.unwrap_or(flags & opdict::SYMMETRIC != 0);
                (
                    lspace,
                    rspace,
                    Slot::Stretchy {
                        node: child,
                        symmetric,
                    },
                )
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

    // Pass 2: stretch deferred operators to the row's extent, then place —
    // left-to-right, or mirrored from the right edge under dir="rtl".
    let boxes: Vec<(f32, f32, MathBox)> = slots
        .into_iter()
        .map(|(lspace, rspace, slot)| {
            let b = match slot {
                Slot::Fixed(b) => b,
                Slot::Stretchy { node, symmetric } => {
                    layout_embellished_stretchy(ctx, node, symmetric, max_ascent, max_descent)
                }
            };
            (lspace, rspace, b)
        })
        .collect();
    let total: f32 = boxes.iter().map(|(l, r, b)| l + b.width + r).sum();
    let mut out = MathBox::empty();
    out.width = total;
    let mut x = 0.0;
    for (lspace, rspace, mut b) in boxes {
        let pos = if ctx.rtl {
            total - x - lspace - b.width
        } else {
            x + lspace
        };
        for item in &mut b.items {
            item.translate(pos, 0.0);
        }
        out.items.append(&mut b.items);
        out.ascent = out.ascent.max(b.ascent);
        out.descent = out.descent.max(b.descent);
        out.italic_correction = b.italic_correction;
        x += lspace + b.width + rspace;
    }
    out
}

/// Stretch the core of an embellished operator to the row extent, then
/// re-wrap it in its script attachments and style scopes. Under/over
/// embellishments lay out unstretched (they'd need their own re-wrapping
/// pass).
fn layout_embellished_stretchy(
    ctx: &Ctx,
    node: &Node,
    symmetric: bool,
    max_ascent: f32,
    max_descent: f32,
) -> MathBox {
    match node {
        Node::Styled { styles, children } if children.len() == 1 => {
            let inner = ctx.styled_child(styles);
            return decorate(
                styles,
                layout_embellished_stretchy(
                    &inner,
                    &children[0],
                    symmetric,
                    max_ascent,
                    max_descent,
                ),
            );
        }
        _ => {}
    }
    match node {
        Node::Operator { text, .. } => match single_char(text) {
            Some(c) => {
                let Node::Operator { attrs, .. } = node else {
                    unreachable!("matched Operator above")
                };
                layout_stretchy_operator(
                    ctx,
                    c,
                    symmetric,
                    attrs.minsize,
                    attrs.maxsize,
                    max_ascent,
                    max_descent,
                )
            }
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

/// Unicode bidi-mirrored counterpart for RTL rendering of paired
/// delimiters and directional relations (the pairs math actually uses).
fn bidi_mirror(c: char) -> Option<char> {
    const PAIRS: &[(char, char)] = &[
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('⟨', '⟩'),
        ('⌈', '⌉'),
        ('⌊', '⌋'),
        ('<', '>'),
        ('≤', '≥'),
        ('⟦', '⟧'),
        ('∈', '∋'),
        ('∉', '∌'),
        ('⊂', '⊃'),
        ('⊆', '⊇'),
    ];
    PAIRS.iter().find_map(|&(a, b)| {
        if c == a {
            Some(b)
        } else if c == b {
            Some(a)
        } else {
            None
        }
    })
}

/// The character to render for `c` in the current direction.
fn directed_char(ctx: &Ctx, c: char) -> char {
    if ctx.rtl {
        bidi_mirror(c).unwrap_or(c)
    } else {
        c
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
/// returning `(ink height, advance width)` in layout units. `mirror` flips
/// the glyphs horizontally (RTL radicals).
fn emit_stretched(
    ctx: &Ctx,
    stretched: &Stretched,
    x: f32,
    top: f32,
    mirror: bool,
    items: &mut Vec<Item>,
) -> (f32, f32) {
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
                color: ctx.color,
                mirrored: mirror,
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
                let Some(ink) = ctx.font.ink_box(g) else {
                    continue;
                };
                // Part's ink bottom sits `offset` above the assembly bottom.
                items.push(Item::Glyph {
                    id: g,
                    x,
                    y: bottom - offset * ctx.scale + f32::from(ink.y_min) * ctx.scale,
                    size: ctx.size,
                    color: ctx.color,
                    mirrored: mirror,
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
                color: ctx.color,
                mirrored: false,
            });
            out.width = ctx.font.advance(*g) * ctx.scale;
            out.lone_glyph = Some(*g);
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
                    color: ctx.color,
                    mirrored: false,
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
/// of scripts/under-over wrappers, or a style scope / one-element row around
/// an embellished operator (MathML Core's embellished-operator definition,
/// minus the space-like-sibling row case).
fn core_operator(node: &Node) -> Option<(&str, &crate::ast::OperatorAttrs)> {
    match node {
        Node::Operator { text, attrs } => Some((text, attrs)),
        Node::Scripts { base, .. } | Node::UnderOver { base, .. } => core_operator(base),
        Node::Styled { children, .. } | Node::Row(children) if children.len() == 1 => {
            core_operator(&children[0])
        }
        _ => None,
    }
}

/// Lay out a node that may be a lone large operator: in display style the
/// base of scripts/limits picks its DisplayOperatorMinHeight variant.
/// Style scopes around the operator are looked through and re-applied.
fn layout_operator_base(ctx: &Ctx, node: &Node) -> MathBox {
    match node {
        Node::Operator { text, attrs } if ctx.display_style => {
            if let Some(c) = single_char(text) {
                let flags = dictionary_entry(text, Form::Infix).2;
                if attrs.largeop.unwrap_or(flags & opdict::LARGEOP != 0) {
                    return layout_large_operator(ctx, c);
                }
            }
            layout_node(ctx, node)
        }
        Node::Styled { styles, children } if children.len() == 1 => {
            let inner = ctx.styled_child(styles);
            decorate(styles, layout_operator_base(&inner, &children[0]))
        }
        _ => layout_node(ctx, node),
    }
}

/// A vertically-stretchy operator covering the row's extent: symmetric ones
/// (fences) grow equally about the math axis, others cover ascent + descent
/// directly. minsize/maxsize clamp the target (percentages resolve against
/// the unstretched glyph). Excess from a too-tall variant is centered over
/// the target.
fn layout_stretchy_operator(
    ctx: &Ctx,
    c: char,
    symmetric: bool,
    minsize: Option<Length>,
    maxsize: Option<Length>,
    max_ascent: f32,
    max_descent: f32,
) -> MathBox {
    let c = directed_char(ctx, c);
    let glyph = ctx.font.glyph_index(c).expect("checked by caller");
    let axis = ctx.constant(ctx.font.constants().axis_height());
    let (mut target, mut target_ascent) = if symmetric {
        let above = (max_ascent - axis).max(max_descent + axis).max(0.0);
        (2.0 * above, axis + above)
    } else {
        ((max_ascent + max_descent).max(0.0), max_ascent)
    };
    if minsize.is_some() || maxsize.is_some() {
        let natural = ctx
            .font
            .ink_box(glyph)
            .map_or(0.0, |ink| f32::from(ink.y_max - ink.y_min) * ctx.scale);
        let clamped = target
            .max(minsize.map_or(0.0, |l| ctx.resolve(l, natural)))
            .min(maxsize.map_or(f32::INFINITY, |l| ctx.resolve(l, natural)))
            .max(0.0);
        if symmetric {
            target_ascent = axis + clamped / 2.0;
        } else {
            // Distribute the size change evenly about the covered range.
            target_ascent += (clamped - target) / 2.0;
        }
        target = clamped;
    }
    if target <= 0.0 {
        // Nothing to cover (row of only stretchy operators): natural glyph.
        return layout_text_run(ctx, &c.to_string());
    }
    let stretched = ctx.font.stretch_vertical(glyph, target / ctx.scale);
    let mut out = MathBox::empty();
    // Probe the actual ink height first to center any excess.
    let mut probe = Vec::new();
    let (height, _) = emit_stretched(ctx, &stretched, 0.0, 0.0, false, &mut probe);
    let ascent = target_ascent + (height - target).max(0.0) / 2.0;
    let (height, advance) = emit_stretched(ctx, &stretched, 0.0, -ascent, false, &mut out.items);
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
    let (height, _) = emit_stretched(ctx, &stretched, 0.0, 0.0, false, &mut probe);
    let ascent = axis + height / 2.0;
    let mut out = MathBox::empty();
    let (height, advance) = emit_stretched(ctx, &stretched, 0.0, -ascent, false, &mut out.items);
    out.width = advance;
    out.ascent = ascent;
    out.descent = height - ascent;
    if let Stretched::Glyph(g) = stretched {
        out.italic_correction = ctx.font.italic_correction(g) * ctx.scale;
        out.lone_glyph = Some(g);
    }
    out
}

/// Elements that don't count as operands when inferring operator forms
/// (MathML Core's space-like definition).
fn is_space_like(node: &Node) -> bool {
    match node {
        Node::Space { .. } | Node::Text(_) => true,
        Node::Row(children) | Node::Phantom(children) => children.iter().all(is_space_like),
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

/// A run of token text: shaped with rustybuzz when the `shaping` feature is
/// on (kerning, `ssty` script alternates), otherwise per-character cmap
/// lookup + advances — adequate for isolated math glyphs, which don't form
/// clusters or ligate.
fn layout_text_run(ctx: &Ctx, text: &str) -> MathBox {
    #[cfg(feature = "shaping")]
    if let Some(b) = shape_text_run(ctx, text) {
        return b;
    }
    let mut out = MathBox::empty();
    for c in text.chars() {
        let c = directed_char(ctx, c);
        let Some(gid) = ctx.font.glyph_index(c) else {
            // No .notdef rendering yet: skip unmapped characters.
            continue;
        };
        let gid = ctx.script_glyph(gid);
        out.items.push(Item::Glyph {
            id: gid,
            x: out.width,
            y: 0.0,
            size: ctx.size,
            color: ctx.color,
            mirrored: false,
        });
        out.width += ctx.font.advance(gid) * ctx.scale;
        out.italic_correction = ctx.font.italic_correction(gid) * ctx.scale;
        if let Some(ink) = ctx.font.ink_box(gid) {
            out.ascent = out.ascent.max(ink.y_max as f32 * ctx.scale);
            out.descent = out.descent.max(-(ink.y_min as f32) * ctx.scale);
        }
    }
    finish_text_run(ctx, out)
}

/// Shape a run with rustybuzz. `ssty` is enabled in script styles so fonts
/// can swap in script-tuned alternates (primes being the classic case).
#[cfg(feature = "shaping")]
fn shape_text_run(ctx: &Ctx, text: &str) -> Option<MathBox> {
    let shaper = ctx.font.shaper()?;
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    let mirrored: String;
    let text = if ctx.rtl {
        mirrored = text.chars().map(|c| directed_char(ctx, c)).collect();
        &mirrored
    } else {
        text
    };
    buffer.push_str(text);
    buffer.set_direction(rustybuzz::Direction::LeftToRight);
    // Select the OpenType `math` script: math fonts register ssty/dtls there,
    // and Common-script characters would otherwise resolve to DFLT.
    buffer.set_script(rustybuzz::script::SCRIPT_MATH);
    let mut features = Vec::new();
    if ctx.script_level > 0 {
        features.push(rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(b"ssty"),
            u32::from(ctx.script_level.min(2)),
            ..,
        ));
    }
    let shaped = rustybuzz::shape(shaper, &features, buffer);

    let mut out = MathBox::empty();
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        let x = out.width + pos.x_offset as f32 * ctx.scale;
        // HarfBuzz y offsets are y-up; the display list is y-down.
        let y = -(pos.y_offset as f32) * ctx.scale;
        if info.glyph_id == 0 {
            out.width += pos.x_advance as f32 * ctx.scale;
            continue; // unmapped: advance but draw nothing, like the fallback
        }
        let shaped_gid = GlyphId(info.glyph_id as u16);
        let gid = ctx.script_glyph(shaped_gid);
        // A substituted alternate has its own advance; the shaped advance
        // belongs to the glyph it replaced.
        out.width += if gid == shaped_gid {
            pos.x_advance as f32 * ctx.scale
        } else {
            ctx.font.advance(gid) * ctx.scale
        };
        out.items.push(Item::Glyph {
            id: gid,
            x,
            y,
            size: ctx.size,
            color: ctx.color,
            mirrored: false,
        });
        out.italic_correction = ctx.font.italic_correction(gid) * ctx.scale;
        if let Some(ink) = ctx.font.ink_box(gid) {
            out.ascent = out.ascent.max(f32::from(ink.y_max) * ctx.scale - y);
            out.descent = out.descent.max(y - f32::from(ink.y_min) * ctx.scale);
        }
    }
    Some(finish_text_run(ctx, out))
}

/// A run with no ink (all spaces) still occupies the line: fall back to
/// font-wide metrics so an empty-looking box doesn't collapse vertically.
fn finish_text_run(ctx: &Ctx, mut out: MathBox) -> MathBox {
    if !out.items.is_empty() && out.ascent == 0.0 && out.descent == 0.0 {
        let (asc, desc) = ctx.font.line_metrics();
        out.ascent = asc * ctx.scale;
        out.descent = desc * ctx.scale;
    }
    if let [Item::Glyph { id, .. }] = out.items.as_slice() {
        out.lone_glyph = Some(*id);
    }
    out
}
