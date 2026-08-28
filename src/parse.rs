//! Presentation MathML → typed element tree.
//!
//! Parsing is lenient, per MathML Core's error handling: unknown elements
//! and structurally invalid markup lay out as `mrow` fallbacks, recorded as
//! [`Warning`]s on the returned [`MathRoot`]. Only malformed XML and a
//! non-`<math>` root are hard errors.

use crate::ast::{
    Color, ColumnAlign, Direction, DisplayMode, Form, Length, MathRoot, Node, OperatorAttrs,
    ScriptLevel, StyleOverrides, TableCell, Warning,
};
use crate::mathvariant::{apply_variant, to_math_italic, MathVariant};

/// Errors that prevent producing a tree at all. Everything else is recovered
/// from and reported via [`MathRoot::warnings`].
#[derive(Debug)]
pub enum ParseError {
    /// The input is not well-formed XML.
    Xml(roxmltree::Error),
    /// The root element is not `<math>`.
    NotMath { found: String },
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Xml(e) => write!(f, "malformed XML: {e}"),
            ParseError::NotMath { found } => {
                write!(f, "expected <math> root element, found <{found}>")
            }
        }
    }
}

impl std::error::Error for ParseError {}

impl From<roxmltree::Error> for ParseError {
    fn from(e: roxmltree::Error) -> Self {
        ParseError::Xml(e)
    }
}

/// Parse a MathML fragment whose root element is `<math>`.
pub fn parse(source: &str) -> Result<MathRoot, ParseError> {
    let doc = roxmltree::Document::parse(source)?;
    let root = doc.root_element();
    if root.tag_name().name() != "math" {
        return Err(ParseError::NotMath {
            found: root.tag_name().name().to_string(),
        });
    }
    let display = match root.attribute("display") {
        Some("block") => DisplayMode::Block,
        _ => DisplayMode::Inline,
    };
    let mut warnings = Vec::new();
    let mut children = parse_children(root, &mut warnings);
    // Style attributes on <math> itself scope over all its children.
    // displaystyle is handled via MathRoot, not the wrapper.
    let mut styles = style_overrides(root);
    styles.display_style = None;
    if !styles.is_empty() {
        children = vec![Node::Styled { styles, children }];
    }
    Ok(MathRoot {
        display,
        displaystyle: bool_attr(root, "displaystyle"),
        children,
        warnings,
    })
}

fn parse_children(parent: roxmltree::Node, warnings: &mut Vec<Warning>) -> Vec<Node> {
    parent
        .children()
        .filter(|c| c.is_element())
        .map(|c| parse_node(c, warnings))
        .collect()
}

/// The spec's recovery for invalid markup: the element renders as an `mrow`
/// of whatever children it has.
fn invalid(
    node: roxmltree::Node,
    element: &'static str,
    detail: String,
    warnings: &mut Vec<Warning>,
) -> Node {
    warnings.push(Warning::InvalidStructure {
        element: element.to_string(),
        detail,
    });
    Node::Row(parse_children(node, warnings))
}

/// Parse one element, wrapping it in a style scope if it carries any of the
/// global style attributes.
fn parse_node(node: roxmltree::Node, warnings: &mut Vec<Warning>) -> Node {
    let parsed = parse_element(node, warnings);
    let styles = style_overrides(node);
    if styles.is_empty() {
        parsed
    } else {
        Node::Styled {
            styles,
            children: vec![parsed],
        }
    }
}

/// The global style attributes present on an element.
fn style_overrides(node: roxmltree::Node) -> StyleOverrides {
    StyleOverrides {
        display_style: bool_attr(node, "displaystyle"),
        script_level: node.attribute("scriptlevel").and_then(parse_script_level),
        math_size: length_attr(node, "mathsize"),
        color: color_attr(node, "mathcolor"),
        background: color_attr(node, "mathbackground"),
        border: None,
        dir: match node.attribute("dir") {
            Some(v) if v.eq_ignore_ascii_case("rtl") => Some(Direction::Rtl),
            Some(v) if v.eq_ignore_ascii_case("ltr") => Some(Direction::Ltr),
            _ => None,
        },
    }
}

