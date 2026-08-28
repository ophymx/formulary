//! Presentation MathML → typed element tree.

use crate::ast::{DisplayMode, Form, Length, MathRoot, Node, OperatorAttrs, ScriptLevel};

/// Errors produced while turning MathML markup into an element tree.
#[derive(Debug)]
pub enum ParseError {
    /// The input is not well-formed XML.
    Xml(roxmltree::Error),
    /// The root element is not `<math>`.
    NotMath { found: String },
    /// An element this version of the crate cannot lay out yet.
    Unsupported { element: String },
    /// An element with a fixed arity got the wrong number of children
    /// (e.g. `<mfrac>` requires exactly two).
    WrongArity {
        element: &'static str,
        expected: usize,
        found: usize,
    },
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Xml(e) => write!(f, "malformed XML: {e}"),
            ParseError::NotMath { found } => {
                write!(f, "expected <math> root element, found <{found}>")
            }
            ParseError::Unsupported { element } => {
                write!(f, "unsupported MathML element <{element}>")
            }
            ParseError::WrongArity {
                element,
                expected,
                found,
            } => {
                write!(f, "<{element}> requires {expected} children, found {found}")
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
    let children = parse_children(root)?;
    Ok(MathRoot { display, children })
}

fn parse_children(parent: roxmltree::Node) -> Result<Vec<Node>, ParseError> {
    parent
        .children()
        .filter(|c| c.is_element())
        .map(parse_node)
        .collect()
}

fn parse_node(node: roxmltree::Node) -> Result<Node, ParseError> {
    let name = node.tag_name().name();
    match name {
        "mi" => Ok(Node::Identifier(text_content(node))),
        "mn" => Ok(Node::Number(text_content(node))),
        "mo" => Ok(Node::Operator {
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
            },
        }),
        "mtext" => Ok(Node::Text(text_content(node))),
        "mrow" => Ok(Node::Row(parse_children(node)?)),
        "mfrac" => {
            let mut children = parse_children(node)?;
            if children.len() != 2 {
                return Err(ParseError::WrongArity {
                    element: "mfrac",
                    expected: 2,
                    found: children.len(),
                });
            }
            let den = Box::new(children.pop().expect("len checked"));
            let num = Box::new(children.pop().expect("len checked"));
            Ok(Node::Frac { num, den })
        }
        "msub" | "msup" | "msubsup" => {
            let expected = if name == "msubsup" { 3 } else { 2 };
            let mut children = parse_children(node)?;
            if children.len() != expected {
                return Err(ParseError::WrongArity {
                    element: match name {
                        "msub" => "msub",
                        "msup" => "msup",
                        _ => "msubsup",
                    },
                    expected,
                    found: children.len(),
                });
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
            Ok(Node::Scripts { base, sub, sup })
        }
        "munder" | "mover" | "munderover" => {
            let expected = if name == "munderover" { 3 } else { 2 };
            let mut children = parse_children(node)?;
            if children.len() != expected {
                return Err(ParseError::WrongArity {
                    element: match name {
                        "munder" => "munder",
                        "mover" => "mover",
                        _ => "munderover",
                    },
                    expected,
                    found: children.len(),
                });
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
            Ok(Node::UnderOver {
                base,
                under,
                over,
                accent: bool_attr(node, "accent"),
                accent_under: bool_attr(node, "accentunder"),
            })
        }
        "mtable" => {
            let rows = node
                .children()
                .filter(|c| c.is_element())
                .map(|row| {
                    if row.tag_name().name() != "mtr" {
                        return Err(ParseError::Unsupported {
                            element: row.tag_name().name().to_string(),
                        });
                    }
                    row.children()
                        .filter(|c| c.is_element())
                        .map(|cell| {
                            if cell.tag_name().name() != "mtd" {
                                return Err(ParseError::Unsupported {
                                    element: cell.tag_name().name().to_string(),
                                });
                            }
                            Ok(Node::Row(parse_children(cell)?))
                        })
                        .collect()
                })
                .collect::<Result<Vec<Vec<Node>>, ParseError>>()?;
            Ok(Node::Table { rows })
        }
        "msqrt" => Ok(Node::Sqrt(parse_children(node)?)),
        "mspace" => Ok(Node::Space {
            width: length_attr(node, "width"),
            height: length_attr(node, "height"),
            depth: length_attr(node, "depth"),
        }),
        "mstyle" => Ok(Node::Styled {
            display_style: match node.attribute("displaystyle") {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            },
            script_level: node.attribute("scriptlevel").and_then(parse_script_level),
            children: parse_children(node)?,
        }),
        "mphantom" => Ok(Node::Phantom(parse_children(node)?)),
        "mpadded" => Ok(Node::Padded {
            width: length_attr(node, "width"),
            height: length_attr(node, "height"),
            depth: length_attr(node, "depth"),
            lspace: length_attr(node, "lspace"),
            voffset: length_attr(node, "voffset"),
            children: parse_children(node)?,
        }),
        "mroot" => {
            let mut children = parse_children(node)?;
            if children.len() != 2 {
                return Err(ParseError::WrongArity {
                    element: "mroot",
                    expected: 2,
                    found: children.len(),
                });
            }
            let index = Box::new(children.pop().expect("len checked"));
            let base = Box::new(children.pop().expect("len checked"));
            Ok(Node::Root { base, index })
        }
        other => Err(ParseError::Unsupported {
            element: other.to_string(),
        }),
    }
}

/// A length-valued attribute. Invalid values behave like an absent attribute,
/// per MathML's error-recovery convention — markup in the wild is messy and a
/// bad `width` shouldn't kill the whole formula.
fn length_attr(node: roxmltree::Node, name: &str) -> Option<Length> {
    node.attribute(name).and_then(parse_length)
}

fn bool_attr(node: roxmltree::Node, name: &str) -> Option<bool> {
    match node.attribute(name) {
        Some("true") => Some(true),
        Some("false") => Some(false),
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
/// runs collapsed to a single space.
fn text_content(node: roxmltree::Node) -> String {
    let raw: String = node.children().filter_map(|c| c.text()).collect();
    let mut out = String::with_capacity(raw.len());
    let mut in_space = true; // leading whitespace drops
    for c in raw.chars() {
        if c.is_whitespace() {
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
