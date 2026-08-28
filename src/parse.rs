//! Presentation MathML → typed element tree.

use crate::ast::{DisplayMode, MathRoot, Node};

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
        "mo" => Ok(Node::Operator(text_content(node))),
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
        other => Err(ParseError::Unsupported {
            element: other.to_string(),
        }),
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