fn parse_element(node: roxmltree::Node, warnings: &mut Vec<Warning>) -> Node {
    let name = node.tag_name().name();
    match name {
        "mi" => {
            let text = text_content(node);
            let styled = match variant_attr(node) {
                Some(v) => apply(&text, v),
                // Single-char <mi> defaults to math italic.
                None if text.chars().count() == 1 => text.chars().map(to_math_italic).collect(),
                None => text,
            };
            Node::Identifier(styled)
        }
        "mn" => Node::Number(styled_text(node)),
        // `<ms>` renders as text wrapped in its quote characters.
        "ms" => {
            let lquote = node.attribute("lquote").unwrap_or("\"");
            let rquote = node.attribute("rquote").unwrap_or("\"");
            Node::Text(format!("{lquote}{}{rquote}", styled_text(node)))
        }
        // `<semantics>` and legacy `<maction>` render their first child;
        // annotations and alternate actions are ignored.
        "semantics" | "maction" => match node.children().find(|c| c.is_element()) {
            Some(first) => parse_node(first, warnings),
            None => Node::Row(Vec::new()),
        },
        // `<merror>` renders its contents with the Core user-agent styling:
        // a red border over a light-yellow background.
        "merror" => Node::Styled {
            styles: StyleOverrides {
                background: Some(Color::rgb(0xFF, 0xFF, 0xE0)),
                border: Some(Color::rgb(0xFF, 0, 0)),
                ..StyleOverrides::default()
            },
            children: parse_children(node, warnings),
        },
        // Deprecated `<mfenced>` desugars to its equivalent mrow: open fence,
        // children joined by separators (last one repeating), close fence.
        "mfenced" => {
            let mo = |text: &str| Node::Operator {
                text: text.to_string(),
                attrs: OperatorAttrs::default(),
            };
            let open = node.attribute("open").unwrap_or("(").trim();
            let close = node.attribute("close").unwrap_or(")").trim();
            let separators: Vec<char> = node
                .attribute("separators")
                .unwrap_or(",")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let mut row = Vec::new();
            if !open.is_empty() {
                row.push(mo(open));
            }
            for (i, child) in parse_children(node, warnings).into_iter().enumerate() {
                if i > 0 {
                    if let Some(sep) = separators.get(i - 1).or(separators.last()) {
                        row.push(mo(&sep.to_string()));
                    }
                }
                row.push(child);
            }
            if !close.is_empty() {
                row.push(mo(close));
            }
            Node::Row(row)
        }
        "mo" => Node::Operator {
            text: text_content(node),
            attrs: OperatorAttrs {
                form: match node.attribute("form") {
                    Some("infix") => Some(Form::Infix),
                    Some("prefix") => Some(Form::Prefix),
                    Some("postfix") => Some(Form::Postfix),
                    _ => None,
                },
                lspace: length_attr(node, "lspace"),
                rspace: length_attr(node, "rspace"),
                stretchy: bool_attr(node, "stretchy"),
                symmetric: bool_attr(node, "symmetric"),
                largeop: bool_attr(node, "largeop"),
                movablelimits: bool_attr(node, "movablelimits"),
                minsize: length_attr(node, "minsize"),
                maxsize: length_attr(node, "maxsize"),
            },
        },
        "mtext" => Node::Text(styled_text(node)),
        "mrow" => Node::Row(parse_children(node, warnings)),
        "mfrac" => {
            let mut children = parse_children(node, warnings);
            if children.len() != 2 {
                return invalid_parsed(name, children, warnings);
            }
            let den = Box::new(children.pop().expect("len checked"));
            let num = Box::new(children.pop().expect("len checked"));
            Node::Frac {
                num,
                den,
                line_thickness: length_attr(node, "linethickness"),
            }
        }
        "msub" | "msup" | "msubsup" => {
            let expected = if name == "msubsup" { 3 } else { 2 };
            let mut children = parse_children(node, warnings);
            if children.len() != expected {
                return invalid_parsed(name, children, warnings);
            }
            let mut rest = children.split_off(1);
            let base = Box::new(children.pop().expect("len checked"));
            let (sub, sup) = match name {
                "msub" => (Some(Box::new(rest.remove(0))), None),
                "msup" => (None, Some(Box::new(rest.remove(0)))),
                _ => (
                    Some(Box::new(rest.remove(0))),
                    Some(Box::new(rest.remove(0))),
                ),
            };
            Node::Scripts { base, sub, sup }
        }
        "munder" | "mover" | "munderover" => {
            let expected = if name == "munderover" { 3 } else { 2 };
            let mut children = parse_children(node, warnings);
            if children.len() != expected {
                return invalid_parsed(name, children, warnings);
            }
            let mut rest = children.split_off(1);
            let base = Box::new(children.pop().expect("len checked"));
            let (under, over) = match name {
                "munder" => (Some(Box::new(rest.remove(0))), None),
                "mover" => (None, Some(Box::new(rest.remove(0)))),
                _ => (
                    Some(Box::new(rest.remove(0))),
                    Some(Box::new(rest.remove(0))),
                ),
            };
            Node::UnderOver {
                base,
                under,
                over,
                accent: bool_attr(node, "accent"),
                accent_under: bool_attr(node, "accentunder"),
            }
        }
        "mmultiscripts" => parse_multiscripts(node, warnings),
        "mtable" => {
            let rows = node
                .children()
                .filter(|c| c.is_element())
                .map(|row| {
                    if row.tag_name().name() != "mtr" {
                        // Anonymous fixup, as browsers do: a stray child
                        // becomes a one-cell row.
                        warnings.push(Warning::InvalidStructure {
                            element: "mtable".to_string(),
                            detail: format!("child <{}> is not <mtr>", row.tag_name().name()),
                        });
                        return vec![TableCell::new(parse_node(row, warnings))];
                    }
                    row.children()
                        .filter(|c| c.is_element())
                        .map(|cell| {
                            if cell.tag_name().name() != "mtd" {
                                warnings.push(Warning::InvalidStructure {
                                    element: "mtr".to_string(),
                                    detail: format!(
                                        "child <{}> is not <mtd>",
                                        cell.tag_name().name()
                                    ),
                                });
                                return TableCell::new(parse_node(cell, warnings));
                            }
                            // Invalid span values behave like an absent
                            // attribute (span 1); spans are capped like
                            // HTML's.
                            let span = |name: &str, cap: u32| {
                                cell.attribute(name)
                                    .and_then(|v| v.trim().parse::<u32>().ok())
                                    .filter(|&v| v >= 1)
                                    .unwrap_or(1)
                                    .min(cap)
                            };
                            TableCell {
                                content: Node::Row(parse_children(cell, warnings)),
                                row_span: span("rowspan", 65_534),
                                col_span: span("columnspan", 1_000),
                            }
                        })
                        .collect()
                })
                .collect();
            let column_align = node
                .attribute("columnalign")
                .map(|s| {
                    s.split_whitespace()
                        .map(|w| match w {
                            "left" => ColumnAlign::Left,
                            "right" => ColumnAlign::Right,
                            _ => ColumnAlign::Center,
                        })
                        .collect()
                })
                .unwrap_or_default();
            Node::Table { rows, column_align }
        }
        "msqrt" => Node::Sqrt(parse_children(node, warnings)),
        "mspace" => Node::Space {
            width: length_attr(node, "width"),
            height: length_attr(node, "height"),
            depth: length_attr(node, "depth"),
        },
        // <mstyle> is a plain group; its style attributes are picked up by
        // the universal wrapper in parse_node like on any other element.
        "mstyle" => Node::Row(parse_children(node, warnings)),
        "mphantom" => Node::Phantom(parse_children(node, warnings)),
        "mpadded" => Node::Padded {
            width: length_attr(node, "width"),
            height: length_attr(node, "height"),
            depth: length_attr(node, "depth"),
            lspace: length_attr(node, "lspace"),
            voffset: length_attr(node, "voffset"),
            children: parse_children(node, warnings),
        },
        "mroot" => {
            let mut children = parse_children(node, warnings);
            if children.len() != 2 {
                return invalid_parsed(name, children, warnings);
            }
            let index = Box::new(children.pop().expect("len checked"));
            let base = Box::new(children.pop().expect("len checked"));
            Node::Root { base, index }
        }
        // `<none/>` and `<mprescripts/>` outside mmultiscripts render as
        // nothing.
        "none" | "mprescripts" => {
            warnings.push(Warning::InvalidStructure {
                element: name.to_string(),
                detail: "only valid inside <mmultiscripts>".to_string(),
            });
            Node::Row(Vec::new())
        }
        other => {
            warnings.push(Warning::UnknownElement {
                element: other.to_string(),
            });
            // Core lays out unknown elements as mrow. An unknown element
            // holding only text (stray HTML like <b>x</b>) keeps its text.
            if node.children().any(|c| c.is_element()) {
                Node::Row(parse_children(node, warnings))
            } else {
                Node::Text(text_content(node))
            }
        }
    }
}

