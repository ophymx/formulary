//! OpenType [MATH] table reader over `read-fonts` byte primitives.
//!
//! `read-fonts` (as of 0.43) ships no MATH table; fontations' main branch has
//! one, and the types and accessors here follow its naming so that swapping
//! to it once released is a mechanical change. Device tables are ignored:
//! layout is resolution-independent, so only design-unit values matter.
//!
//! Every accessor is total: a truncated or corrupt subtable reads as absent
//! or zero rather than failing, which is what a layout engine wants from a
//! font it has already accepted.
//!
//! [MATH]: https://learn.microsoft.com/en-us/typography/opentype/spec/math

use read_fonts::tables::layout::CoverageTable;
use read_fonts::types::GlyphId16;
use read_fonts::{FontData, FontRead};

/// A MATH value in design units (the device-table adjustment is dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MathValue {
    pub value: i16,
}

fn u16_at(data: FontData<'_>, offset: usize) -> Option<u16> {
    data.read_at::<u16>(offset).ok()
}

fn i16_at(data: FontData<'_>, offset: usize) -> Option<i16> {
    data.read_at::<i16>(offset).ok()
}

fn value_at(data: FontData<'_>, offset: usize) -> Option<MathValue> {
    i16_at(data, offset).map(|value| MathValue { value })
}

/// The subtable an `Offset16` at `offset` points to, `None` when null or out
/// of bounds.
fn subtable_at<'a>(data: FontData<'a>, offset: usize) -> Option<FontData<'a>> {
    match u16_at(data, offset)? {
        0 => None,
        off => data.split_off(usize::from(off)),
    }
}

fn coverage_at<'a>(data: FontData<'a>, offset: usize) -> Option<CoverageTable<'a>> {
    CoverageTable::read(subtable_at(data, offset)?).ok()
}

/// The MATH table header.
#[derive(Clone)]
pub(crate) struct Math<'a> {
    constants: MathConstants<'a>,
    glyph_info: Option<MathGlyphInfo<'a>>,
    variants: Option<MathVariants<'a>>,
}

impl<'a> Math<'a> {
    /// Parse a MATH table. `None` when the version is unknown or the
    /// constants subtable (which layout can't do without) is missing or
    /// truncated.
    pub(crate) fn read(data: FontData<'a>) -> Option<Self> {
        if u16_at(data, 0)? != 1 {
            return None;
        }
        let constants = MathConstants::read(subtable_at(data, 4)?)?;
        Some(Math {
            constants,
            glyph_info: subtable_at(data, 6).map(MathGlyphInfo),
            variants: subtable_at(data, 8).map(MathVariants),
        })
    }

    pub(crate) fn constants(&self) -> MathConstants<'a> {
        self.constants
    }

    pub(crate) fn glyph_info(&self) -> Option<&MathGlyphInfo<'a>> {
        self.glyph_info.as_ref()
    }

    pub(crate) fn variants(&self) -> Option<&MathVariants<'a>> {
        self.variants.as_ref()
    }
}

/// The MathConstants subtable: font-wide layout parameters.
#[derive(Clone, Copy)]
pub(crate) struct MathConstants<'a>(FontData<'a>);

/// Byte size of a complete MathConstants subtable.
const CONSTANTS_LEN: usize = 214;

macro_rules! math_values {
    ($($name:ident = $index:expr),* $(,)?) => {
        $(
            #[inline]
            pub(crate) fn $name(&self) -> MathValue {
                value_at(self.0, 8 + 4 * $index).unwrap_or_default()
            }
        )*
    };
}

// The full constant set is exposed for parity with the spec (and with the
// fontations table this mirrors), not just the subset layout uses today.
#[allow(dead_code)]
impl<'a> MathConstants<'a> {
    fn read(data: FontData<'a>) -> Option<Self> {
        (data.len() >= CONSTANTS_LEN).then_some(MathConstants(data))
    }

    pub(crate) fn script_percent_scale_down(&self) -> i16 {
        i16_at(self.0, 0).unwrap_or(0)
    }

    pub(crate) fn script_script_percent_scale_down(&self) -> i16 {
        i16_at(self.0, 2).unwrap_or(0)
    }

    pub(crate) fn delimited_sub_formula_min_height(&self) -> u16 {
        u16_at(self.0, 4).unwrap_or(0)
    }

    pub(crate) fn display_operator_min_height(&self) -> u16 {
        u16_at(self.0, 6).unwrap_or(0)
    }

