//! Math font access: a thin wrapper over `ttf-parser` that guarantees the
//! face carries an OpenType MATH table.

pub use ttf_parser::GlyphId;

/// Errors constructing a [`MathFont`].
#[derive(Debug)]
pub enum FontError {
    /// The bytes are not a parseable OpenType/TrueType font.
    Face(ttf_parser::FaceParsingError),
    /// The font parsed but has no MATH table; math layout is impossible
    /// without one.
    NoMathTable,
}

impl core::fmt::Display for FontError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FontError::Face(e) => write!(f, "cannot parse font: {e}"),
            FontError::NoMathTable => write!(f, "font has no OpenType MATH table"),
        }
    }
}

impl std::error::Error for FontError {}

/// A font validated to contain an OpenType MATH table.
///
/// Borrows the caller's font bytes; the crate does no I/O.
pub struct MathFont<'a> {
    face: ttf_parser::Face<'a>,
    units_per_em: f32,
}

impl<'a> MathFont<'a> {
    /// Parse `data` (face `index` for collections, 0 otherwise) and verify a
    /// MATH table with constants is present.
    pub fn new(data: &'a [u8], index: u32) -> Result<Self, FontError> {
        let face = ttf_parser::Face::parse(data, index).map_err(FontError::Face)?;
        if face.tables().math.and_then(|m| m.constants).is_none() {
            return Err(FontError::NoMathTable);
        }
        let units_per_em = face.units_per_em() as f32;
        Ok(MathFont { face, units_per_em })
    }

    pub fn units_per_em(&self) -> f32 {
        self.units_per_em
    }

    /// The MATH constants table. Present by construction.
    #[allow(dead_code)] // first consumer is mfrac layout
    pub(crate) fn constants(&self) -> ttf_parser::math::Constants<'a> {
        self.face
            .tables()
            .math
            .and_then(|m| m.constants)
            .expect("verified at construction")
    }

    pub(crate) fn face(&self) -> &ttf_parser::Face<'a> {
        &self.face
    }

    pub(crate) fn glyph_index(&self, c: char) -> Option<GlyphId> {
        self.face.glyph_index(c)
    }

    /// Horizontal advance in font design units.
    pub(crate) fn advance(&self, glyph: GlyphId) -> f32 {
        self.face.glyph_hor_advance(glyph).unwrap_or(0) as f32
    }

    /// Ink bounding box in font design units (y-up), if the glyph has outlines.
    pub(crate) fn ink_box(&self, glyph: GlyphId) -> Option<ttf_parser::Rect> {
        self.face.glyph_bounding_box(glyph)
    }

    /// The smallest vertical variant of `glyph` whose advance covers
    /// `target` design units, or the largest available if none do, or
    /// `glyph` itself if the font has no construction for it.
    ///
    /// Glyph assembly (building arbitrary heights from extender parts) is not
    /// implemented yet; very tall radicals/delimiters top out at the largest
    /// pre-drawn variant.
    pub(crate) fn vertical_variant(&self, glyph: GlyphId, target: f32) -> GlyphId {
        let Some(construction) = self
            .face
            .tables()
            .math
            .and_then(|m| m.variants)
            .and_then(|v| v.vertical_constructions.get(glyph))
        else {
            return glyph;
        };
        let mut best = glyph;
        for v in construction.variants {
            best = v.variant_glyph;
            if f32::from(v.advance_measurement) >= target {
                break;
            }
        }
        best
    }

    /// Font-wide ascent/descent in design units (descent returned positive).
    pub(crate) fn line_metrics(&self) -> (f32, f32) {
        (
            self.face.ascender() as f32,
            -(self.face.descender() as f32),
        )
    }
}