/// `mrow` fallback for a fixed-arity element whose children are already
/// parsed.
fn invalid_parsed(element: &str, children: Vec<Node>, warnings: &mut Vec<Warning>) -> Node {
    warnings.push(Warning::InvalidStructure {
        element: element.to_string(),
        detail: format!("wrong number of children ({})", children.len()),
    });
    Node::Row(children)
}

fn parse_multiscripts(node: roxmltree::Node, warnings: &mut Vec<Warning>) -> Node {
    let mut elements = node.children().filter(|c| c.is_element());
    let base = match elements.next() {
        Some(b) if !matches!(b.tag_name().name(), "none" | "mprescripts") => {
            Box::new(parse_node(b, warnings))
        }
        _ => return invalid(node, "mmultiscripts", "missing base".to_string(), warnings),
    };
    // Flat script slots, split at <mprescripts/>; <none/> is an empty slot.
    let mut sections: [Vec<Option<Node>>; 2] = [Vec::new(), Vec::new()];
    let mut section = 0;
    for child in elements {
        match child.tag_name().name() {
            "mprescripts" => {
                if section == 1 {
                    return invalid(
                        node,
                        "mmultiscripts",
                        "more than one <mprescripts/>".to_string(),
                        warnings,
                    );
                }
                section = 1;
            }
            "none" => sections[section].push(None),
            _ => sections[section].push(Some(parse_node(child, warnings))),
        }
    }
    if sections.iter().any(|s| !s.len().is_multiple_of(2)) {
        return invalid(
            node,
            "mmultiscripts",
            "scripts must come in sub/sup pairs".to_string(),
            warnings,
        );
    }
    let pair_up = |slots: Vec<Option<Node>>| {
        let mut pairs = Vec::with_capacity(slots.len() / 2);
        let mut it = slots.into_iter();
        while let (Some(sub), Some(sup)) = (it.next(), it.next()) {
            pairs.push((sub, sup));
        }
        pairs
    };
    let [post_slots, pre_slots] = sections;
    Node::MultiScripts {
        base,
        post: pair_up(post_slots),
        pre: pair_up(pre_slots),
    }
}

