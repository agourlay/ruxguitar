#!/usr/bin/env sh
# Render the link preview card, site/social.html, to site/social.png.
#
# Run it after changing the card or retaking site/screenshot.png, and commit
# what it writes: the Pages workflow only copies the PNG.
#
# Needs a Chromium: set $CHROME to its path, or have chromium or
# google-chrome on the PATH.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

chrome=${CHROME:-$(command -v chromium || command -v chromium-browser || command -v google-chrome || true)}
[ -n "$chrome" ] || { echo "social.sh: no Chromium found; set \$CHROME" >&2; exit 1; }

# --no-sandbox because some installs abort without it in headless mode; the
# page rendered is our own local file. The time budget lets the screenshot
# decode before the capture.
"$chrome" --headless --no-sandbox --disable-gpu --hide-scrollbars \
  --force-device-scale-factor=1 --virtual-time-budget=3000 \
  --window-size=1200,630 --screenshot="$root/site/social.png" \
  "file://$root/site/social.html" 2>/dev/null

printf 'wrote %s\n' "$root/site/social.png"
