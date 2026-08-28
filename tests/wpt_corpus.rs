//! Conformance harness over the web-platform-tests `mathml/` suite.
//!
//! WPT's reftests need a browser; what this harness extracts from them is
//! their *markup corpus*: every `<math>` fragment in the suite runs through
//! parse + layout under invariant checks (no panics, finite geometry, linear
//! size scaling), and parse recoveries are tallied by element into a
//! warning matrix printed at the end (run with `--nocapture` to see it).
//!
//! The corpus lives in `third_party/wpt` (gitignored); fetch it with
//! `tools/fetch-wpt.sh`. When absent, the test passes as a skip so offline
//! builds stay green.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use formulary::{layout, parse, Item, LayoutOptions, MathFont, ParseError};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("third_party/wpt/mathml")
}

fn html_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            html_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "html") {
            out.push(path);
        }
    }
}

/// Extract `<math>…</math>` fragments from an HTML source. Naive scan; a
/// fragment with no closing tag (crashtests contain some) is skipped.
fn math_fragments(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let rest = html;
    let mut offset = 0;
    while let Some(start) = rest[offset..].find("<math") {
        let start = offset + start;
        match rest[start..].find("</math>") {
            Some(end) => {
                let end = start + end + "</math>".len();
                out.push(&rest[start..end]);
                offset = end;
            }
            None => break,
        }
    }
    let _ = rest;
    out
}

/// Replace named character references the XML parser won't know. Known math
/// entities map to their characters; unknown ones become U+FFFD so the
/// fragment still exercises layout.
fn resolve_entities(fragment: &str) -> String {
    const KNOWN: &[(&str, &str)] = &[
        ("nbsp", "\u{A0}"),
        ("ApplyFunction", "\u{2061}"),
        ("af", "\u{2061}"),
        ("InvisibleTimes", "\u{2062}"),
        ("it", "\u{2062}"),
        ("InvisibleComma", "\u{2063}"),
        ("ic", "\u{2063}"),
        ("ThinSpace", "\u{2009}"),
        ("MediumSpace", "\u{205F}"),
        ("ThickSpace", "\u{205F}\u{200A}"),
        ("ZeroWidthSpace", "\u{200B}"),
        ("PlusMinus", "\u{B1}"),
        ("pm", "\u{B1}"),
        ("minus", "\u{2212}"),
        ("times", "\u{D7}"),
        ("sum", "\u{2211}"),
        ("int", "\u{222B}"),
        ("prod", "\u{220F}"),
        ("alpha", "\u{3B1}"),
        ("beta", "\u{3B2}"),
        ("gamma", "\u{3B3}"),
        ("RightArrow", "\u{2192}"),
        ("rightarrow", "\u{2192}"),
        ("DoubleRightArrow", "\u{21D2}"),
        ("OverBrace", "\u{23DE}"),
        ("UnderBrace", "\u{23DF}"),
        ("Hat", "\u{302}"),
        ("prime", "\u{2032}"),
        ("infin", "\u{221E}"),
    ];
    let mut out = String::with_capacity(fragment.len());
    let mut chars = fragment.char_indices();
    while let Some((i, c)) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let rest = &fragment[i + 1..];
        // Predefined XML and numeric references pass through untouched.
        let semicolon = rest.find(';').filter(|&n| n > 0 && n <= 40);
        let name = semicolon.map(|n| &rest[..n]);
        match name {
            Some(name)
                if name.starts_with('#')
                    || matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") =>
            {
                out.push('&');
            }
            Some(name) if name.chars().all(|c| c.is_ascii_alphanumeric()) => {
                let replacement = KNOWN
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map_or("\u{FFFD}", |(_, v)| v);
                out.push_str(replacement);
                for _ in 0..name.len() + 1 {
                    chars.next();
                }
            }
            _ => out.push('&'),
        }
    }
    out
}

fn finite(v: f32) -> bool {
    v.is_finite()
}

