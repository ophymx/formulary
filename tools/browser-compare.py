#!/usr/bin/env python3
"""Side-by-side drift check: formulary's golden SVGs vs a browser rendering
the same markup with the same font.

Extracts every check_golden(name, markup) case from tests/golden.rs, writes a
comparison page per case (our SVG on top, native MathML below, STIX Two Math
at 32px), and screenshots each with a headless browser into
target/browser-compare/. Eyeball the PNGs; formulary is expected to match
Chromium (the MathML Core reference implementation). Known Firefox
deviations, both legacy Gecko behavior:
  - fences that are direct children of <math> don't stretch (they do in an
    explicit <mrow>);
  - operator lspace/rspace collapse at script level (MathML 3 rule, absent
    from Core).

Usage: tools/browser-compare.py [firefox|chrome]
"""

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "browser-compare"

PAGE = """<!doctype html>
<meta charset="utf-8">
<style>
  @font-face {{
    font-family: "STIX Two Math";
    src: url("file://{root}/fonts/STIXTwoMath-Regular.otf");
  }}
  body {{ margin: 8px; background: white; font-family: sans-serif; }}
  .label {{ font-size: 11px; color: #888; margin: 4px 0 2px; }}
  math {{ font-family: "STIX Two Math"; font-size: 32px; }}
  .box {{ border-left: 3px solid #ddd; padding-left: 10px; min-height: 40px; }}
</style>
<div class="label">formulary</div>
<div class="box"><img src="file://{root}/tests/golden/{name}.svg"></div>
<div class="label">browser</div>
<div class="box">{markup}</div>
"""


def golden_cases():
    src = (ROOT / "tests" / "golden.rs").read_text()
    pattern = re.compile(
        r'check_golden\(\s*"(\w+)"\s*,\s*(?:r#"(.*?)"#|"(.*?)")\s*,?\s*\)',
        re.DOTALL,
    )
    for m in pattern.finditer(src):
        yield m.group(1), m.group(2) or m.group(3)


def screenshot(browser, page, png):
    with tempfile.TemporaryDirectory() as profile:
        if browser == "chrome":
            cmd = [
                "google-chrome", "--headless=new", "--disable-gpu",
                "--allow-file-access-from-files",
                f"--screenshot={png}", "--window-size=700,340",
                f"--user-data-dir={profile}", f"file://{page}",
            ]
        else:
            cmd = [
                "firefox", "--headless", "--profile", profile,
                "--screenshot", str(png), f"file://{page}",
                "--window-size=700,340",
            ]
        subprocess.run(cmd, capture_output=True, timeout=60, check=False)


def main():
    browser = sys.argv[1] if len(sys.argv) > 1 else "firefox"
    OUT.mkdir(parents=True, exist_ok=True)
    cases = list(golden_cases())
    for name, markup in cases:
        page = OUT / f"{name}.html"
        page.write_text(PAGE.format(root=ROOT, name=name, markup=markup))
        png = OUT / f"{name}.{browser}.png"
        screenshot(browser, page, png)
        status = "ok" if png.exists() else "FAILED"
        print(f"{name}: {status}")
    print(f"\n{len(cases)} comparisons in {OUT}")


if __name__ == "__main__":
    main()
