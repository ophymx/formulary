#!/bin/sh
# Fetch the web-platform-tests mathml/ suite used by tests/wpt_corpus.rs.
# The harness is skipped when third_party/wpt is absent, so this is optional.
set -eu
cd "$(dirname "$0")/.."
PIN=5f8cfdc18b18b1619c9fe431eab72f2831823327
if [ -d third_party/wpt ]; then
    echo "third_party/wpt already exists" >&2
    exit 0
fi
git clone --depth 1 --filter=blob:none --sparse \
    https://github.com/web-platform-tests/wpt.git third_party/wpt
cd third_party/wpt
git sparse-checkout set mathml
echo "note: cloned latest; harness was last validated against $PIN" >&2
