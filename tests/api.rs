//! Feature-independent tests of the 0.2 public API additions: `probe`,
//! `Item::Glyph::advance`, the `GlyphId` newtype, and Arabic-text detection.

use formulary::{layout, parse, GlyphId, Item, LayoutOptions, MathFont};

fn stix() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/STIXTwoMath-Regular.otf"
    ))
    .expect("vendored test font")
}

#[test]
fn probe_accepts_math_font_and_agrees_with_new() {
    let data = stix();
    assert!(MathFont::probe(&data, 0));
    assert!(MathFont::new(&data, 0).is_ok());
}

#[test]
fn probe_rejects_non_fonts() {
    assert!(!MathFont::probe(&[], 0));
    assert!(!MathFont::probe(b"definitely not a font", 0));
    assert!(!MathFont::probe(&stix()[..64], 0));
}

#[test]
fn probe_rejects_font_without_math_table() {
    // Rename the MATH tag in the table directory so the same otherwise-valid
    // font no longer carries one.
    let mut data = stix();
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    let mut renamed = false;
    for i in 0..num_tables {
        let at = 12 + 16 * i;
        if &data[at..at + 4] == b"MATH" {
            data[at..at + 4].copy_from_slice(b"XATH");
            renamed = true;
        }
    }
    assert!(renamed, "test font should have a MATH table to rename");
    assert!(!MathFont::probe(&data, 0));
    assert!(MathFont::new(&data, 0).is_err());
}

#[test]
fn glyph_items_carry_the_nominal_advance() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let tree = parse("<math><mi>x</mi></math>").unwrap();
    let laid = layout(&tree, &font, &LayoutOptions { font_size: 16.0 });
    let glyphs: Vec<_> = laid
        .items
        .iter()
        .filter_map(|item| match *item {
            Item::Glyph { id, advance, .. } => Some((id, advance)),
            _ => None,
        })
        .collect();
    assert_eq!(glyphs.len(), 1);
    let (id, advance) = glyphs[0];
    assert!(id != GlyphId(0), "x should map to a real glyph");
    assert!(advance > 0.0);
    // The nominal advance and the run width coincide here because a lone
    // glyph gets no kerning and STIX applies no GPOS width adjustment to
    // it — an empirical check on the vendored font, not a contract: shaped
    // runs may bake different spacing into positions than the nominal
    // advance reports.
    assert!((advance - laid.width).abs() < 1e-4);
}

#[test]
fn advance_scales_with_font_size() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let tree = parse("<math><mn>7</mn></math>").unwrap();
    let advance_at = |size: f32| {
        let laid = layout(&tree, &font, &LayoutOptions { font_size: size });
        laid.items
            .iter()
            .find_map(|item| match *item {
                Item::Glyph { advance, .. } => Some(advance),
                _ => None,
            })
            .unwrap()
    };
    let a16 = advance_at(16.0);
    let a32 = advance_at(32.0);
    assert!((a32 - 2.0 * a16).abs() < 1e-3);
}

#[test]
fn arabic_text_detection() {
    let has = |markup: &str| parse(markup).unwrap().has_arabic_text();
    assert!(!has("<math><mi>x</mi><mo>+</mo><mn>2</mn></math>"));
    // An Arabic letter in a token.
    assert!(has("<math><mi>\u{0633}</mi></math>"));
    // Nested inside structure.
    assert!(has(
        "<math><mfrac><mn>1</mn><mrow><mi>\u{0645}</mi></mrow></mfrac></math>"
    ));
    assert!(has(
        "<math><mtable><mtr><mtd><mtext>\u{FEB3}</mtext></mtd></mtr></mtable></math>"
    ));
    // Arabic Mathematical Alphabetic Symbols block.
    assert!(has("<math><mi>\u{1EE00}</mi></math>"));
    // Latin structure under dir=rtl is NOT flagged: the gate is Arabic
    // text, not RTL layout.
    assert!(!has("<math dir=\"rtl\"><msqrt><mi>x</mi></msqrt></math>"));
}
