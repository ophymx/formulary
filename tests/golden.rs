//! SVG golden tests plus property-style sanity checks.
//!
//! Regenerate goldens with `UPDATE_GOLDEN=1 cargo test` and eyeball the SVG
//! before committing; `tools/browser-compare.py` renders each golden next to
//! a browser's rendering of the same markup for drift checks.

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
fn unknown_element_recovers_with_warning() {
    let root = parse("<math><menclose notation=\"box\"><mn>1</mn></menclose></math>").unwrap();
    assert_eq!(root.warnings.len(), 1);
    assert!(matches!(
        &root.warnings[0],
        formulary::Warning::UnknownElement { element } if element == "menclose"
    ));
    // The contents still render.
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let laid = layout(&root, &font, &LayoutOptions { font_size: 16.0 });
    assert_eq!(glyphs(&laid).len(), 1);
}

#[test]
fn mfrac_wrong_arity_falls_back_to_row() {
    let root = parse("<math><mfrac><mn>1</mn></mfrac></math>").unwrap();
    assert_eq!(root.warnings.len(), 1);
    // Spec fallback: the child renders as if in an mrow, no fraction bar.
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let laid = layout(&root, &font, &LayoutOptions { font_size: 16.0 });
    assert!(laid.items.iter().all(|i| matches!(i, formulary::Item::Glyph { .. })));
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
fn golden_msup() {
    check_golden("msup", "<math><msup><mi>x</mi><mn>2</mn></msup></math>");
}

#[test]
fn golden_msubsup() {
    check_golden(
        "msubsup",
        "<math><msubsup><mi>x</mi><mi>i</mi><mn>2</mn></msubsup><mo>+</mo><msub><mi>y</mi><mn>0</mn></msub></math>",
    );
}

fn glyphs(laid: &formulary::Layout) -> Vec<(f32, f32, f32)> {
    laid.items
        .iter()
        .filter_map(|i| match *i {
            formulary::Item::Glyph { x, y, size, .. } => Some((x, y, size)),
            _ => None,
        })
        .collect()
}

#[test]
fn scripts_geometry_sane() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    let sup = layout(
        &parse("<math><msup><mi>x</mi><mn>2</mn></msup></math>").unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&sup);
    assert_eq!(g.len(), 2);
    let (base, script) = (g[0], g[1]);
    assert!(script.1 < 0.0, "superscript baseline must sit above the main one");
    assert!(script.2 < base.2, "superscript must drop to script size");
    assert!(script.0 > base.0, "superscript attaches after the base");
    let bare = layout(&parse("<math><mi>x</mi></math>").unwrap(), &font, &opts);
    assert!(sup.ascent > bare.ascent);
    assert!(sup.width > bare.width);

    let sub = layout(
        &parse("<math><msub><mi>x</mi><mi>i</mi></msub></math>").unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&sub);
    assert_eq!(g.len(), 2);
    assert!(g[1].1 > 0.0, "subscript baseline must sit below the main one");
    assert!(sub.descent > bare.descent);

    // msubsup: sub and sup attach at the same base, offset only by italic
    // correction, and stay apart vertically.
    let both = layout(
        &parse("<math><msubsup><mi>x</mi><mi>i</mi><mn>2</mn></msubsup></math>").unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&both);
    assert_eq!(g.len(), 3);
    // Display-list order is base, superscript, subscript. The subscript
    // tucks left of the superscript by the base's italic correction.
    let (sup_g, sub_g) = (g[1], g[2]);
    assert!(sub_g.0 <= sup_g.0);
    assert!(sup_g.0 - sub_g.0 < 1.0, "italic x has only a slight slant");
    assert!(sup_g.1 < 0.0 && sub_g.1 > 0.0);

    // Nested superscripts reach script-script size, smaller than script size.
    let nested = layout(
        &parse("<math><msup><mi>x</mi><msup><mn>2</mn><mn>2</mn></msup></msup></math>").unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&nested);
    assert_eq!(g.len(), 3);
    assert!(g[2].2 < g[1].2 && g[1].2 < g[0].2);
}

#[test]
fn golden_msqrt() {
    check_golden(
        "msqrt",
        "<math><msqrt><mi>x</mi><mo>+</mo><mn>1</mn></msqrt></math>",
    );
}

#[test]
fn golden_msqrt_tall() {
    check_golden(
        "msqrt_tall",
        r#"<math display="block"><msqrt><mfrac><mn>1</mn><mi>x</mi></mfrac></msqrt></math>"#,
    );
}

#[test]
fn golden_mroot() {
    check_golden("mroot", "<math><mroot><mi>x</mi><mn>3</mn></mroot></math>");
}

