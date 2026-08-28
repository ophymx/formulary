//! Typed MathML element tree.
//!
//! The parser produces this tree from presentation MathML; layout consumes
//! it. Elements without a variant here (e.g. `menclose`) are handled by the
//! parser's error recovery rather than the tree.

/// Whether the formula is laid out for its own line or inline with text.
///
/// Maps to the `display` attribute on `<math>`: `block` → [`DisplayMode::Block`],
/// anything else (including absent) → [`DisplayMode::Inline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    #[default]
    Inline,
    Block,
}

/// A MathML length value (MathML Core "length-percentage" subset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// Relative to the current font size (which shrinks in scripts).
    Em(f32),
    /// Relative to the font's x-height at the current size.
    Ex(f32),
    /// CSS pixels. Resolved 1:1 into layout units, i.e. the caller's
    /// `font_size` is assumed to be in CSS px.
    Px(f32),
    /// CSS points (`1pt = 96/72 px`).
    Pt(f32),
    /// Percentage of a reference dimension; only meaningful where the spec
    /// defines one (e.g. `mpadded` uses the natural size).
    Percent(f32),
}

/// Layout direction, from the `dir` global attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

/// An sRGB color with alpha, as parsed from `mathcolor`/`mathbackground`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }
}

/// Style attributes valid on every MathML element, captured by a
/// [`Node::Styled`] wrapper around the element they appeared on.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleOverrides {
    pub display_style: Option<bool>,
    pub script_level: Option<ScriptLevel>,
    /// `mathsize`: new font size for the subtree (em/% relative to the
    /// inherited size).
    pub math_size: Option<Length>,
    /// `mathcolor`: ink color for the subtree.
    pub color: Option<Color>,
    /// `mathbackground`: painted behind the subtree's box.
    pub background: Option<Color>,
    /// Not a MathML attribute: a 1px border in this color around the box,
    /// used for `merror`'s user-agent styling.
    pub border: Option<Color>,
    /// `dir`: layout direction for the subtree.
    pub dir: Option<Direction>,
}

impl StyleOverrides {
    pub fn is_empty(&self) -> bool {
        *self == StyleOverrides::default()
    }
}

/// One `<mtd>`: its children as an implied `mrow`, plus grid spans.
#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub content: Node,
    /// `rowspan`, clamped to at least 1 (and to the table's extent at
    /// layout time).
    pub row_span: u32,
    /// `columnspan`, clamped to at least 1.
    pub col_span: u32,
}

impl TableCell {
    pub fn new(content: Node) -> Self {
        TableCell {
            content,
            row_span: 1,
            col_span: 1,
        }
    }
}

/// Horizontal alignment of cells within a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// An operator's syntactic position, which selects its operator-dictionary
/// entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Infix,
    Prefix,
    Postfix,
}

/// Attributes of `<mo>` that override the operator dictionary. `None` means
/// "use the dictionary value".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OperatorAttrs {
    pub form: Option<Form>,
    pub lspace: Option<Length>,
    pub rspace: Option<Length>,
    pub stretchy: Option<bool>,
    pub symmetric: Option<bool>,
    pub largeop: Option<bool>,
    pub movablelimits: Option<bool>,
    /// Clamp on the stretched size. Percentages resolve against the
    /// unstretched glyph's size.
    pub minsize: Option<Length>,
    pub maxsize: Option<Length>,
}

/// A `scriptlevel` attribute value: absolute, or relative with an explicit
/// `+`/`-` sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptLevel {
    Set(u8),
    Add(i8),
}

/// A parsed `<math>` element.
#[derive(Debug, Clone, PartialEq)]
pub struct MathRoot {
    pub display: DisplayMode,
    /// The `displaystyle` attribute, overriding the default implied by
    /// `display` (block → true, inline → false).
    pub displaystyle: Option<bool>,
    /// The children of `<math>`, treated as an anonymous `mrow`.
    pub children: Vec<Node>,
    /// Recoveries applied during parsing (MathML Core's error handling lays
    /// out unknown or structurally invalid elements as `mrow`). Empty means
    /// the markup was fully understood; non-empty markup still renders.
    pub warnings: Vec<Warning>,
}

