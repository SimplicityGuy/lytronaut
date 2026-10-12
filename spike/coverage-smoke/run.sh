#!/usr/bin/env bash
# Throwaway spike driver. Env: TOOLCHAIN, EXPECT_LLVM, MINICOV, OUT, SPIKE_DIR, WB_BIN
set -euo pipefail
: "${TOOLCHAIN:?}" "${EXPECT_LLVM:?}" "${MINICOV:?}" "${OUT:?}" "${SPIKE_DIR:?}" "${WB_BIN:?}"
mkdir -p "$OUT"
W="$RUNNER_TEMP/covsmoke"
rm -rf "$W" && mkdir -p "$W/src"
cp "$SPIKE_DIR/src/lib.rs" "$W/src/lib.rs"
cat > "$W/Cargo.toml" <<EOF
[package]
name = "covsmoke"
version = "0.0.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
wasm-bindgen = "=0.2.129"
minicov = "=${MINICOV}"
EOF

echo "::group::tool versions"
{
  echo "== rustc -vV"; rustc "+$TOOLCHAIN" -vV
  echo "== cargo -V"; cargo "+$TOOLCHAIN" -V
  echo "== clang --version"; clang --version
  echo "== wasm-bindgen --version"; "$WB_BIN" --version
  echo "== node --version"; node --version
  echo "== google-chrome --version"; /usr/bin/google-chrome --version
} | tee "$OUT/versions.txt"
echo "::endgroup::"

LLVM_SEEN=$(rustc "+$TOOLCHAIN" -vV | sed -n 's/^LLVM version: //p')
if [[ "$LLVM_SEEN" != "$EXPECT_LLVM"* ]]; then
  echo "::error::expected LLVM $EXPECT_LLVM, rustc reports $LLVM_SEEN"; exit 1
fi

SYSROOT=$(rustc "+$TOOLCHAIN" --print sysroot)
BIN=$(dirname "$(ls "$SYSROOT"/lib/rustlib/*/bin/llvm-profdata)")
PROFDATA="$BIN/llvm-profdata"; LLVMCOV="$BIN/llvm-cov"
{ echo "== llvm-profdata --version"; "$PROFDATA" --version; echo "== llvm-cov --version"; "$LLVMCOV" --version; } | tee -a "$OUT/versions.txt"

# cc for wasm32 wants an llvm-ar; ubuntu ships versioned ones.
AR_BIN=$(command -v llvm-ar || ls /usr/bin/llvm-ar-* 2>/dev/null | sort -V | tail -1 || true)
echo "AR for wasm32: $AR_BIN"
export AR_wasm32_unknown_unknown="$AR_BIN"
export CARGO_INCREMENTAL=0
export RUSTFLAGS="-Cinstrument-coverage -Zno-profiler-runtime -Clink-args=--no-gc-sections ${EXTRA_RUSTFLAGS:-}"
echo "RUSTFLAGS=$RUSTFLAGS" | tee -a "$OUT/versions.txt"

echo "::group::cargo build"
(cd "$W" && cargo "+$TOOLCHAIN" build --target wasm32-unknown-unknown 2>&1) | tee "$OUT/build.log"
echo "::endgroup::"
cp "$W/Cargo.lock" "$OUT/Cargo.lock"
RAW="$W/target/wasm32-unknown-unknown/debug/covsmoke.wasm"
cp "$RAW" "$OUT/raw.wasm"
echo "raw wasm sha256: $(sha256sum "$RAW")"

# Custom sections present in the raw module (look for coverage map sections).
python3 -I - "$RAW" <<'PY' | tee "$OUT/sections.txt"
import sys
b = open(sys.argv[1], 'rb').read()
def leb(i):
    r = s = 0
    while True:
        c = b[i]; i += 1; r |= (c & 0x7f) << s; s += 7
        if c < 0x80: return r, i
i = 8
while i < len(b):
    sid = b[i]; i += 1
    size, i = leb(i); end = i + size
    if sid == 0:
        n, j = leb(i); print('custom', b[j:j+n].decode(errors='replace'), size)
    else:
        print('section', sid, size)
    i = end
PY

mkdir -p "$W/site/pkg"
"$WB_BIN" --target web --out-dir "$W/site/pkg" "$RAW"
cp "$W/site/pkg/covsmoke_bg.wasm" "$OUT/bindgen.wasm"
echo '<!doctype html><meta charset="utf-8"><title>covsmoke</title>' > "$W/site/index.html"

