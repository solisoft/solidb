#!/usr/bin/env bash
#
# Regenerate the Open Graph share cards from tools/og/og.html.
#
#   ./tools/og/build.sh
#
# Writes public/images/og-cover.png (home) and og-docs.png (documentation).
# Needs a Chromium/Chrome on PATH and network access on first run — the card
# pulls Space Grotesk / JetBrains Mono / Inter from Google Fonts, and the
# rendered PNG is what ships, so the fonts are only needed here.
#
# Snap-confined Chromium cannot write into /tmp, nor into dot-directories under
# $HOME, so we render into a visible $HOME staging dir and move the result.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out_dir="$here/../../public/images"
src="file://$here/og.html"

chrome="${CHROME:-}"
if [[ -z "$chrome" ]]; then
    for candidate in chromium chromium-browser google-chrome /snap/bin/chromium; do
        if command -v "$candidate" >/dev/null 2>&1; then
            chrome="$candidate"
            break
        fi
    done
fi
if [[ -z "$chrome" ]]; then
    echo "error: no chromium found. Set CHROME=/path/to/chrome and re-run." >&2
    exit 1
fi

staging="$(mktemp -d "${HOME}/soli-og-build.XXXXXX")"
trap 'rm -rf "$staging"' EXIT

mkdir -p "$out_dir"

render() {
    local variant_hash="$1" out_name="$2"
    # --virtual-time-budget lets the webfonts land before the shutter fires.
    "$chrome" --headless=new --disable-gpu --no-sandbox --hide-scrollbars \
        --force-device-scale-factor=1 --window-size=1200,630 \
        --virtual-time-budget=8000 \
        --screenshot="$staging/$out_name" "${src}${variant_hash}" 2>/dev/null
    mv "$staging/$out_name" "$out_dir/$out_name"
    echo "  wrote public/images/$out_name"
}

echo "rendering Open Graph cards with $chrome"
render ""      "og-cover.png"
render "#docs" "og-docs.png"