/// A length-valued attribute. Invalid values behave like an absent attribute,
/// per MathML's error-recovery convention — markup in the wild is messy and a
/// bad `width` shouldn't kill the whole formula.
fn length_attr(node: roxmltree::Node, name: &str) -> Option<Length> {
    node.attribute(name).and_then(parse_length)
}

/// The `mathvariant` attribute, when present and valid.
fn variant_attr(node: roxmltree::Node) -> Option<MathVariant> {
    node.attribute("mathvariant")
        .and_then(MathVariant::from_attr)
}

fn apply(text: &str, v: MathVariant) -> String {
    text.chars().map(|c| apply_variant(c, v)).collect()
}

/// Token text with any `mathvariant` styling applied.
fn styled_text(node: roxmltree::Node) -> String {
    let text = text_content(node);
    match variant_attr(node) {
        Some(v) => apply(&text, v),
        None => text,
    }
}

fn color_attr(node: roxmltree::Node, name: &str) -> Option<Color> {
    node.attribute(name).and_then(parse_color)
}

/// CSS color subset: #rgb/#rgba/#rrggbb/#rrggbbaa hex forms and common named
/// colors. Invalid values behave like an absent attribute.
fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let d = |i: usize| {
            hex.as_bytes()
                .get(i)
                .and_then(|b| (*b as char).to_digit(16))
        };
        return match hex.len() {
            3 | 4 => {
                let mut c = [0u8; 4];
                for (i, v) in c.iter_mut().enumerate().take(hex.len()) {
                    *v = (d(i)? * 17) as u8;
                }
                if hex.len() == 3 {
                    c[3] = 255;
                }
                Some(Color {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                    a: c[3],
                })
            }
            6 | 8 => {
                let mut c = [0u8; 4];
                for (i, v) in c.iter_mut().enumerate().take(hex.len() / 2) {
                    *v = (d(2 * i)? * 16 + d(2 * i + 1)?) as u8;
                }
                if hex.len() == 6 {
                    c[3] = 255;
                }
                Some(Color {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                    a: c[3],
                })
            }
            _ => None,
        };
    }
    let named = match s.to_ascii_lowercase().as_str() {
        "black" => (0x00, 0x00, 0x00),
        "silver" => (0xC0, 0xC0, 0xC0),
        "gray" | "grey" => (0x80, 0x80, 0x80),
        "white" => (0xFF, 0xFF, 0xFF),
        "maroon" => (0x80, 0x00, 0x00),
        "red" => (0xFF, 0x00, 0x00),
        "purple" => (0x80, 0x00, 0x80),
        "fuchsia" | "magenta" => (0xFF, 0x00, 0xFF),
        "green" => (0x00, 0x80, 0x00),
        "lime" => (0x00, 0xFF, 0x00),
        "olive" => (0x80, 0x80, 0x00),
        "yellow" => (0xFF, 0xFF, 0x00),
        "navy" => (0x00, 0x00, 0x80),
        "blue" => (0x00, 0x00, 0xFF),
        "teal" => (0x00, 0x80, 0x80),
        "aqua" | "cyan" => (0x00, 0xFF, 0xFF),
        "orange" => (0xFF, 0xA5, 0x00),
        "brown" => (0xA5, 0x2A, 0x2A),
        "pink" => (0xFF, 0xC0, 0xCB),
        "gold" => (0xFF, 0xD7, 0x00),
        "violet" => (0xEE, 0x82, 0xEE),
        "indigo" => (0x4B, 0x00, 0x82),
        "lightyellow" => (0xFF, 0xFF, 0xE0),
        "lightgray" | "lightgrey" => (0xD3, 0xD3, 0xD3),
        "darkgray" | "darkgrey" => (0xA9, 0xA9, 0xA9),
        "darkred" => (0x8B, 0x00, 0x00),
        "darkgreen" => (0x00, 0x64, 0x00),
        "darkblue" => (0x00, 0x00, 0x8B),
        "transparent" => {
            return Some(Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            })
        }
        _ => return None,
    };
    Some(Color::rgb(named.0, named.1, named.2))
}

