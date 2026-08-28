# formulary

Native [MathML Core](https://www.w3.org/TR/mathml-core/) layout in Rust. No
webview, no JavaScript, no browser engine: parse presentation MathML, lay it
out using the font's OpenType MATH table, and get back a
resolution-independent display list you rasterize through your own text
pipeline.

```rust
use formulary::{parse, layout, LayoutOptions, MathFont};

let font_data = std::fs::read("fonts/STIXTwoMath-Regular.otf")?;
let font = MathFont::new(&font_data, 0)?;
let tree = parse(r#"<math><mfrac><mn>1</mn><mrow><mi>x</mi><mo>+</mo><mn>2</mn></mrow></mfrac></math>"#)?;
let laid = layout(&tree, &font, &LayoutOptions { font_size: 16.0 });

// laid.width / laid.ascent / laid.descent: place the baseline on your text
// baseline. laid.items: glyphs (by id, at positions) and rules to draw.
```

## Design

- **Display list out, not pixels.** `Layout` holds glyph ids with positions
  plus rules (fraction bars, radical overbars), all in the unit of the
  `font_size` you passed. Baseline metrics (`ascent`/`descent`) are the
  inline-integration contract. Re-run layout on size change — it's cheap.
- **The font drives the typography.** Fraction shifts, script positions,
  radical gaps, stretchy glyph variants and assemblies, italic corrections:
  all read from the font's MATH table, the same data browsers and TeX-family
  engines use. Any font with a MATH table works (STIX Two Math, Latin Modern
  Math, New Computer Modern, …); a font without one is rejected at
  construction.
- **You bring the font bytes.** The core does no I/O and knows nothing about
  your rendering stack, so glyphs rasterize through whatever pipeline draws
  your body text and math matches it exactly.

## Feature flags

| feature   | default | effect |
|-----------|---------|--------|
| `shaping` | yes     | token runs shaped with rustybuzz (kerning, script alternates) |
| `svg`     | yes     | `svg::to_svg` — self-contained SVG with glyphs outlined to paths; the debugging window and golden-test format |

With both off, the only dependencies are `ttf-parser` and `roxmltree`.

## Coverage

Supported: token elements (`mi` `mn` `mo` `mtext` `ms`) with `mathvariant`
styling, `mrow`, `mfrac` (incl. `linethickness`, with `0` giving bar-less
binomial stacks), `msqrt`/`mroot`, `msub`/`msup`/`msubsup`,
`munder`/`mover`/`munderover` (accents, movable limits), `mmultiscripts`
(+ `mprescripts`/`none`), `mtable`/`mtr`/`mtd` (`columnalign`, Core cell
padding), `mspace`, `mstyle`, `mpadded`, `mphantom`, `merror`,
`semantics`/`maction` (first child), deprecated `mfenced` (desugared);
operator dictionary spacing and forms, vertically and horizontally stretchy
operators via glyph variants *and* assembly with `minsize`/`maxsize`
clamping, large operators in display style, italic correction, `ssty`
script alternates, display/inline modes (`display` and `displaystyle`) with
full script-level and cramped-style propagation.

Not (yet) supported: `menclose` (not in MathML Core), `mtable` spans, RTL
math, content MathML, HTML inside token elements, linebreaking.

### Error handling

Parsing is lenient, following MathML Core's error-recovery rules: unknown
elements and structurally invalid markup (an `mfrac` with three children, a
stray `<none/>`) render via the spec's `mrow` fallback instead of failing,
and each recovery is reported in `MathRoot::warnings`. Only malformed XML
and a non-`<math>` root are hard errors. Check `warnings.is_empty()` when
you need to know the markup was fully understood.

Against the [web-platform-tests](https://github.com/web-platform-tests/wpt)
`mathml/` corpus (2,762 extracted fragments, including crashtests): 98%
render — 88% with no warnings — with zero panics and zero
geometric-invariant violations; the remaining 2% are malformed-XML
fragments. Fetch the corpus with `tools/fetch-wpt.sh`, then `cargo test
--test wpt_corpus -- --nocapture` prints the support matrix.

## Testing

- `cargo test` — unit/property tests plus SVG golden files under
  `tests/golden/` (regenerate with `UPDATE_GOLDEN=1 cargo test`, then review
  the diff; compare against a browser rendering the same markup).
- The golden SVGs are self-contained (glyphs outlined to paths) and
  text-diffable.

`fonts/STIXTwoMath-Regular.otf` (SIL OFL 1.1) is vendored for tests.

## License

MIT OR Apache-2.0.