    math_values! {
        math_leading = 0,
        axis_height = 1,
        accent_base_height = 2,
        flattened_accent_base_height = 3,
        subscript_shift_down = 4,
        subscript_top_max = 5,
        subscript_baseline_drop_min = 6,
        superscript_shift_up = 7,
        superscript_shift_up_cramped = 8,
        superscript_bottom_min = 9,
        superscript_baseline_drop_max = 10,
        sub_superscript_gap_min = 11,
        superscript_bottom_max_with_subscript = 12,
        space_after_script = 13,
        upper_limit_gap_min = 14,
        upper_limit_baseline_rise_min = 15,
        lower_limit_gap_min = 16,
        lower_limit_baseline_drop_min = 17,
        stack_top_shift_up = 18,
        stack_top_display_style_shift_up = 19,
        stack_bottom_shift_down = 20,
        stack_bottom_display_style_shift_down = 21,
        stack_gap_min = 22,
        stack_display_style_gap_min = 23,
        stretch_stack_top_shift_up = 24,
        stretch_stack_bottom_shift_down = 25,
        stretch_stack_gap_above_min = 26,
        stretch_stack_gap_below_min = 27,
        fraction_numerator_shift_up = 28,
        fraction_numerator_display_style_shift_up = 29,
        fraction_denominator_shift_down = 30,
        fraction_denominator_display_style_shift_down = 31,
        fraction_numerator_gap_min = 32,
        fraction_num_display_style_gap_min = 33,
        fraction_rule_thickness = 34,
        fraction_denominator_gap_min = 35,
        fraction_denom_display_style_gap_min = 36,
        skewed_fraction_horizontal_gap = 37,
        skewed_fraction_vertical_gap = 38,
        overbar_vertical_gap = 39,
        overbar_rule_thickness = 40,
        overbar_extra_ascender = 41,
        underbar_vertical_gap = 42,
        underbar_rule_thickness = 43,
        underbar_extra_descender = 44,
        radical_vertical_gap = 45,
        radical_display_style_vertical_gap = 46,
        radical_rule_thickness = 47,
        radical_extra_ascender = 48,
        radical_kern_before_degree = 49,
        radical_kern_after_degree = 50,
    }

    pub(crate) fn radical_degree_bottom_raise_percent(&self) -> i16 {
        i16_at(self.0, 212).unwrap_or(0)
    }
}

/// A coverage table paired with one MathValueRecord per covered glyph
/// (MathItalicsCorrectionInfo and MathTopAccentAttachment share the layout).
#[derive(Clone)]
pub(crate) struct MathValueTable<'a> {
    coverage: CoverageTable<'a>,
    data: FontData<'a>,
}

impl<'a> MathValueTable<'a> {
    fn read(data: FontData<'a>) -> Option<Self> {
        Some(MathValueTable {
            coverage: coverage_at(data, 0)?,
            data,
        })
    }

    pub(crate) fn get(&self, glyph: GlyphId16) -> Option<MathValue> {
        let index = usize::from(self.coverage.get(glyph)?);
        if index >= usize::from(u16_at(self.data, 2)?) {
            return None;
        }
        value_at(self.data, 4 + 4 * index)
    }
}

/// The MathGlyphInfo subtable: per-glyph corrections and kerning.
#[derive(Clone)]
pub(crate) struct MathGlyphInfo<'a>(FontData<'a>);

impl<'a> MathGlyphInfo<'a> {
    pub(crate) fn italics_correction(&self) -> Option<MathValueTable<'a>> {
        MathValueTable::read(subtable_at(self.0, 0)?)
    }

    pub(crate) fn top_accent_attachment(&self) -> Option<MathValueTable<'a>> {
        MathValueTable::read(subtable_at(self.0, 2)?)
    }

    pub(crate) fn kern_info(&self) -> Option<MathKernInfo<'a>> {
        MathKernInfo::read(subtable_at(self.0, 6)?)
    }
}

/// Which corner of a glyph a [`MathKern`] applies to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum MathKernCorner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

/// The MathKernInfo subtable: cut-in kerning per glyph corner.
#[derive(Clone)]
pub(crate) struct MathKernInfo<'a> {
    coverage: CoverageTable<'a>,
    data: FontData<'a>,
}

impl<'a> MathKernInfo<'a> {
    fn read(data: FontData<'a>) -> Option<Self> {
        Some(MathKernInfo {
            coverage: coverage_at(data, 0)?,
            data,
        })
    }

