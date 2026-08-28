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

/// A stretched glyph: a single (possibly variant) glyph, or a stack of
/// assembly parts along the stretch axis.
pub(crate) enum Stretched {
    Glyph(GlyphId),
    /// `parts` are `(glyph, offset along the stretch axis)` in design units,
    /// measured from the assembly's start (bottom for vertical, left for
    /// horizontal); `extent` is the total assembled size along that axis.
    Assembly {
        parts: Vec<(GlyphId, f32)>,
        extent: f32,
    },
}

/// A font validated to contain an OpenType MATH table.
///
/// Borrows the caller's font bytes; the crate does no I/O.
pub struct MathFont<'a> {
    face: ttf_parser::Face<'a>,
    units_per_em: f32,
    #[cfg(feature = "shaping")]
    shaper: Option<rustybuzz::Face<'a>>,
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
        Ok(MathFont {
            face,
            units_per_em,
            #[cfg(feature = "shaping")]
            shaper: rustybuzz::Face::from_slice(data, index),
        })
    }

    #[cfg(feature = "shaping")]
    pub(crate) fn shaper(&self) -> Option<&rustybuzz::Face<'a>> {
        self.shaper.as_ref()
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

    /// A glyph stretched vertically to at least `target` design units.
    pub(crate) fn stretch_vertical(&self, glyph: GlyphId, target: f32) -> Stretched {
        self.stretch(glyph, target, false)
    }

    /// A glyph stretched horizontally to at least `target` design units.
    pub(crate) fn stretch_horizontal(&self, glyph: GlyphId, target: f32) -> Stretched {
        self.stretch(glyph, target, true)
    }

    /// Tries the pre-drawn variants smallest-first, then glyph assembly from
    /// extender parts; falls back to the largest variant (or the base glyph)
    /// when neither can reach the target.
    fn stretch(&self, glyph: GlyphId, target: f32, horizontal: bool) -> Stretched {
        let Some(variants) = self.face.tables().math.and_then(|m| m.variants) else {
            return Stretched::Glyph(glyph);
        };
        let constructions = if horizontal {
            variants.horizontal_constructions
        } else {
            variants.vertical_constructions
        };
        let Some(construction) = constructions.get(glyph) else {
            return Stretched::Glyph(glyph);
        };
        let mut best = glyph;
        for v in construction.variants {
            best = v.variant_glyph;
            if f32::from(v.advance_measurement) >= target {
                return Stretched::Glyph(best);
            }
        }
        construction
            .assembly
            .and_then(|asm| {
                self.assemble(&asm, f32::from(variants.min_connector_overlap), target)
            })
            .unwrap_or(Stretched::Glyph(best))
    }

    /// Stack assembly parts (listed start-to-end along the stretch axis) to
    /// reach `target` design units, repeating extenders as needed and
    /// distributing a uniform connector overlap.
    fn assemble(
        &self,
        asm: &ttf_parser::math::GlyphAssembly,
        min_overlap: f32,
        target: f32,
    ) -> Option<Stretched> {
        for repeats in 1..=32u32 {
            let mut seq: Vec<ttf_parser::math::GlyphPart> = Vec::new();
            for part in asm.parts {
                let n = if part.part_flags.extender() { repeats } else { 1 };
                for _ in 0..n {
                    seq.push(part);
                }
            }
            if seq.len() < 2 {
                return None;
            }
            let sum: f32 = seq.iter().map(|p| f32::from(p.full_advance)).sum();
            let joints = (seq.len() - 1) as f32;
            if sum - min_overlap * joints < target {
                continue; // not enough parts yet even at minimum overlap
            }
            // Overlap that lands exactly on target, kept within what the
            // connectors allow.
            let mut overlap = (sum - target) / joints;
            let max_overlap = seq
                .windows(2)
                .map(|w| {
                    f32::from(w[0].end_connector_length.min(w[1].start_connector_length))
                })
                .fold(f32::INFINITY, f32::min);
            overlap = overlap.clamp(min_overlap, max_overlap.max(min_overlap));

            let mut parts = Vec::with_capacity(seq.len());
            let mut bottom = 0.0;
            for p in &seq {
                parts.push((p.glyph_id, bottom));
                bottom += f32::from(p.full_advance) - overlap;
            }
            let extent = bottom + overlap;
            return Some(Stretched::Assembly { parts, extent });
        }
        None
    }

    /// The font's `ssty` alternate for `glyph` at script depth `level` (1 or
    /// 2): script-tuned glyph variants, primes being the classic case.
    ///
    /// Applied manually from GSUB rather than through shaping: math fonts
    /// register `ssty` under the OpenType `math` script, which rustybuzz
    /// (as of 0.20) never selects — it lowercases the Unicode script tag
    /// `Zmth` to `zmth` instead of HarfBuzz's special-cased `math`.
    pub(crate) fn script_alternate(&self, glyph: GlyphId, level: u16) -> Option<GlyphId> {
        debug_assert!(level >= 1);
        let gsub = self.face.tables().gsub?;
        let feature = gsub
            .features
            .find(ttf_parser::Tag::from_bytes(b"ssty"))?;
        for li in feature.lookup_indices {
            let Some(lookup) = gsub.lookups.get(li) else {
                continue;
            };
            for sub in lookup
                .subtables
                .into_iter::<ttf_parser::gsub::SubstitutionSubtable>()
            {
                if let ttf_parser::gsub::SubstitutionSubtable::Alternate(alt) = sub {
                    if let Some(idx) = alt.coverage.get(glyph) {
                        let set = alt.alternate_sets.get(idx)?;
                        // Deeper nesting takes the furthest available
                        // alternate (ssty1, then ssty2 when the font has it).
                        let last = set.alternates.len().checked_sub(1)?;
                        return set.alternates.get(level.saturating_sub(1).min(last));
                    }
                }
            }
        }
        None
    }

    /// Italic correction of a glyph in design units (0 when absent): how far
    /// the glyph's ink slants past its advance, used to attach superscripts
    /// (add) and subscripts (subtract) around slanted glyphs.
    pub(crate) fn italic_correction(&self, glyph: GlyphId) -> f32 {
        self.face
            .tables()
            .math
            .and_then(|m| m.glyph_info)
            .and_then(|gi| gi.italic_corrections)
            .and_then(|ic| ic.get(glyph))
            .map_or(0.0, |v| f32::from(v.value))
    }

    /// x-height in design units, with a common fallback when the OS/2 table
    /// doesn't provide one.
    pub(crate) fn x_height(&self) -> f32 {
        self.face
            .x_height()
            .map(f32::from)
            .unwrap_or(self.units_per_em * 0.5)
    }

    /// Font-wide ascent/descent in design units (descent returned positive).
    pub(crate) fn line_metrics(&self) -> (f32, f32) {
        (
            self.face.ascender() as f32,
            -(self.face.descender() as f32),
        )
    }
}
