//! SVG golden tests plus property-style sanity checks.
//!
//! Regenerate goldens with `UPDATE_GOLDEN=1 cargo test`, then eyeball the SVG
//! (and compare against Firefox rendering the same markup) before committing.

use formulary::{layout, parse, LayoutOptions, MathFont};

fn stix() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/STIXTwoMath-Regular.otf"
    ))
    .expect("vendored test font")
}

fn render_svg(mathml: &str, font_size: f32) -> String {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let tree = parse(mathml).unwrap();
    let laid = layout(&tree, &font, &LayoutOptions { font_size });
    formulary::svg::to_svg(&laid, &font)
}

fn check_golden(name: &str, mathml: &str) {
    let svg = render_svg(mathml, 32.0);
    let path = format!("{}/tests/golden/{name}.svg", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(format!("{}/tests/golden", env!("CARGO_MANIFEST_DIR"))).unwrap();
        std::fs::write(&path, &svg).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {path}; run with UPDATE_GOLDEN=1"));
    assert_eq!(svg, expected, "golden mismatch for {name} ({path})");
}

#[test]
fn golden_simple_row() {
    check_golden("simple_row", "<math><mi>x</mi><mo>+</mo><mn>2</mn></math>");
}

#[test]
fn golden_nested_mrow() {
    check_golden(
        "nested_mrow",
        "<math><mrow><mn>3</mn><mo>&#x2212;</mo><mrow><mi>a</mi><mi>b</mi></mrow></mrow></math>",
    );
}

#[test]
fn baseline_metrics_sane() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let tree = parse("<math><mi>x</mi><mo>+</mo><mn>2</mn></math>").unwrap();
    let laid = layout(&tree, &font, &LayoutOptions { font_size: 16.0 });
    assert!(laid.width > 0.0);
    assert!(laid.ascent > 0.0, "x-height ink must rise above baseline");
    assert!(laid.descent >= 0.0);
    // 'x', '+', '2' each produce one glyph.
    assert_eq!(laid.items.len(), 3);
}

#[test]
fn scaling_is_linear() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let tree = parse("<math><mi>y</mi><mo>=</mo><mi>m</mi><mi>x</mi></math>").unwrap();
    let at16 = layout(&tree, &font, &LayoutOptions { font_size: 16.0 });
    let at32 = layout(&tree, &font, &LayoutOptions { font_size: 32.0 });
    assert!((at32.width - 2.0 * at16.width).abs() < 1e-3);
    assert!((at32.ascent - 2.0 * at16.ascent).abs() < 1e-3);
    assert!((at32.descent - 2.0 * at16.descent).abs() < 1e-3);
}

#[test]
fn single_char_mi_italicizes() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let italic = layout(
        &parse("<math><mi>x</mi></math>").unwrap(),
        &font,
        &LayoutOptions { font_size: 16.0 },
    );
    let upright = layout(
        &parse("<math><mtext>x</mtext></math>").unwrap(),
        &font,
        &LayoutOptions { font_size: 16.0 },
    );
    assert_ne!(
        italic.items, upright.items,
        "single-char <mi> must map to the math-italic glyph"
    );
}

#[test]
fn multi_char_mi_stays_upright() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let mi = layout(
        &parse("<math><mi>sin</mi></math>").unwrap(),
        &font,
        &LayoutOptions { font_size: 16.0 },
    );
    let text = layout(
        &parse("<math><mtext>sin</mtext></math>").unwrap(),
        &font,
        &LayoutOptions { font_size: 16.0 },
    );
    assert_eq!(mi.items, text.items);
}

#[test]
fn rejects_font_without_math_table() {
    // A truncated/garbage buffer must fail as a face, and any parseable font
    // lacking MATH must report NoMathTable — proxied here by the error type
    // existing; real non-MATH font fixture comes later.
    assert!(MathFont::new(b"not a font", 0).is_err());
}

#[test]
fn rejects_non_math_root() {
    assert!(parse("<mrow><mi>x</mi></mrow>").is_err());
}

#[test]
fn unsupported_element_errors() {
    assert!(parse("<math><msqrt><mn>2</mn></msqrt></math>").is_err());
}

#[test]
fn mfrac_wrong_arity_errors() {
    assert!(parse("<math><mfrac><mn>1</mn></mfrac></math>").is_err());
}

#[test]
fn golden_mfrac_inline() {
    check_golden(
        "mfrac_inline",
        "<math><mfrac><mn>1</mn><mrow><mi>x</mi><mo>+</mo><mn>2</mn></mrow></mfrac></math>",
    );
}

#[test]
fn golden_mfrac_display() {
    check_golden(
        "mfrac_display",
        r#"<math display="block"><mfrac><mn>1</mn><mrow><mi>x</mi><mo>+</mo><mn>2</mn></mrow></mfrac></math>"#,
    );
}

#[test]
fn mfrac_geometry_sane() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let tree = parse("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>").unwrap();
    let laid = layout(&tree, &font, &LayoutOptions { font_size: 16.0 });

    // Exactly one rule (the fraction bar), spanning the full width.
    let rules: Vec<_> = laid
        .items
        .iter()
        .filter_map(|i| match *i {
            formulary::Item::Rule { x, y, w, h } => Some((x, y, w, h)),
            _ => None,
        })
        .collect();
    assert_eq!(rules.len(), 1);
    let (rx, ry, rw, rh) = rules[0];
    assert_eq!(rx, 0.0);
    assert!((rw - laid.width).abs() < 1e-3);
    assert!(rh > 0.0);
    // Bar sits above the baseline (on the math axis).
    assert!(ry < 0.0 && ry > -laid.ascent);

    // Fraction parts drop to script size in inline (text) style.
    let glyph_sizes: Vec<f32> = laid
        .items
        .iter()
        .filter_map(|i| match *i {
            formulary::Item::Glyph { size, .. } => Some(size),
            _ => None,
        })
        .collect();
    assert_eq!(glyph_sizes.len(), 2);
    assert!(glyph_sizes.iter().all(|&s| s < 16.0));

    // Display style keeps parts at full size and spreads them further apart.
    let display_tree =
        parse(r#"<math display="block"><mfrac><mn>1</mn><mn>2</mn></mfrac></math>"#).unwrap();
    let display = layout(&display_tree, &font, &LayoutOptions { font_size: 16.0 });
    let display_sizes: Vec<f32> = display
        .items
        .iter()
        .filter_map(|i| match *i {
            formulary::Item::Glyph { size, .. } => Some(size),
            _ => None,
        })
        .collect();
    assert!(display_sizes.iter().all(|&s| (s - 16.0).abs() < 1e-6));
    assert!(display.ascent + display.descent > laid.ascent + laid.descent);
}