(cd "$W/site" && python3 -m http.server 8765 --bind 127.0.0.1 >"$OUT/http.log" 2>&1) &
HTTP_PID=$!
trap 'kill $HTTP_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do curl -fsS -o /dev/null http://127.0.0.1:8765/index.html && break; sleep 0.2; done

echo "::group::headless chrome"
drive_fail=0
node "$NODE_DIR/drive.mjs" http://127.0.0.1:8765/index.html "$OUT/x.profraw" "$OUT/drive.json" 2>&1 | tee "$OUT/drive.log" || drive_fail=1
echo "::endgroup::"
[ -s "$OUT/x.profraw" ] || { echo "::error::no profraw produced"; exit 1; }

set +e
"$PROFDATA" merge -sparse -failure-mode=all "$OUT/x.profraw" -o "$OUT/x.profdata" 2> "$OUT/profdata.stderr"; rc1=$?
"$PROFDATA" show --all-functions --counts "$OUT/x.profdata" > "$OUT/profdata-show.txt" 2>> "$OUT/profdata.stderr"
"$LLVMCOV" export -format=lcov -instr-profile "$OUT/x.profdata" -object "$OUT/raw.wasm" > "$OUT/x.lcov" 2> "$OUT/cov-export.stderr"; rc2=$?
"$LLVMCOV" report -instr-profile "$OUT/x.profdata" -object "$OUT/raw.wasm" > "$OUT/cov-report.txt" 2> "$OUT/cov-report.stderr"; rc3=$?
"$LLVMCOV" report -show-functions -instr-profile "$OUT/x.profdata" "$OUT/raw.wasm" "$W/src/lib.rs" > "$OUT/cov-report-functions.txt" 2>> "$OUT/cov-report.stderr"
"$LLVMCOV" show -instr-profile "$OUT/x.profdata" "$OUT/raw.wasm" "$W/src/lib.rs" > "$OUT/cov-show.txt" 2> "$OUT/cov-show.stderr"; rc4=$?
set -e
for f in profdata.stderr cov-export.stderr cov-report.stderr cov-show.stderr; do echo "== $f (rc)"; cat "$OUT/$f"; done
echo "rc: merge=$rc1 export=$rc2 report=$rc3 show=$rc4"
echo "::group::profdata show"; cat "$OUT/profdata-show.txt"; echo "::endgroup::"
echo "== llvm-cov report"; cat "$OUT/cov-report.txt"
echo "== llvm-cov report -show-functions lib.rs"; cat "$OUT/cov-report-functions.txt"
echo "== llvm-cov show lib.rs"; cat "$OUT/cov-show.txt"

fail=$drive_fail
[ $drive_fail -eq 0 ] || echo "::error::headless driver failed (signature mismatch or runtime error)"
[ $rc1 -eq 0 ] && [ $rc2 -eq 0 ] && [ $rc3 -eq 0 ] && [ $rc4 -eq 0 ] || { echo "::error::a tool exited non-zero"; fail=1; }
if cat "$OUT"/*.stderr | grep -iE 'mismatch|malformed|unsupported|no coverage data|warning|error'; then
  echo "::error::tool stderr contains warning/error text"; fail=1
fi

python3 -I - "$OUT/x.lcov" "$W/src/lib.rs" <<'PY' | tee "$OUT/verdict.txt" || fail=1
import sys
lcov, src = sys.argv[1], sys.argv[2]
lines = open(src).read().splitlines()
mark = {l.strip(): i + 1 for i, l in enumerate(lines) if l.strip().startswith('// ')}
hit = range(mark['// HIT-BEGIN'], mark['// HIT-END'])
miss = range(mark['// MISS-BEGIN'], mark['// MISS-END'])
da, cur = {}, None
for l in open(lcov):
    l = l.strip()
    if l.startswith('SF:'): cur = l[3:]
    elif l.startswith('DA:') and cur and cur.endswith('src/lib.rs'):
        n, c = l[3:].split(',')[:2]; da[int(n)] = int(c)
h = {n: c for n, c in da.items() if n in hit}
m = {n: c for n, c in da.items() if n in miss}
print('lib.rs DA records:', sorted(da.items()))
print('hit() lines :', sorted(h.items()))
print('miss() lines:', sorted(m.items()))
ok = bool(h) and all(c > 0 for c in h.values()) and bool(m) and all(c == 0 for c in m.values())
print('VERDICT:', 'PASS' if ok else 'FAIL')
sys.exit(0 if ok else 1)
PY
exit $fail
