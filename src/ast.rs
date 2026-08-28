//! Typed MathML element tree.
//!
//! The parser produces this tree from presentation MathML; layout consumes it.
//! Only Tier-0 token elements and `mrow` exist so far; further variants are
//! added as their layout is implemented.

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
    /// The children of `<math>`, treated as an anonymous `mrow`.
    pub children: Vec<Node>,
}

/// One presentation MathML element.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// `<mi>` — identifier. Single characters default to math-italic per
    /// MathML Core; the mapping happens at layout time.
    Identifier(String),
    /// `<mn>` — numeric literal.
    Number(String),
    /// `<mo>` — operator. Spacing/stretching (operator dictionary) is Tier 1;
    /// for now it lays out like text.
    Operator(String),
    /// `<mtext>` — literal text.
    Text(String),
    /// `<mrow>` — horizontal grouping.
    Row(Vec<Node>),
    /// `<mfrac>` — numerator over denominator. Exactly two children.
    Frac { num: Box<Node>, den: Box<Node> },
    /// `<msub>`, `<msup>`, or `<msubsup>`, normalized to one shape: a base
    /// with an optional subscript and/or superscript (at least one present).
    Scripts {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
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
    /// `<mstyle>` — style container. Only `displaystyle` and `scriptlevel`
    /// affect layout in MathML Core.
    Styled {
        display_style: Option<bool>,
        script_level: Option<ScriptLevel>,
        children: Vec<Node>,
    },
    /// `<mphantom>` — occupies its children's space, draws nothing.
    Phantom(Vec<Node>),
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
