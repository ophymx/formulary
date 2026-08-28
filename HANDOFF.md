# formulary — handoff brief

Native MathML renderer in Rust. No webview, no JS, no browser engine. Parse
presentation MathML, lay it out using OpenType MATH table metrics, emit a
resolution-independent display list. This crate is an **independent library**:
it must stand on its own, with no knowledge of any particular consumer. Design
every API as if the users are strangers.

Name is decided: `formulary` (a formulary is a book of formulas; confirmed
available on crates.io 2026-08-27).

## Why this is tractable

- **MathML Core** (W3C) is the target spec, not MathML 3. It is a deliberately
  shrunken subset whose layout rules are defined *operationally in terms of
  OpenType MATH table constants* — fraction rule thickness, script shifts,
  radical gaps all come from the font. Chromium's implementation (Igalia) and
  Firefox are living references. https://www.w3.org/TR/mathml-core/
- The Rust font stack already does the hard parts: `ttf-parser` parses the MATH
  table (constants, glyph variants, glyph assembly); `rustybuzz` shapes text
  including the `ssty` script-alternates feature.
- **web-platform-tests** has a full `mathml/` conformance suite — free,
  browser-validated test corpus.
- Prior art to mine (read, don't fork): ReX (github.com/ReTeX/ReX — Rust TeX
  math typesetter over OpenType MATH, stale but architecturally right,
  especially its stretchy-glyph assembly) and typst's math module (excellent,
  maintained, but entangled with the typst compiler — reference only).

## Architecture (decided)

Pipeline: MathML string → `quick-xml` or `roxmltree` parse → typed element
tree → layout pass reading MATH constants → **display list**.

The core output is a display list, NOT pixels/PNG/canvas/SVG:

```rust
pub struct Layout {
    pub width: f32,
    pub ascent: f32,   // above baseline — the inline-integration contract
    pub descent: f32,
    pub items: Vec<Item>,
}

pub enum Item {
    Glyph { id: GlyphId, x: f32, y: f32, size: f32 },
    Rule  { x: f32, y: f32, w: f32, h: f32 },   // fraction bars, radical rules
}
```

(Sketch, not gospel — refine as needed. Coordinates resolution-independent;
caller supplies target font size, layout is re-run on size change. Glyph IDs
not outlines, so consumers rasterize through their own text pipeline and math
matches surrounding body text exactly.)

Rationale for display-list-first: consumers (ereaders, document renderers)
must reflow at arbitrary font sizes/DPI; baseline metrics make inline math sit
on the text baseline; rasterization policy (hinting, grayscale, e-ink) belongs
to the app.

Adapters, feature-gated or separate crates, thin:
- **SVG emitter** — build FIRST despite not being the "real" output: it's the
  debugging window and golden-test format (glyphs outlined to paths via
  ttf-parser; text-diffable; side-by-side comparable with Firefox rendering).
- **PNG via tiny-skia** — dev-dependency for pixel-golden regression tests only.

Font handling: caller provides font bytes (must be a font with a MATH table).
Bundle STIX Two Math or New Computer Modern (both OFL) behind an optional
`bundled-font` feature so the crate works out of the box but stays lean.

Crate hygiene targets: pure library, no I/O in core, minimal deps
(ttf-parser, a Rust XML parser, optionally rustybuzz), no_std-friendly if
cheap. This crate should be usable by any Rust project, forever, alone.

## Scope

**Tier 0 (first milestone, ~90% of textbook math):** mi/mn/mo/mtext token
elements, mrow, mfrac, msup/msub/msubsup, msqrt/mroot, mspace, mstyle +
scriptlevel/displaystyle propagation, mpadded/mphantom. Both display and
inline math modes (cramped styles, displaystyle).

**Tier 1:** operator dictionary (MathML Core ships the data table — copy it),
stretchy operators via MATH glyph variants + assembly, large operators,
munder/mover/munderover, accents, msubsup alignment refinements.

**Tier 2:** mtable (matrices/alignment — it's a mini table engine; ship
without it initially), menclose subset if ever (not in Core), linebreaking of
long formulas (defer indefinitely).

**Non-goals:** content MathML, maction, elementary-math layout (mstack etc.),
RTL math (revisit only on demand), CSS integration, HTML interleaving.

Effort calibration from planning: Tier 0 ≈ 4–6 weeks; solid Core coverage
(through mtable) ≈ 3–5 person-months. Known risk areas: stretchy glyph
assembly edge cases per font; mtable; operator dictionary size (mechanical
but large).

## Testing strategy

- WPT `mathml/` suite as the conformance backbone (adapt reftests to display
  list/SVG assertions where possible).
- SVG golden files per feature, reviewed against Firefox screenshots of the
  same markup — prevents drifting into "looks okay to me."
- Property-ish checks: ink bounds within layout bounds, baseline sanity,
  monotonic size scaling.

## Open decisions for the implementing session

1. **Shaping ownership** for multi-char `<mi>`/`<mtext>` runs: pull in
   rustybuzz (self-contained, default-feature) vs. a caller-provided shaper
   trait (dependency-light, composes with a host text stack). Leaning:
   rustybuzz behind a default feature, trait escape hatch later if a consumer
   needs it. Independence of the crate wins ties.
2. XML parser choice: quick-xml vs roxmltree (roxmltree is simpler for a
   read-only tree; quick-xml is lighter/streaming). Either is fine.
3. Display-list units: font units vs ems vs px-at-size. Pick one, document it,
   keep baseline/ascent/descent in the same unit.
4. Whether `Item` needs glyph clusters/font references for mixed-font
   fallback, or v1 assumes a single math font (recommend: single font for v1).

## First-session checklist

1. `cargo init --lib`, workspace optional (core + svg adapter is fine as one
   crate with features to start).
2. Vendor a test font (STIX Two Math, OFL) into `fonts/` for tests.
3. Element tree + parser + Tier-0 layout skeleton for `mrow` of tokens (just
   horizontal advance + baseline), SVG dump, first golden test.
4. Then mfrac (first real MATH-constants use), then scripts, then radicals.
