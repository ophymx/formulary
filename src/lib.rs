//! # formulary
//!
//! Native [MathML Core](https://www.w3.org/TR/mathml-core/) layout in Rust.
//! Parse presentation MathML, lay it out using the font's OpenType MATH table,
//! and get back a resolution-independent [`Layout`] — glyph ids and rules with
//! baseline metrics — that you rasterize through your own text pipeline.
//!
//! No webview, no rasterization in the core, no I/O: you bring the font bytes
//! (any font with a MATH table, e.g. STIX Two Math or New Computer Modern).
//!
//! ```
//! use formulary::{parse, layout, LayoutOptions, MathFont};
//!
//! let font_data = std::fs::read("fonts/STIXTwoMath-Regular.otf").unwrap();
//! let font = MathFont::new(&font_data, 0).unwrap();
//! let tree = parse("<math><mi>x</mi><mo>+</mo><mn>2</mn></math>").unwrap();
//! let laid = layout(&tree, &font, &LayoutOptions { font_size: 16.0 });
//! assert!(laid.width > 0.0);
//! ```

pub mod ast;
mod font;
mod layout;
mod mathvariant;
mod parse;
#[cfg(feature = "svg")]
pub mod svg;

pub use ast::{DisplayMode, MathRoot};
pub use font::{FontError, GlyphId, MathFont};
pub use layout::{layout, Item, Layout, LayoutOptions};
pub use parse::{parse, ParseError};