/// Check one laid-out fragment's geometric invariants; returns an error
/// description on violation.
fn check_invariants(markup: &str, font: &MathFont) -> Result<(), String> {
    let tree = match parse(markup) {
        Ok(t) => t,
        Err(_) => return Ok(()), // counted separately by the caller
    };
    let at16 = layout(&tree, font, &LayoutOptions { font_size: 16.0 });
    if !(finite(at16.width) && finite(at16.ascent) && finite(at16.descent)) {
        return Err("non-finite metrics".into());
    }
    if at16.width < -1e-3 || at16.ascent < -1e-3 || at16.descent < -1e-3 {
        return Err(format!(
            "negative extent: w={} a={} d={}",
            at16.width, at16.ascent, at16.descent
        ));
    }
    for item in &at16.items {
        match *item {
            Item::Glyph { x, y, size, .. } => {
                if !(finite(x) && finite(y) && finite(size)) || size <= 0.0 {
                    return Err(format!("bad glyph item: x={x} y={y} size={size}"));
                }
            }
            Item::Rule { x, y, w, h }
                if (!(finite(x) && finite(y) && finite(w) && finite(h)) || w < 0.0 || h < 0.0) => {
                    return Err(format!("bad rule item: x={x} y={y} w={w} h={h}"));
                }
            _ => {}
        }
    }
    // Absolute lengths (px/pt attribute values) legitimately break linear
    // scaling with font size, so only em/ex/%-free fragments are checked.
    if !markup.contains("px") && !markup.contains("pt") {
        let at32 = layout(&tree, font, &LayoutOptions { font_size: 32.0 });
        let lin = |a: f32, b: f32| (b - 2.0 * a).abs() <= 0.02 * a.abs().max(1.0);
        if !(lin(at16.width, at32.width)
            && lin(at16.ascent, at32.ascent)
            && lin(at16.descent, at32.descent))
        {
            return Err("size scaling is not linear".into());
        }
    }
    Ok(())
}

#[test]
fn wpt_corpus() {
    let root = corpus_root();
    if !root.is_dir() {
        eprintln!("wpt corpus not present; run tools/fetch-wpt.sh to enable this test");
        return;
    }
    let font_data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/STIXTwoMath-Regular.otf"
    ))
    .expect("vendored test font");
    let font = MathFont::new(&font_data, 0).unwrap();

    let mut files = Vec::new();
    html_files(&root, &mut files);
    files.sort();

    let mut total = 0usize;
    let mut clean = 0usize;
    let mut recovered = 0usize;
    let mut xml_errors = 0usize;
    let mut warned: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();

    for file in &files {
        let Ok(html) = std::fs::read_to_string(file) else {
            continue;
        };
        for fragment in math_fragments(&html) {
            total += 1;
            let markup = resolve_entities(fragment);
            let outcome = std::panic::catch_unwind(|| {
                let warnings = parse(&markup).map(|root| root.warnings);
                let invariants = check_invariants(&markup, &font);
                (warnings, invariants)
            });
            let rel = file.strip_prefix(&root).unwrap_or(file).display();
            match outcome {
                Err(_) => failures.push(format!("PANIC in {rel}: {}", snippet(&markup))),
                Ok((warnings, invariants)) => {
                    match warnings {
                        Ok(w) if w.is_empty() => clean += 1,
                        Ok(w) => {
                            recovered += 1;
                            for warning in w {
                                let element = match warning {
                                    formulary::Warning::UnknownElement { element } => element,
                                    formulary::Warning::InvalidStructure {
                                        element, ..
                                    } => format!("{element} (structure)"),
                                };
                                *warned.entry(element).or_default() += 1;
                            }
                        }
                        Err(ParseError::Xml(_)) | Err(ParseError::NotMath { .. }) => {
                            xml_errors += 1
                        }
                    }
                    if let Err(e) = invariants {
                        failures.push(format!("INVARIANT in {rel}: {e}: {}", snippet(&markup)));
                    }
                }
            }
        }
    }

    eprintln!("== WPT mathml corpus ==");
    eprintln!("files: {}, fragments: {total}", files.len());
    eprintln!("clean: {clean}, recovered with warnings: {recovered}");
    eprintln!("rendered: {} ({:.1}%)", clean + recovered,
        100.0 * (clean + recovered) as f64 / total as f64);
    eprintln!("xml/html-isms rejected: {xml_errors}");
    let mut by_count: Vec<_> = warned.into_iter().collect();
    by_count.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    eprintln!("top warnings:");
    for (element, n) in by_count.iter().take(20) {
        eprintln!("  {element:>25}: {n}");
    }
    // Group failures by kind so 500 repeats of one bug read as one line.
    let mut by_kind: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    for f in &failures {
        let kind = f.split(':').nth(2).map(str::trim).unwrap_or("other");
        let entry = by_kind.entry(kind).or_insert((0, f.as_str()));
        entry.0 += 1;
    }
    for (kind, (count, example)) in &by_kind {
        eprintln!("failure kind [{kind}] x{count}, e.g.:\n  {example}");
    }
    assert!(
        failures.is_empty(),
        "{} corpus failures (see stderr)",
        failures.len()
    );
}

fn snippet(s: &str) -> String {
    let mut t: String = s.chars().take(160).collect();
    if t.len() < s.len() {
        t.push('…');
    }
    t
}
