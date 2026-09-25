//! Math font access: a thin wrapper over `read-fonts`/`skrifa` that
//! guarantees the face carries an OpenType MATH table.

use read_fonts::tables::gsub::{AlternateSubstFormat1, SubstitutionSubtables};
use read_fonts::types::{GlyphId16, Tag};
use read_fonts::{FontRef, TableProvider};
use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::{BoundingBox, GlyphMetrics};
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlinePen};
use skrifa::MetadataProvider;

use crate::math::{self, Math, MathKernCorner};

/// A glyph index in the math font — the same 16-bit id the font's own
/// tables use; `0` is `.notdef`.
///
/// This is formulary's own type (not a re-export of its font parser's), so
/// internal parser upgrades are not breaking changes for consumers holding
/// glyph ids. Pass `.0` to whatever rasterization API draws your glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct GlyphId(pub u16);

impl GlyphId {
    /// The equivalent id in the font parser's terms, for internal table access.
    pub(crate) fn raw(self) -> GlyphId16 {
        GlyphId16::new(self.0)
    }
}

impl From<GlyphId16> for GlyphId {
    fn from(g: GlyphId16) -> Self {
        GlyphId(g.to_u16())
    }
}

/// Errors constructing a [`MathFont`].
#[derive(Debug)]
pub enum FontError {
    /// The bytes are not a parseable OpenType/TrueType font; the payload is
    /// a human-readable description of the parse failure.
    Face(String),
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

/// A corner of a glyph for MathKern lookups.
pub(crate) type KernCorner = MathKernCorner;

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

const MATH: Tag = Tag::new(b"MATH");
const SSTY: Tag = Tag::new(b"ssty");

/// A font validated to contain an OpenType MATH table.
///
/// Borrows the caller's font bytes; the crate does no I/O.
///
/// `MathFont` is `Send`, `Sync`, and unwind-safe (`RefUnwindSafe`) — a
/// guarantee, not an accident of the current fields: sharing one across
/// threads or catching a layout panic around a `&MathFont` is supported.
/// Its interior caches are all write-once, so a panic can't leave them
/// torn. A compile-time assertion below pins this.
pub struct MathFont<'a> {
    /// Kept for building shapers; every other accessor holds its own
    /// pre-resolved table view.
    #[cfg(feature = "shaping")]
    font: FontRef<'a>,
    units_per_em: f32,
    /// Font-wide ascent and descent in design units (descent positive).
    ascent: f32,
    descent: f32,
    x_height: Option<f32>,
    charmap: Charmap<'a>,
    metrics: GlyphMetrics<'a>,
    outlines: OutlineGlyphCollection<'a>,
    math: Math<'a>,
    /// Lazily-computed ink boxes, indexed by glyph id. Bounds of a CFF glyph
    /// come from walking its charstring on every call, and layout asks per
    /// glyph placed. The table itself is also lazy, so constructing a font
    /// that never renders stays allocation-light.
    ink_boxes: std::sync::OnceLock<Vec<std::sync::OnceLock<Option<BoundingBox>>>>,
    /// The GSUB `ssty` alternate subtables, resolved once so per-glyph
    /// script-alternate lookups don't rescan the feature list.
    ssty: Vec<AlternateSubstFormat1<'a>>,
    /// The shaper's per-font caches (lookup accelerators, cmap cache). A
    /// `Shaper` borrows these, so one is built per run; that's cheap, the
    /// expensive state lives here.
    #[cfg(feature = "shaping")]
    shaper: harfrust::ShaperData,
    /// Shape plans per script level (0, 1, 2+). Plan compilation walks the
    /// font's whole feature list and costs more than shaping a short run.
    /// `AssertUnwindSafe` upholds the type's documented unwind-safety
    /// despite the plan's internal `dyn Any`; the cache is write-once, so a
    /// panic can't leave it torn.
    #[cfg(feature = "shaping")]
    plans: std::panic::AssertUnwindSafe<[std::sync::OnceLock<harfrust::ShapePlan>; 3]>,
}