    /// The kern table for one corner of `glyph`, if the font has one.
    pub(crate) fn get(&self, glyph: GlyphId16, corner: MathKernCorner) -> Option<MathKern<'a>> {
        let index = usize::from(self.coverage.get(glyph)?);
        if index >= usize::from(u16_at(self.data, 2)?) {
            return None;
        }
        let corner = match corner {
            MathKernCorner::TopRight => 0,
            MathKernCorner::TopLeft => 2,
            MathKernCorner::BottomRight => 4,
            MathKernCorner::BottomLeft => 6,
        };
        subtable_at(self.data, 4 + 8 * index + corner).map(MathKern)
    }
}

/// A MathKern table: a staircase of kern values by height. `kern(i)` applies
/// up to `height(i)`, with `kern(count())` above the last step.
#[derive(Clone)]
pub(crate) struct MathKern<'a>(FontData<'a>);

impl<'a> MathKern<'a> {
    pub(crate) fn count(&self) -> u16 {
        u16_at(self.0, 0).unwrap_or(0)
    }

    pub(crate) fn height(&self, index: u16) -> Option<MathValue> {
        (index < self.count()).then(|| value_at(self.0, 2 + 4 * usize::from(index)))?
    }

    pub(crate) fn kern(&self, index: u16) -> Option<MathValue> {
        let count = self.count();
        (index <= count)
            .then(|| value_at(self.0, 2 + 4 * usize::from(count) + 4 * usize::from(index)))?
    }
}

/// The MathVariants subtable: pre-drawn stretched variants and assemblies.
#[derive(Clone)]
pub(crate) struct MathVariants<'a>(FontData<'a>);

impl<'a> MathVariants<'a> {
    pub(crate) fn min_connector_overlap(&self) -> u16 {
        u16_at(self.0, 0).unwrap_or(0)
    }

    /// The construction for stretching `glyph` along the given axis.
    pub(crate) fn construction(
        &self,
        glyph: GlyphId16,
        horizontal: bool,
    ) -> Option<MathGlyphConstruction<'a>> {
        let vert_count = usize::from(u16_at(self.0, 6)?);
        let (coverage, count, base) = if horizontal {
            (4, usize::from(u16_at(self.0, 8)?), 10 + 2 * vert_count)
        } else {
            (2, vert_count, 10)
        };
        let index = usize::from(coverage_at(self.0, coverage)?.get(glyph)?);
        if index >= count {
            return None;
        }
        subtable_at(self.0, base + 2 * index).map(MathGlyphConstruction)
    }
}

/// A pre-drawn variant of a stretchy glyph.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MathGlyphVariant {
    pub variant_glyph: GlyphId16,
    /// The variant's size along the stretch axis, in design units.
    pub advance_measurement: u16,
}

/// The MathGlyphConstruction subtable for one glyph and axis.
#[derive(Clone)]
pub(crate) struct MathGlyphConstruction<'a>(FontData<'a>);

impl<'a> MathGlyphConstruction<'a> {
    pub(crate) fn glyph_assembly(&self) -> Option<GlyphAssembly<'a>> {
        subtable_at(self.0, 0).map(GlyphAssembly)
    }

    /// The variants, smallest first.
    pub(crate) fn variants(&self) -> impl Iterator<Item = MathGlyphVariant> + '_ {
        let count = usize::from(u16_at(self.0, 2).unwrap_or(0));
        (0..count).map_while(move |i| {
            Some(MathGlyphVariant {
                variant_glyph: GlyphId16::new(u16_at(self.0, 4 + 4 * i)?),
                advance_measurement: u16_at(self.0, 6 + 4 * i)?,
            })
        })
    }
}

/// One piece of a glyph assembly.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphPart {
    pub glyph_id: GlyphId16,
    pub start_connector_length: u16,
    pub end_connector_length: u16,
    pub full_advance: u16,
    /// Whether the part may repeat to fill the target size.
    pub extender: bool,
}

/// The GlyphAssembly subtable: parts stacked along the stretch axis.
#[derive(Clone)]
pub(crate) struct GlyphAssembly<'a>(FontData<'a>);

impl<'a> GlyphAssembly<'a> {
    /// The parts, listed start-to-end along the stretch axis.
    pub(crate) fn parts(&self) -> impl Iterator<Item = GlyphPart> + '_ {
        let count = usize::from(u16_at(self.0, 4).unwrap_or(0));
        (0..count).map_while(move |i| {
            let at = 6 + 10 * i;
            Some(GlyphPart {
                glyph_id: GlyphId16::new(u16_at(self.0, at)?),
                start_connector_length: u16_at(self.0, at + 2)?,
                end_connector_length: u16_at(self.0, at + 4)?,
                full_advance: u16_at(self.0, at + 6)?,
                extender: u16_at(self.0, at + 8)? & 1 != 0,
            })
        })
    }
}
