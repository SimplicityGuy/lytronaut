#!/usr/bin/env bash
set -euo pipefail
if [[ ${LYTRONAUT_GPU_LIFETIME_TEST:-0} == 1 ]]; then
    test "${LYTRONAUT_GPU_TEST:-0}" = 1
    test "${LYTRONAUT_PRIVATE_TEST:-0}" != 1
    test "${LYTRONAUT_OWNER_TEST:-0}" != 1
    test -z "${LYTRONAUT_VIDEO_PRIVATE_ROOT:-}${LYTRONAUT_PRIVATE_SEALED_ROOT:-}"
    test -n "${LYTRONAUT_GPU_EVIDENCE:-}"
fi
browser_scratch=$(mktemp -d "${TMPDIR:-/tmp}/lytronaut-browser.XXXXXX")
chmod 700 "$browser_scratch"
# Keep synthetic screenshot/results for local review; CI scratch contains no private data.
target_root=${CARGO_TARGET_DIR:-target}
"$target_root/debug/examples/synthetic_jpeg" "$browser_scratch/synthetic.jpg"
if [[ ${LYTRONAUT_OWNER_TEST:-0} == 1 && ${LYTRONAUT_OWNER_SHAPE:-small} != small ]]; then
    case "$LYTRONAUT_OWNER_SHAPE" in large|padded) ;; *) echo 'Unsupported public owner shape' >&2; exit 1;; esac
    node scripts/live-photo-fixture.cjs "$browser_scratch/synthetic.video.lfp" "$LYTRONAUT_OWNER_SHAPE"
else
    LYTRONAUT_VIDEO_FIXTURE_OUT="$browser_scratch/synthetic.video.lfp" node scripts/video-test.cjs
fi
python3 scripts/synthetic-library.py "$browser_scratch"
case "$target_root" in /*) browser_binary="$target_root/debug/lytronaut-service";; *) browser_binary="$(pwd)/$target_root/debug/lytronaut-service";; esac
LYTRONAUT_TEST_ROOT="$browser_scratch" LYTRONAUT_BINARY="$browser_binary" node scripts/browser-test.cjs
if [[ ${LYTRONAUT_OWNER_TEST:-0} == 1 ]]; then
    cp "$browser_scratch/owner-results.json" "$LYTRONAUT_OWNER_EVIDENCE/runtime${LYTRONAUT_OWNER_SUFFIX:-}.json"
fi
if [[ ${LYTRONAUT_GPU_LIFETIME_TEST:-0} == 1 ]]; then
    test -n "${LYTRONAUT_GPU_EVIDENCE:-}"
    test "$(wc -c < "$browser_scratch/gpu-lifetime-results.json")" -le 16384
    cp "$browser_scratch/gpu-lifetime-results.json" "$LYTRONAUT_GPU_EVIDENCE/runtime-gpu-lifetime.json"
fi
printf 'Synthetic browser artifacts: %s\n' "$browser_scratch"