// Pin the documented auto-trait guarantees: adding a field that silently
// revoked any of these would be a breaking change consumers can't see in a
// diff, so make it fail the build here instead.
const _: () = {
    const fn assert_auto_traits<
        T: Send + Sync + std::panic::RefUnwindSafe + std::panic::UnwindSafe,
    >() {
    }
    assert_auto_traits::<MathFont<'static>>();
};

impl<'a> MathFont<'a> {
    /// Cheaply test whether `data` (face `index` for collections) carries a
    /// MATH table with constants — the precondition [`MathFont::new`]
    /// enforces — without constructing anything.
    ///
    /// Reads only the table directory and the MATH header, so it's suitable
    /// for scanning font lists. `true` means the precondition holds;
    /// construction can still fail if the font is malformed in other ways.
    pub fn probe(data: &[u8], index: u32) -> bool {
        FontRef::from_index(data, index)
            .ok()
            .and_then(|font| font.table_data(MATH))
            // The same parse `new` performs, so probe can't bless a MATH
            // table construction would reject.
            .and_then(Math::read)
            .is_some()
    }

    /// Parse `data` (face `index` for collections, 0 otherwise) and verify a
    /// MATH table with constants is present.
    pub fn new(data: &'a [u8], index: u32) -> Result<Self, FontError> {
        let font = FontRef::from_index(data, index).map_err(|e| FontError::Face(e.to_string()))?;
        let math = font
            .table_data(MATH)
            .and_then(Math::read)
            .ok_or(FontError::NoMathTable)?;
        let head = font.head().map_err(|e| FontError::Face(e.to_string()))?;
        let units_per_em = f32::from(head.units_per_em());
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let ssty = ssty_subtables(&font);
        Ok(MathFont {
            units_per_em,
            ascent: metrics.ascent,
            descent: -metrics.descent,
            x_height: metrics.x_height,
            charmap: font.charmap(),
            metrics: font.glyph_metrics(Size::unscaled(), LocationRef::default()),
            outlines: font.outline_glyphs(),
            math,
            ink_boxes: std::sync::OnceLock::new(),
            ssty,
            #[cfg(feature = "shaping")]
            shaper: harfrust::ShaperData::new(&font),
            #[cfg(feature = "shaping")]
            plans: std::panic::AssertUnwindSafe([const { std::sync::OnceLock::new() }; 3]),
            #[cfg(feature = "shaping")]
            font,
        })
    }

