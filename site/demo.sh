#!/usr/bin/env sh
# Record the demo, ruxguitar.gif, and its still, site/screenshot.png.
#
# Run it after changing the interface, and commit what it writes: the Pages
# workflow only copies the pictures. The link preview card embeds the still:
# run site/social.sh afterwards.
#
# The recording is scripted in src/ui/application/app/demo.rs, which renders
# the interface without a window and paces playback by hand: nothing is
# filmed from the screen. It takes a few minutes.
#
# Needs ffmpeg on the PATH, and Linux with ALSA: the player opens an audio
# output, pointed here at one that discards the sound.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

command -v ffmpeg >/dev/null || { echo "demo.sh: no ffmpeg found" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/asound.conf" <<'CONF'
</usr/share/alsa/alsa.conf>
pcm.!default { type null }
CONF

cd "$root"
ALSA_CONFIG_PATH="$tmp/asound.conf" \
DEMO_GIF="$root/ruxguitar.gif" \
DEMO_STILL="$root/site/screenshot.png" \
  cargo test --quiet record_the_demo -- --ignored

printf 'wrote %s\n' "$root/ruxguitar.gif" "$root/site/screenshot.png"