/// A recovered-from problem in the source markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// An element this crate doesn't know; rendered as an `mrow` of its
    /// children (or as text, if it only contains text).
    UnknownElement { element: String },
    /// A known element whose children don't form the required structure
    /// (wrong arity, stray table children, odd script counts); rendered via
    /// the spec's `mrow` fallback.
    InvalidStructure { element: String, detail: String },
}

/// One presentation MathML element.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// `<mi>` — identifier. Single characters default to math-italic per
    /// MathML Core; the parser applies that mapping (and any `mathvariant`),
    /// so the string holds final code points.
    Identifier(String),
    /// `<mn>` — numeric literal.
    Number(String),
    /// `<mo>` — operator. Spacing and properties come from the MathML Core
    /// operator dictionary keyed by (character, form), overridable by the
    /// attributes captured here.
    Operator {
        text: String,
        attrs: OperatorAttrs,
    },
    /// `<mtext>` — literal text.
    Text(String),
    /// `<mrow>` — horizontal grouping.
    Row(Vec<Node>),
    /// `<mfrac>` — numerator over denominator. Exactly two children.
    /// `line_thickness` overrides the font's fraction rule thickness; zero
    /// yields a bar-less stack (the binomial-coefficient idiom).
    Frac {
        num: Box<Node>,
        den: Box<Node>,
        line_thickness: Option<Length>,
    },
    /// `<msub>`, `<msup>`, or `<msubsup>`, normalized to one shape: a base
    /// with an optional subscript and/or superscript (at least one present).
    Scripts {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    /// `<mmultiscripts>` — a base with any number of (sub, sup) script pairs
    /// after it and, following `<mprescripts/>`, before it. `None` slots come
    /// from `<none/>` placeholders.
    MultiScripts {
        base: Box<Node>,
        post: Vec<(Option<Node>, Option<Node>)>,
        pre: Vec<(Option<Node>, Option<Node>)>,
    },
    /// `<msqrt>` — children form an implied `mrow` under the radical.
    Sqrt(Vec<Node>),
    /// `<mroot>` — radicand plus explicit degree. Exactly two children.
    Root { base: Box<Node>, index: Box<Node> },
    /// `<mspace>` — fixed blank space. Unset attributes default to zero.
    Space {
        width: Option<Length>,
        height: Option<Length>,
        depth: Option<Length>,
    },
    /// A style scope: `<mstyle>`, or any element carrying global style
    /// attributes (`displaystyle`, `scriptlevel`, `mathsize`, `mathcolor`,
    /// `mathbackground`), which parse into this wrapper around the element.
    Styled {
        styles: StyleOverrides,
        children: Vec<Node>,
    },
    /// `<mphantom>` — occupies its children's space, draws nothing.
    Phantom(Vec<Node>),
    /// `<munder>`, `<mover>`, or `<munderover>`, normalized like [`Node::Scripts`].
    UnderOver {
        base: Box<Node>,
        under: Option<Box<Node>>,
        over: Option<Box<Node>>,
        /// The `accent` attribute (tightens the overscript and keeps its size).
        accent: Option<bool>,
        /// The `accentunder` attribute.
        accent_under: Option<bool>,
    },
    /// `<mtable>` — rows of `<mtr>` containing `<mtd>` cells. Rows may be
    /// ragged (missing trailing cells render empty).
    Table {
        rows: Vec<Vec<TableCell>>,
        /// Per-column alignment from `columnalign`; the last entry repeats
        /// for further columns. Empty means center.
        column_align: Vec<ColumnAlign>,
    },
    /// `<mpadded>` — overrides the reported box of its content and can shift
    /// the content within it. `None` keeps the natural value.
    Padded {
        width: Option<Length>,
        height: Option<Length>,
        depth: Option<Length>,
        lspace: Option<Length>,
        voffset: Option<Length>,
        children: Vec<Node>,
    },
}