/// Boolean attribute values are ASCII case-insensitive in MathML Core.
fn bool_attr(node: roxmltree::Node, name: &str) -> Option<bool> {
    match node.attribute(name) {
        Some(v) if v.eq_ignore_ascii_case("true") => Some(true),
        Some(v) if v.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn parse_length(s: &str) -> Option<Length> {
    let s = s.trim();
    let split = s
        .find(|c: char| c != '+' && c != '-' && c != '.' && !c.is_ascii_digit())
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let value: f32 = num.parse().ok()?;
    match unit.trim() {
        "em" => Some(Length::Em(value)),
        "ex" => Some(Length::Ex(value)),
        "px" => Some(Length::Px(value)),
        "pt" => Some(Length::Pt(value)),
        "%" => Some(Length::Percent(value)),
        // Unitless nonzero numbers are invalid in MathML Core.
        "" if value == 0.0 => Some(Length::Px(0.0)),
        _ => None,
    }
}

fn parse_script_level(s: &str) -> Option<ScriptLevel> {
    let s = s.trim();
    if s.starts_with('+') || s.starts_with('-') {
        s.parse().ok().map(ScriptLevel::Add)
    } else {
        s.parse().ok().map(ScriptLevel::Set)
    }
}

/// Concatenated text of a token element, with the whitespace trimming MathML
/// applies to token content: leading/trailing whitespace removed, internal
/// runs collapsed to a single space. Only XML whitespace (space, tab, CR,
/// LF) collapses — U+00A0, U+2009 and the other Unicode spaces are content.
fn text_content(node: roxmltree::Node) -> String {
    let raw: String = node.children().filter_map(|c| c.text()).collect();
    let mut out = String::with_capacity(raw.len());
    let mut in_space = true; // leading whitespace drops
    for c in raw.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(c);
            in_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}