#[test]
fn radical_geometry_sane() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    let plain = layout(
        &parse("<math><mrow><mi>x</mi><mo>+</mo><mn>1</mn></mrow></math>").unwrap(),
        &font,
        &opts,
    );
    let sqrt = layout(
        &parse("<math><msqrt><mi>x</mi><mo>+</mo><mn>1</mn></msqrt></math>").unwrap(),
        &font,
        &opts,
    );
    // Radical glyph before the radicand, overbar above it.
    assert_eq!(glyphs(&sqrt).len(), 4);
    assert!(sqrt.width > plain.width);
    assert!(sqrt.ascent > plain.ascent, "bar and gap must add height");
    let rules: Vec<_> = sqrt
        .items
        .iter()
        .filter_map(|i| match *i {
            formulary::Item::Rule { x, y, w, h, .. } => Some((x, y, w, h)),
            _ => None,
        })
        .collect();
    assert_eq!(rules.len(), 1);
    let (rx, ry, rw, _) = rules[0];
    assert!((rw - plain.width).abs() < 1e-3, "bar spans the radicand");
    assert!(ry < -plain.ascent, "bar sits above the radicand's ink");
    assert!(rx > 0.0, "bar starts after the radical glyph");

    // A tall radicand must select a taller radical variant: the radical
    // glyph's ink must reach from the bar down past the radicand's descent.
    let tall = layout(
        &parse("<math><msqrt><mfrac><mn>1</mn><mi>x</mi></mfrac></msqrt></math>").unwrap(),
        &font,
        &opts,
    );
    let sqrt_radical_id = match sqrt.items[0] {
        formulary::Item::Glyph { id, .. } => id,
        _ => panic!("radical glyph first"),
    };
    let tall_radical_id = match tall.items[0] {
        formulary::Item::Glyph { id, .. } => id,
        _ => panic!("radical glyph first"),
    };
    assert_ne!(
        sqrt_radical_id, tall_radical_id,
        "taller content must pick a taller radical variant"
    );
    assert!(tall.ascent + tall.descent > sqrt.ascent + sqrt.descent);

    // mroot: degree glyph is present, small, and raised above the baseline.
    let root = layout(
        &parse("<math><mroot><mi>x</mi><mn>3</mn></mroot></math>").unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&root);
    assert_eq!(g.len(), 3, "degree, radical, radicand");
    let degree = g[0];
    assert!(degree.2 < 16.0 * 0.6, "degree renders at script-script size");
    assert!(degree.1 < 0.0, "degree is raised");
    let sqrt_x = layout(
        &parse("<math><msqrt><mi>x</mi></msqrt></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(root.width > sqrt_x.width, "degree widens the box");
}

#[test]
fn golden_spacing_and_style() {
    check_golden(
        "spacing_and_style",
        r#"<math><mi>a</mi><mspace width="1em"/><mstyle displaystyle="true"><mfrac><mn>1</mn><mn>2</mn></mfrac></mstyle></math>"#,
    );
}

#[test]
fn golden_quadratic_formula() {
    check_golden(
        "quadratic_formula",
        r#"<math display="block"><mi>x</mi><mo>=</mo><mfrac><mrow><mo>&#x2212;</mo><mi>b</mi><mo>&#xB1;</mo><msqrt><msup><mi>b</mi><mn>2</mn></msup><mo>&#x2212;</mo><mn>4</mn><mi>a</mi><mi>c</mi></msqrt></mrow><mrow><mn>2</mn><mi>a</mi></mrow></mfrac></math>"#,
    );
}

#[test]
fn mspace_occupies_space() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let spaced = layout(
        &parse(r#"<math><mi>a</mi><mspace width="1em"/><mi>b</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    let tight = layout(
        &parse("<math><mi>a</mi><mi>b</mi></math>").unwrap(),
        &font,
        &opts,
    );
    assert!((spaced.width - tight.width - 16.0).abs() < 1e-3);

    let tall = layout(
        &parse(r#"<math><mspace width="4px" height="20px" depth="8px"/></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert_eq!((tall.width, tall.ascent, tall.descent), (4.0, 20.0, 8.0));
    assert!(tall.items.is_empty());
}

#[test]
fn mstyle_switches_style() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    // displaystyle=true inside inline math must match block-math layout.
    let styled = layout(
        &parse(r#"<math><mstyle displaystyle="true"><mfrac><mn>1</mn><mn>2</mn></mfrac></mstyle></math>"#).unwrap(),
        &font,
        &opts,
    );
    let block = layout(
        &parse(r#"<math display="block"><mfrac><mn>1</mn><mn>2</mn></mfrac></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert_eq!(styled, block);

    // scriptlevel="+1" shrinks glyphs; an absolute value works too.
    let bumped = layout(
        &parse(r#"<math><mstyle scriptlevel="+1"><mi>x</mi></mstyle></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!(glyphs(&bumped)[0].2 < 16.0);
    let set = layout(
        &parse(r#"<math><mstyle scriptlevel="2"><mi>x</mi></mstyle></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!(glyphs(&set)[0].2 < glyphs(&bumped)[0].2);
}

#[test]
fn mphantom_spaces_without_ink() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let phantom = layout(
        &parse("<math><mphantom><mfrac><mn>1</mn><mn>2</mn></mfrac></mphantom></math>").unwrap(),
        &font,
        &opts,
    );
    let visible = layout(
        &parse("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(phantom.items.is_empty());
    assert_eq!(
        (phantom.width, phantom.ascent, phantom.descent),
        (visible.width, visible.ascent, visible.descent)
    );
}

#[test]
fn mpadded_overrides_box() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let natural = layout(&parse("<math><mi>x</mi></math>").unwrap(), &font, &opts);
    let padded = layout(
        &parse(r#"<math><mpadded width="200%" height="30px" voffset="5px"><mi>x</mi></mpadded></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    assert!((padded.width - 2.0 * natural.width).abs() < 1e-3);
    assert_eq!(padded.ascent, 30.0);
    // voffset moves the ink up without changing the reported box.
    assert!((glyphs(&padded)[0].1 - (-5.0)).abs() < 1e-6);

    // Invalid length values fall back to the natural dimension.
    let bad = layout(
        &parse(r#"<math><mpadded width="banana"><mi>x</mi></mpadded></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert_eq!(bad.width, natural.width);
}

#[test]
fn operator_spacing_from_dictionary() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 18.0 }; // 1/18em == 1 unit

    // Infix '+' carries 4/18 em on each side.
    let sum = layout(
        &parse("<math><mi>a</mi><mo>+</mo><mi>b</mi></math>").unwrap(),
        &font,
        &opts,
    );
    let bare = layout(
        &parse("<math><mi>a</mi><mtext>+</mtext><mi>b</mi></math>").unwrap(),
        &font,
        &opts,
    );
    assert!((sum.width - bare.width - 8.0).abs() < 1e-3);

    // Leading '−' is prefix: no space on either side.
    let neg = layout(
        &parse("<math><mo>&#x2212;</mo><mi>b</mi></math>").unwrap(),
        &font,
        &opts,
    );
    let neg_text = layout(
        &parse("<math><mtext>&#x2212;</mtext><mi>b</mi></math>").unwrap(),
        &font,
        &opts,
    );
    assert!((neg.width - neg_text.width).abs() < 1e-3);

    // Explicit form attribute overrides position inference.
    let forced_infix = layout(
        &parse(r#"<math><mo form="infix">&#x2212;</mo><mi>b</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!(forced_infix.width > neg.width);

    // lspace/rspace attributes override the dictionary.
    let custom = layout(
        &parse(r#"<math><mi>a</mi><mo lspace="0" rspace="0">+</mo><mi>b</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!((custom.width - bare.width).abs() < 1e-3);

    // Space-like siblings don't affect form inference: '−' before an mspace
    // and an identifier is still prefix.
    let with_space = layout(
        &parse(r#"<math><mtext>pad</mtext><mo>&#x2212;</mo><mi>b</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    let with_space_text = layout(
        &parse(r#"<math><mtext>pad</mtext><mtext>&#x2212;</mtext><mi>b</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!((with_space.width - with_space_text.width).abs() < 1e-3);
}

#[test]
fn golden_stretchy_parens() {
    check_golden(
        "stretchy_parens",
        r#"<math display="block"><mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo></math>"#,
    );
}

#[test]
fn golden_integral_display() {
    check_golden(
        "integral_display",
        r#"<math display="block"><mo>&#x222B;</mo><msup><mi>x</mi><mn>2</mn></msup><mspace width="0.17em"/><mi>d</mi><mi>x</mi></math>"#,
    );
}

#[test]
fn stretchy_parens_cover_content() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    let small = layout(
        &parse("<math><mo>(</mo><mi>x</mi><mo>)</mo></math>").unwrap(),
        &font,
        &opts,
    );
    let tall = layout(
        &parse("<math><mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo></math>").unwrap(),
        &font,
        &opts,
    );
    // Around tall content the paren must be a different, taller glyph.
    let paren_of = |l: &formulary::Layout| match l.items[0] {
        formulary::Item::Glyph { id, .. } => id,
        _ => panic!("expected glyph"),
    };
    assert_ne!(paren_of(&small), paren_of(&tall));
    // Symmetric fences cover both extremes of the content.
    let content_ascent_covered = tall.ascent;
    let frac_alone = layout(
        &parse("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(content_ascent_covered >= frac_alone.ascent - 1e-3);
    assert!(tall.descent >= frac_alone.descent - 1e-3);

    // Enormous content exhausts pre-drawn variants and forces assembly:
    // the fence becomes multiple stacked glyph parts.
    let huge = layout(
        &parse(r#"<math><mo>(</mo><mpadded height="120px" depth="120px"><mi>x</mi></mpadded><mo>)</mo></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let glyph_count = glyphs(&huge).len();
    assert!(
        glyph_count > 3,
        "expected assembled fences, got {glyph_count} glyphs"
    );
    assert!(huge.ascent + huge.descent >= 240.0 - 1e-3);
}

#[test]
fn largeop_grows_in_display_style() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let inline = layout(
        &parse("<math><mo>&#x222B;</mo><mi>x</mi></math>").unwrap(),
        &font,
        &opts,
    );
    let display = layout(
        &parse(r#"<math display="block"><mo>&#x222B;</mo><mi>x</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!(display.ascent + display.descent > 1.5 * (inline.ascent + inline.descent));
    // largeop="false" opts out.
    let opted_out = layout(
        &parse(r#"<math display="block"><mo largeop="false">&#x222B;</mo><mi>x</mi></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    assert!(opted_out.ascent + opted_out.descent < display.ascent + display.descent);
}

#[test]
fn golden_sum_limits() {
    check_golden(
        "sum_limits",
        r#"<math display="block"><munderover><mo>&#x2211;</mo><mrow><mi>n</mi><mo>=</mo><mn>1</mn></mrow><mi>N</mi></munderover><msup><mi>n</mi><mn>2</mn></msup></math>"#,
    );
}

#[test]
fn golden_accent_and_arrow() {
    check_golden(
        "accent_and_arrow",
        r#"<math><mover accent="true"><mi>x</mi><mo>&#x302;</mo></mover><mo>+</mo><mover><mrow><mi>a</mi><mi>b</mi><mi>c</mi></mrow><mo>&#x2192;</mo></mover></math>"#,
    );
}

#[test]
fn movablelimits_and_limit_placement() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let markup = "<math{}><munder><mo>&#x2211;</mo><mrow><mi>n</mi><mo>=</mo><mn>1</mn></mrow></munder></math>";

    // Display style: limits below, centered — box no wider than needed,
    // descent grows well past the base's.
    let display = layout(
        &parse(&markup.replace("{}", r#" display="block""#)).unwrap(),
        &font,
        &opts,
    );
    // Inline: movablelimits turns it into a subscript — wider, shallower.
    let inline = layout(&parse(&markup.replace("{}", "")).unwrap(), &font, &opts);
    assert!(inline.width > display.width);
    assert!(display.descent > inline.descent);

    // movablelimits="false" keeps limits underneath even inline.
    let forced = layout(
        &parse("<math><munder><mo movablelimits=\"false\">&#x2211;</mo><mrow><mi>n</mi><mo>=</mo><mn>1</mn></mrow></munder></math>")
            .unwrap(),
        &font,
        &opts,
    );
    assert!(forced.descent > inline.descent);

    // In display style the ∑ base itself takes its large variant.
    let inline_sum = layout(&parse("<math><mo>&#x2211;</mo></math>").unwrap(), &font, &opts);
    assert!(display.ascent + display.descent > inline_sum.ascent + inline_sum.descent);
}

#[test]
fn accents_keep_size_and_position() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let hat = layout(
        &parse(r#"<math><mover accent="true"><mi>x</mi><mo>&#x302;</mo></mover></math>"#).unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&hat);
    assert_eq!(g.len(), 2);
    // Accent stays at full size (no script shrink)...
    assert_eq!(g[1].2, 16.0);
    // ...and raises the box above the bare letter.
    let bare = layout(&parse("<math><mi>x</mi></math>").unwrap(), &font, &opts);
    assert!(hat.ascent > bare.ascent);
    // Non-accent mover drops the script a level.
    let plain = layout(
        &parse("<math><mover><mi>x</mi><mo>&#x302;</mo></mover></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(glyphs(&plain)[1].2 < 16.0);
}

#[test]
fn horizontal_arrow_stretches_over_base() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let narrow = layout(
        &parse("<math><mover><mi>a</mi><mo>&#x2192;</mo></mover></math>").unwrap(),
        &font,
        &opts,
    );
    let wide = layout(
        &parse("<math><mover><mrow><mi>a</mi><mi>b</mi><mi>c</mi><mi>d</mi></mrow><mo>&#x2192;</mo></mover></math>")
            .unwrap(),
        &font,
        &opts,
    );
    // The arrow must track the base width: total width equals the base row's
    // width, and the arrow accounts for (nearly) all of it.
    let base_row = layout(
        &parse("<math><mrow><mi>a</mi><mi>b</mi><mi>c</mi><mi>d</mi></mrow></math>").unwrap(),
        &font,
        &opts,
    );
    assert!((wide.width - base_row.width).abs() < 1e-3);
    assert!(wide.width > narrow.width * 2.0);
}

#[test]
fn golden_integral_bounds() {
    check_golden(
        "integral_bounds",
        r#"<math display="block"><msubsup><mo>&#x222B;</mo><mn>0</mn><mn>1</mn></msubsup><mi>x</mi><mspace width="0.17em"/><mi>d</mi><mi>x</mi></math>"#,
    );
}

#[test]
fn italic_correction_tucks_integral_bounds() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let laid = layout(
        &parse(r#"<math display="block"><msubsup><mo>&#x222B;</mo><mn>0</mn><mn>1</mn></msubsup></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&laid);
    assert_eq!(g.len(), 3, "integral, superscript, subscript");
    let (sup, sub) = (g[1], g[2]);
    // The big ∫ is heavily slanted: the lower bound tucks well left of the
    // upper bound.
    assert!(
        sup.0 - sub.0 > 2.0,
        "expected a pronounced tuck, got {}",
        sup.0 - sub.0
    );
}

#[test]
fn embellished_operators_space_and_stretch() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 18.0 }; // 1/18 em == 1 unit

    // Spacing: an msup-wrapped '+' spaces like a bare '+' (4/18 em per side).
    let wrapped = layout(
        &parse("<math><mi>a</mi><msup><mo>+</mo><mn>1</mn></msup><mi>b</mi></math>").unwrap(),
        &font,
        &opts,
    );
    let text_wrapped = layout(
        &parse("<math><mi>a</mi><msup><mtext>+</mtext><mn>1</mn></msup><mi>b</mi></math>")
            .unwrap(),
        &font,
        &opts,
    );
    assert!((wrapped.width - text_wrapped.width - 8.0).abs() < 1e-3);

    // Stretching: a squared closing paren still stretches over the fraction.
    let laid = layout(
        &parse("<math><mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><msup><mo>)</mo><mn>2</mn></msup></math>")
            .unwrap(),
        &font,
        &opts,
    );
    let frac_alone = layout(
        &parse("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>").unwrap(),
        &font,
        &opts,
    );
    // Both fences (first and second-to-last glyphs) cover the fraction.
    let g = glyphs(&laid);
    let open = g[0];
    assert!(laid.ascent >= frac_alone.ascent - 1e-3);
    // The open and close fences are the same stretched glyph at mirrored
    // heights; the close one sits inside the msup, before its superscript.
    let close = g[g.len() - 2];
    assert!((open.1 - close.1).abs() < 1e-3, "fences share vertical placement");
}

#[test]
fn scripts_use_ssty_alternates() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let top = layout(
        &parse("<math><mo>&#x2032;</mo></math>").unwrap(),
        &font,
        &opts,
    );
    let scripted = layout(
        &parse("<math><msup><mi>f</mi><mo>&#x2032;</mo></msup></math>").unwrap(),
        &font,
        &opts,
    );
    let id = |l: &formulary::Layout, i: usize| match l.items[i] {
        formulary::Item::Glyph { id, .. } => id,
        _ => panic!("expected glyph"),
    };
    assert_ne!(
        id(&top, 0),
        id(&scripted, 1),
        "script prime must swap to its ssty alternate"
    );
}

#[test]
fn golden_matrix() {
    check_golden(
        "matrix",
        r#"<math><mo>(</mo><mtable><mtr><mtd><mi>a</mi></mtd><mtd><mi>b</mi></mtd></mtr><mtr><mtd><mi>c</mi></mtd><mtd><mi>d</mi></mtd></mtr></mtable><mo>)</mo></math>"#,
    );
}

#[test]
fn mtable_geometry_sane() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    let two_by_two = layout(
        &parse("<math><mtable><mtr><mtd><mi>a</mi></mtd><mtd><mn>42</mn></mtd></mtr><mtr><mtd><mn>100</mn></mtd><mtd><mi>d</mi></mtd></mtr></mtable></math>")
            .unwrap(),
        &font,
        &opts,
    );
    // Two rows stack: the box is taller than a single row and roughly
    // centered on the math axis (ascent > descent > 0).
    let single = layout(
        &parse("<math><mtable><mtr><mtd><mi>a</mi></mtd></mtr></mtable></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(two_by_two.ascent + two_by_two.descent
        > 1.8 * (single.ascent + single.descent));
    assert!(two_by_two.descent > 0.0);
    assert!(two_by_two.ascent > two_by_two.descent, "axis sits above baseline");

    // Cells center within their column: 'a' (narrow, over wide '100') is
    // indented; the first row's glyph starts right of the second row's.
    // Glyph order: a, 4, 2 (row 1), 1, 0, 0, d (row 2).
    let g = glyphs(&two_by_two);
    assert_eq!(g.len(), 7);
    let a_x = g[0].0;
    let hundred_x = g[3].0;
    assert!(a_x > hundred_x);

    // Rows are baseline-aligned per row, rows stack downward.
    assert!(g[0].1 < g[3].1);

    // Fences stretch to cover the table.
    let fenced = layout(
        &parse("<math><mo>(</mo><mtable><mtr><mtd><mi>a</mi></mtd></mtr><mtr><mtd><mi>b</mi></mtd></mtr></mtable><mo>)</mo></math>")
            .unwrap(),
        &font,
        &opts,
    );
    let paren_small = layout(&parse("<math><mo>(</mo><mi>a</mi><mo>)</mo></math>").unwrap(), &font, &opts);
    assert!(fenced.ascent + fenced.descent > paren_small.ascent + paren_small.descent);

    // Ragged rows are tolerated.
    let ragged = layout(
        &parse("<math><mtable><mtr><mtd><mi>a</mi></mtd><mtd><mi>b</mi></mtd></mtr><mtr><mtd><mi>c</mi></mtd></mtr></mtable></math>")
            .unwrap(),
        &font,
        &opts,
    );
    assert_eq!(glyphs(&ragged).len(), 3);

    // Cells lay out in text style: displaystyle constructs shrink.
    let in_table = layout(
        &parse(r#"<math display="block"><mtable><mtr><mtd><mfrac><mn>1</mn><mn>2</mn></mfrac></mtd></mtr></mtable></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let sizes: Vec<f32> = glyphs(&in_table).iter().map(|g| g.2).collect();
    assert!(sizes.iter().all(|&s| s < 16.0), "table cell fraction is text-style");
}

#[test]
fn mtable_stray_children_are_wrapped() {
    // Anonymous fixup: stray children become rows/cells, with warnings.
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    for markup in [
        "<math><mtable><mi>x</mi></mtable></math>",
        "<math><mtable><mtr><mi>x</mi></mtr></mtable></math>",
    ] {
        let root = parse(markup).unwrap();
        assert_eq!(root.warnings.len(), 1, "{markup}");
        let laid = layout(&root, &font, &LayoutOptions { font_size: 16.0 });
        assert_eq!(glyphs(&laid).len(), 1, "{markup}");
    }
}

#[test]
fn golden_multiscripts() {
    // Riemann tensor-ish: base R, postscripts ρ (sup) then σμν pattern via
    // pairs, prescripts j/k.
    check_golden(
        "multiscripts",
        r#"<math><mmultiscripts><mi>R</mi><mi>i</mi><none/><none/><mi>j</mi><mprescripts/><mi>k</mi><mi>l</mi></mmultiscripts></math>"#,
    );
}

#[test]
fn multiscripts_geometry_sane() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    // One postscript pair matches msubsup's layout.
    let multi = layout(
        &parse("<math><mmultiscripts><mi>x</mi><mi>i</mi><mn>2</mn></mmultiscripts></math>")
            .unwrap(),
        &font,
        &opts,
    );
    let subsup = layout(
        &parse("<math><msubsup><mi>x</mi><mi>i</mi><mn>2</mn></msubsup></math>").unwrap(),
        &font,
        &opts,
    );
    // Same geometry; item order may differ (sub/sup emission order).
    assert_eq!(
        (multi.width, multi.ascent, multi.descent),
        (subsup.width, subsup.ascent, subsup.descent)
    );
    let mut mg = glyphs(&multi);
    let mut sg = glyphs(&subsup);
    let key = |g: &(f32, f32, f32)| (g.0.to_bits(), g.1.to_bits());
    mg.sort_by_key(key);
    sg.sort_by_key(key);
    assert_eq!(mg, sg);

    // Prescripts land left of the base; script baselines align across
    // columns (all sups share y, all subs share y).
    let tensor = layout(
        &parse("<math><mmultiscripts><mi>R</mi><mi>i</mi><none/><none/><mi>j</mi><mprescripts/><mi>k</mi><mi>l</mi></mmultiscripts></math>")
            .unwrap(),
        &font,
        &opts,
    );
    // Glyphs: base R first (placement order: pre pair k/l, base, posts) —
    // actually pre places before base in the item list.
    let g = glyphs(&tensor);
    assert_eq!(g.len(), 5, "k, l, R, i, j");
    let (k, l, r, i, j) = (g[0], g[1], g[2], g[3], g[4]);
    assert!(k.0 < r.0 && l.0 < r.0, "prescripts sit left of the base");
    assert!(i.0 > r.0 && j.0 > r.0, "postscripts sit right of the base");
    assert!((k.1 - i.1).abs() < 1e-3, "subscript baselines align");
    assert!((l.1 - j.1).abs() < 1e-3, "superscript baselines align");
    assert!(k.1 > 0.0 && l.1 < 0.0);

    // Structure errors.
    let odd = parse("<math><mmultiscripts><mi>x</mi><mi>i</mi></mmultiscripts></math>").unwrap();
    assert!(!odd.warnings.is_empty());
    assert!(!parse(
        "<math><mmultiscripts><mi>x</mi><mprescripts/><mi>a</mi><mi>b</mi><mprescripts/></mmultiscripts></math>"
    )
    .unwrap()
    .warnings
    .is_empty());
    assert!(!parse("<math><mmultiscripts></mmultiscripts></math>")
        .unwrap()
        .warnings
        .is_empty());
}

#[test]
fn mfenced_desugars_to_fenced_row() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let fenced = layout(
        &parse("<math><mfenced><mi>a</mi><mi>b</mi></mfenced></math>").unwrap(),
        &font,
        &opts,
    );
    let explicit = layout(
        &parse("<math><mrow><mo>(</mo><mi>a</mi><mo>,</mo><mi>b</mi><mo>)</mo></mrow></math>")
            .unwrap(),
        &font,
        &opts,
    );
    assert_eq!(fenced, explicit);

    // Custom fences and empty separators.
    let bracketed = layout(
        &parse(r#"<math><mfenced open="[" close="]" separators=""><mi>a</mi><mi>b</mi></mfenced></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let explicit2 = layout(
        &parse("<math><mrow><mo>[</mo><mi>a</mi><mi>b</mi><mo>]</mo></mrow></math>").unwrap(),
        &font,
        &opts,
    );
    assert_eq!(bracketed, explicit2);
}

#[test]
fn golden_binomial() {
    check_golden(
        "binomial",
        r#"<math><mo>(</mo><mfrac linethickness="0"><mi>n</mi><mi>k</mi></mfrac><mo>)</mo></math>"#,
    );
}

#[test]
fn linethickness_variants() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    // linethickness="0": no bar at all, parts still stacked.
    let binom = layout(
        &parse(r#"<math><mfrac linethickness="0"><mi>n</mi><mi>k</mi></mfrac></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!(binom
        .items
        .iter()
        .all(|i| matches!(i, formulary::Item::Glyph { .. })));
    let g = glyphs(&binom);
    assert_eq!(g.len(), 2);
    assert!(g[0].1 < 0.0 && g[1].1 > 0.0, "still a vertical stack");

    // A thick bar is thicker than the default.
    let rule_h = |l: &formulary::Layout| {
        l.items
            .iter()
            .find_map(|i| match *i {
                formulary::Item::Rule { h, .. } => Some(h),
                _ => None,
            })
            .unwrap()
    };
    let default = layout(
        &parse("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>").unwrap(),
        &font,
        &opts,
    );
    let thick = layout(
        &parse(r#"<math><mfrac linethickness="4px"><mn>1</mn><mn>2</mn></mfrac></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!((rule_h(&thick) - 4.0).abs() < 1e-3);
    assert!(rule_h(&thick) > rule_h(&default));
}

#[test]
fn displaystyle_attribute_on_math() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let attr = layout(
        &parse(r#"<math displaystyle="true"><mfrac><mn>1</mn><mn>2</mn></mfrac></math>"#).unwrap(),
        &font,
        &opts,
    );
    let block = layout(
        &parse(r#"<math display="block"><mfrac><mn>1</mn><mn>2</mn></mfrac></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert_eq!(attr, block);
    // And the reverse: block math demoted to text style.
    let demoted = layout(
        &parse(r#"<math display="block" displaystyle="false"><mfrac><mn>1</mn><mn>2</mn></mfrac></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let inline = layout(
        &parse("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>").unwrap(),
        &font,
        &opts,
    );
    assert_eq!(demoted, inline);
}

#[test]
fn mathvariant_maps_tokens() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let gid = |markup: &str| {
        let laid = layout(&parse(markup).unwrap(), &font, &opts);
        match laid.items[0] {
            formulary::Item::Glyph { id, .. } => id,
            _ => panic!("expected glyph"),
        }
    };

    // mathvariant="normal" suppresses the single-char auto-italic.
    assert_eq!(
        gid(r#"<math><mi mathvariant="normal">x</mi></math>"#),
        gid("<math><mtext>x</mtext></math>")
    );
    // bold and double-struck map into the styled blocks.
    let plain = gid("<math><mtext>x</mtext></math>");
    let italic = gid("<math><mi>x</mi></math>");
    let bold = gid(r#"<math><mi mathvariant="bold">x</mi></math>"#);
    assert!(bold != plain && bold != italic);
    let bb_r = gid(r#"<math><mi mathvariant="double-struck">R</mi></math>"#);
    let literal_bb_r = gid("<math><mi>&#x211D;</mi></math>");
    assert_eq!(bb_r, literal_bb_r, "R maps through the Letterlike hole to ℝ");
    // Digits exist in bold but not italic: italic leaves them alone.
    assert_eq!(
        gid(r#"<math><mn mathvariant="italic">5</mn></math>"#),
        gid("<math><mn>5</mn></math>")
    );
}

#[test]
fn minsize_maxsize_clamp_stretching() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };

    // minsize forces a tall fence even around short content.
    let forced = layout(
        &parse(r#"<math><mo minsize="3em">(</mo><mi>x</mi><mo minsize="3em">)</mo></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let natural = layout(
        &parse("<math><mo>(</mo><mi>x</mi><mo>)</mo></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(forced.ascent + forced.descent >= 44.0); // ~3em at 16px, minus slack
    assert!(forced.ascent + forced.descent > 1.5 * (natural.ascent + natural.descent));

    // maxsize keeps a fence small around tall content.
    let capped = layout(
        &parse(r#"<math><mo maxsize="1em">(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo maxsize="1em">)</mo></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let uncapped = layout(
        &parse("<math><mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo></math>").unwrap(),
        &font,
        &opts,
    );
    // The capped fence must not drive the box taller than the fraction does.
    let frac = layout(
        &parse("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(capped.ascent + capped.descent <= frac.ascent + frac.descent + 1e-3);
    assert!(uncapped.ascent + uncapped.descent > capped.ascent + capped.descent);
}

#[test]
fn columnalign_aligns_cells() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let markup = |align: &str| {
        format!(
            r#"<math><mtable columnalign="{align}"><mtr><mtd><mi>x</mi></mtd></mtr><mtr><mtd><mn>100</mn></mtd></mtr></mtable></math>"#
        )
    };
    let x_of = |align: &str| glyphs(&layout(&parse(&markup(align)).unwrap(), &font, &opts))[0].0;
    let left = x_of("left");
    let center = x_of("center");
    let right = x_of("right");
    assert!(left < center && center < right);
}

#[test]
fn golden_styled() {
    check_golden(
        "styled",
        r##"<math><merror><mi>x</mi></merror><mo>+</mo><mfrac mathcolor="blue" mathbackground="#eee"><mn>1</mn><mn>2</mn></mfrac><mo>+</mo><mi mathsize="2em">y</mi></math>"##,
    );
}

fn item_color(item: &formulary::Item) -> Option<formulary::Color> {
    match *item {
        formulary::Item::Glyph { color, .. } | formulary::Item::Rule { color, .. } => color,
        formulary::Item::Background { color, .. } => Some(color),
        _ => None,
    }
}

#[test]
fn mathcolor_inherits_and_paints() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let laid = layout(
        &parse(r##"<math mathcolor="red"><mfrac><mn>1</mn><mi mathcolor="#00f">x</mi></mfrac></math>"##)
            .unwrap(),
        &font,
        &opts,
    );
    let red = formulary::Color::rgb(255, 0, 0);
    let blue = formulary::Color::rgb(0, 0, 255);
    let colors: Vec<_> = laid.items.iter().map(item_color).collect();
    // numerator glyph and fraction bar inherit red; the mi overrides to blue.
    assert!(colors.contains(&Some(red)));
    assert!(colors.contains(&Some(blue)));
    assert!(!colors.contains(&None));

    // Unstyled markup stays color-free (consumer text color).
    let plain = layout(
        &parse("<math><mfrac><mn>1</mn><mi>x</mi></mfrac></math>").unwrap(),
        &font,
        &opts,
    );
    assert!(plain.items.iter().all(|i| item_color(i).is_none()));
}

#[test]
fn mathbackground_paints_behind() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let laid = layout(
        &parse(r#"<math><mi mathbackground="yellow">x</mi><mi>y</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    // Background comes before the glyph it sits behind (painter's order).
    let bg_index = laid
        .items
        .iter()
        .position(|i| matches!(i, formulary::Item::Background { .. }))
        .expect("background item");
    let x_glyph = laid
        .items
        .iter()
        .position(|i| matches!(i, formulary::Item::Glyph { .. }))
        .unwrap();
    assert!(bg_index < x_glyph);
    let formulary::Item::Background { w, h, color, .. } = laid.items[bg_index] else {
        unreachable!()
    };
    assert_eq!(color, formulary::Color::rgb(255, 255, 0));
    assert!(w > 0.0 && h > 0.0);
}

#[test]
fn mathsize_rescales_subtree() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let laid = layout(
        &parse(r#"<math><mi>x</mi><mi mathsize="2em">x</mi><mi mathsize="150%">x</mi></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let g = glyphs(&laid);
    assert_eq!(g[0].2, 16.0);
    assert!((g[1].2 - 32.0).abs() < 1e-3);
    assert!((g[2].2 - 24.0).abs() < 1e-3);

    // A whole subtree rescales: scripts inside a mathsize scope shrink
    // relative to the enlarged size.
    let scripted = layout(
        &parse(r#"<math><msup mathsize="2em"><mi>x</mi><mn>2</mn></msup></math>"#).unwrap(),
        &font,
        &opts,
    );
    let base_scripted = layout(
        &parse("<math><msup><mi>x</mi><mn>2</mn></msup></math>").unwrap(),
        &font,
        &opts,
    );
    let sg = glyphs(&scripted);
    let bg = glyphs(&base_scripted);
    assert!((sg[0].2 - 2.0 * bg[0].2).abs() < 1e-3);
    assert!((sg[1].2 - 2.0 * bg[1].2).abs() < 1e-3);
}

#[test]
fn global_displaystyle_and_scriptlevel() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    // displaystyle directly on mfrac, no mstyle wrapper needed.
    let on_frac = layout(
        &parse(r#"<math><mfrac displaystyle="true"><mn>1</mn><mn>2</mn></mfrac></math>"#).unwrap(),
        &font,
        &opts,
    );
    let via_mstyle = layout(
        &parse(r#"<math><mstyle displaystyle="true"><mfrac><mn>1</mn><mn>2</mn></mfrac></mstyle></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    assert_eq!(on_frac, via_mstyle);
    // scriptlevel directly on a token.
    let leveled = layout(
        &parse(r#"<math><mi scriptlevel="2">x</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    assert!(glyphs(&leveled)[0].2 < 16.0 * 0.6);
}

#[test]
fn styled_operator_keeps_spacing_and_stretch() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 18.0 }; // 1/18 em == 1 unit

    // A colored '+' still gets its dictionary spacing.
    let colored = layout(
        &parse(r#"<math><mi>a</mi><mo mathcolor="red">+</mo><mi>b</mi></math>"#).unwrap(),
        &font,
        &opts,
    );
    let plain = layout(
        &parse("<math><mi>a</mi><mo>+</mo><mi>b</mi></math>").unwrap(),
        &font,
        &opts,
    );
    assert!((colored.width - plain.width).abs() < 1e-3);

    // A colored fence still stretches, and the stretched glyph is colored.
    let fenced = layout(
        &parse(r#"<math><mo mathcolor="red">(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo></math>"#)
            .unwrap(),
        &font,
        &opts,
    );
    let plain_fenced = layout(
        &parse("<math><mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo></math>").unwrap(),
        &font,
        &opts,
    );
    assert_eq!(fenced.ascent, plain_fenced.ascent);
    assert_eq!(
        item_color(&fenced.items[0]),
        Some(formulary::Color::rgb(255, 0, 0))
    );
}

#[test]
fn merror_gets_ua_styling() {
    let data = stix();
    let font = MathFont::new(&data, 0).unwrap();
    let opts = LayoutOptions { font_size: 16.0 };
    let laid = layout(
        &parse("<math><merror><mi>x</mi></merror></math>").unwrap(),
        &font,
        &opts,
    );
    let backgrounds = laid
        .items
        .iter()
        .filter(|i| matches!(i, formulary::Item::Background { .. }))
        .count();
    let red_rules = laid
        .items
        .iter()
        .filter(|i| {
            matches!(i, formulary::Item::Rule { color: Some(c), .. } if *c == formulary::Color::rgb(255, 0, 0))
        })
        .count();
    assert_eq!(backgrounds, 1, "light-yellow fill behind the contents");
    assert_eq!(red_rules, 4, "red border edges");
}

#[test]
fn underover_wrong_arity_warns() {
    assert!(!parse("<math><mover><mi>x</mi></mover></math>").unwrap().warnings.is_empty());
    assert!(!parse("<math><munderover><mo>&#x2211;</mo><mn>1</mn></munderover></math>")
        .unwrap()
        .warnings
        .is_empty());
}

#[test]
fn scripts_wrong_arity_warns() {
    assert!(!parse("<math><msup><mi>x</mi></msup></math>").unwrap().warnings.is_empty());
    assert!(!parse("<math><msubsup><mi>x</mi><mn>1</mn></msubsup></math>")
        .unwrap()
        .warnings
        .is_empty());
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
            formulary::Item::Rule { x, y, w, h, .. } => Some((x, y, w, h)),
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