    /// A shaper over this font. Cheap to build: it borrows the per-font
    /// caches rather than re-reading tables.
    #[cfg(feature = "shaping")]
    pub(crate) fn shaper(&self) -> harfrust::Shaper<'_> {
        self.shaper.shaper(&self.font).build()
    }

    /// The cached shape plan for a script level (LTR, DFLT script, `ssty`
    /// enabled at levels 1 and 2).
    ///
    /// Runs are shaped under DFLT rather than the OpenType `math` script on
    /// purpose. Math fonts register `ssty`/`dtls` under `math`, but STIX Two
    /// Math (and fonts built the same way) leave `math`'s GPOS language
    /// system empty, so selecting it would silently drop `kern` from every
    /// multi-letter run. DFLT keeps kerning, and [`Self::script_alternate`]
    /// applies `ssty` by hand, which also covers the non-shaping build and
    /// single-glyph paths. Switching to `harfrust::script::MATH` here and in
    /// `shape_text_run` is the one change needed to prefer the font's own
    /// math feature set.
    #[cfg(feature = "shaping")]
    pub(crate) fn shape_plan(&self, script_level: u8) -> &harfrust::ShapePlan {
        let level = usize::from(script_level.min(2));
        self.plans[level].get_or_init(|| {
            let mut features = Vec::new();
            if level > 0 {
                features.push(harfrust::Feature::new(SSTY, level as u32, ..));
            }
            harfrust::ShapePlan::new(
                &self.shaper(),
                harfrust::Direction::LeftToRight,
                Some(harfrust::script::COMMON),
                None,
                &features,
            )
        })
    }

    pub fn units_per_em(&self) -> f32 {
        self.units_per_em
    }

    /// The MATH constants table. Present by construction.
    pub(crate) fn constants(&self) -> math::MathConstants<'a> {
        self.math.constants()
    }

    pub(crate) fn glyph_index(&self, c: char) -> Option<GlyphId> {
        let id = self.charmap.map(c)?;
        u16::try_from(id.to_u32()).ok().map(GlyphId)
    }

    /// Horizontal advance in font design units.
    pub(crate) fn advance(&self, glyph: GlyphId) -> f32 {
        self.metrics
            .advance_width(glyph.raw().into())
            .unwrap_or(0.0)
    }

    /// Ink bounding box in font design units (y-up), if the glyph has outlines.
    pub(crate) fn ink_box(&self, glyph: GlyphId) -> Option<BoundingBox> {
        let table = self.ink_boxes.get_or_init(|| {
            std::iter::repeat_with(std::sync::OnceLock::new)
                .take(self.metrics.glyph_count() as usize)
                .collect()
        });
        match table.get(usize::from(glyph.0)) {
            Some(slot) => *slot.get_or_init(|| self.bounds(glyph)),
            // Out of range: a corrupt font's GSUB/MATH subtables can name
            // glyph ids beyond the glyph count, so fall through uncached.
            None => self.bounds(glyph),
        }
    }

    fn bounds(&self, glyph: GlyphId) -> Option<BoundingBox> {
        self.metrics
            .bounds(glyph.raw().into())
            // A `glyf` glyph with no contours reports an all-zero box; treat
            // it as inkless like an empty CFF charstring.
            .filter(|b| b.x_min < b.x_max || b.y_min < b.y_max)
    }

    /// Emit `glyph`'s outline in design units (y-up) to `pen`. `false` when
    /// the glyph has no outline.
    pub(crate) fn outline(&self, glyph: GlyphId, pen: &mut impl OutlinePen) -> bool {
        let Some(outline) = self.outlines.get(glyph.raw().into()) else {
            return false;
        };
        outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                pen,
            )
            .is_ok()
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
        let Some(variants) = self.math.variants() else {
            return Stretched::Glyph(glyph);
        };
        let Some(construction) = variants.construction(glyph.raw(), horizontal) else {
            return Stretched::Glyph(glyph);
        };
        let mut best = glyph;
        for v in construction.variants() {
            best = v.variant_glyph.into();
            if f32::from(v.advance_measurement) >= target {
                return Stretched::Glyph(best);
            }
        }
        construction
            .glyph_assembly()
            .and_then(|asm| {
                self.assemble(&asm, f32::from(variants.min_connector_overlap()), target)
            })
            .unwrap_or(Stretched::Glyph(best))
    }

    /// Stack assembly parts (listed start-to-end along the stretch axis) to
    /// reach `target` design units, repeating extenders as needed and
    /// distributing a uniform connector overlap.
    fn assemble(
        &self,
        asm: &math::GlyphAssembly,
        min_overlap: f32,
        target: f32,
    ) -> Option<Stretched> {
        let asm_parts: Vec<math::GlyphPart> = asm.parts().collect();
        for repeats in 1..=32u32 {
            let mut seq: Vec<math::GlyphPart> = Vec::new();
            for part in &asm_parts {
                let n = if part.extender { repeats } else { 1 };
                for _ in 0..n {
                    seq.push(*part);
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
                .map(|w| f32::from(w[0].end_connector_length.min(w[1].start_connector_length)))
                .fold(f32::INFINITY, f32::min);
            overlap = overlap.clamp(min_overlap, max_overlap.max(min_overlap));

            let mut parts = Vec::with_capacity(seq.len());
            let mut bottom = 0.0;
            for p in &seq {
                parts.push((p.glyph_id.into(), bottom));
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
    /// Applied manually from GSUB: runs are shaped under DFLT, where math
    /// fonts don't register `ssty` (see [`Self::shape_plan`]), and the
    /// non-shaping build and single-glyph paths (stretchy operators,
    /// accents) need script alternates too.
    pub(crate) fn script_alternate(&self, glyph: GlyphId, level: u16) -> Option<GlyphId> {
        debug_assert!(level >= 1);
        for alt in &self.ssty {
            if let Some(idx) = alt.coverage().ok()?.get(glyph.raw()) {
                let set = alt.alternate_sets().get(usize::from(idx)).ok()?;
                let alternates = set.alternate_glyph_ids();
                // Deeper nesting takes the furthest available
                // alternate (ssty1, then ssty2 when the font has it).
                let last = alternates.len().checked_sub(1)?;
                return alternates
                    .get(usize::from(level.saturating_sub(1)).min(last))
                    .map(|g| g.get().into());
            }
        }
        None
    }

    fn glyph_info(&self, glyph: GlyphId) -> Option<(&math::MathGlyphInfo<'a>, GlyphId16)> {
        Some((self.math.glyph_info()?, glyph.raw()))
    }

    /// Italic correction of a glyph in design units (0 when absent): how far
    /// the glyph's ink slants past its advance, used to attach superscripts
    /// (add) and subscripts (subtract) around slanted glyphs.
    pub(crate) fn italic_correction(&self, glyph: GlyphId) -> f32 {
        self.glyph_info(glyph)
            .and_then(|(gi, g)| gi.italics_correction()?.get(g))
            .map_or(0.0, |v| f32::from(v.value))
    }

    /// The font's preferred horizontal accent position for `glyph`, in
    /// design units from the glyph origin, when the MATH table provides one.
    pub(crate) fn top_accent_attachment(&self, glyph: GlyphId) -> Option<f32> {
        self.glyph_info(glyph)
            .and_then(|(gi, g)| gi.top_accent_attachment()?.get(g))
            .map(|v| f32::from(v.value))
    }

    /// The MathKern cut-in for one corner of `glyph` at `height` design units
    /// above the baseline (negative below), or 0 when the font has none.
    ///
    /// The kern table is a staircase: `kern[i]` applies up to `height[i]`,
    /// with `kern[count]` above the last step.
    pub(crate) fn math_kern(&self, glyph: GlyphId, corner: KernCorner, height: f32) -> f32 {
        let Some(kern) = self
            .glyph_info(glyph)
            .and_then(|(gi, g)| gi.kern_info()?.get(g, corner))
        else {
            return 0.0;
        };
        let count = kern.count();
        let mut index = count;
        for i in 0..count {
            let step = kern.height(i).map_or(f32::MAX, |v| f32::from(v.value));
            if height <= step {
                index = i;
                break;
            }
        }
        kern.kern(index).map_or(0.0, |v| f32::from(v.value))
    }

    /// x-height in design units, with a common fallback when the OS/2 table
    /// doesn't provide one.
    pub(crate) fn x_height(&self) -> f32 {
        self.x_height.unwrap_or(self.units_per_em * 0.5)
    }

    /// Font-wide ascent/descent in design units (descent returned positive).
    pub(crate) fn line_metrics(&self) -> (f32, f32) {
        (self.ascent, self.descent)
    }
}

/// The alternate-substitution subtables reachable from the GSUB `ssty`
/// feature, in feature order.
fn ssty_subtables<'a>(font: &FontRef<'a>) -> Vec<AlternateSubstFormat1<'a>> {
    let mut out = Vec::new();
    let Ok(gsub) = font.gsub() else {
        return out;
    };
    let (Ok(features), Ok(lookups)) = (gsub.feature_list(), gsub.lookup_list()) else {
        return out;
    };
    let Some(feature) = features
        .feature_records()
        .iter()
        .find(|r| r.feature_tag() == SSTY)
        .and_then(|r| r.feature(features.offset_data()).ok())
    else {
        return out;
    };
    for li in feature.lookup_list_indices() {
        let Ok(lookup) = lookups.lookups().get(usize::from(li.get())) else {
            continue;
        };
        if let Ok(SubstitutionSubtables::Alternate(subs)) = lookup.subtables() {
            out.extend(subs.iter().flatten());
        }
    }
    out
}
