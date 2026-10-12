#!/usr/bin/env bash
# Public-only feature evidence. Tests keep their failure status; packaging is atomic.
set -euo pipefail
if [[ -n ${LYTRONAUT_VIDEO_PRIVATE_ROOT:-} || -n ${LYTRONAUT_PRIVATE_SEALED_ROOT:-} || ${LYTRONAUT_PRIVATE_TEST:-0} == 1 ]]; then
    echo 'Public evidence rejects private mode' >&2; exit 1
fi
: "${LYTRONAUT_OWNER_EVIDENCE:?isolated public evidence directory required}"
: "${LYTRONAUT_CLIENT:?current feature client directory required}"
: "${LYTRONAUT_TEST_REVISION:?current source revision required}"
archive=owner-evidence.tar.gz
partial=owner-evidence.tar.gz.partial
rm -f "$archive" "$partial"
# Never carry reports from an earlier run into this proof.
for name in runtime.json runtime-large.json runtime-padded.json jpeg-stress.json status.json source.tar.gz; do
    rm -f "$LYTRONAUT_OWNER_EVIDENCE/$name"
done
package() {
    original=$?
    trap - EXIT
    set +e
    packaged=0
    TEST_EXIT_STATUS="$original" node -e 'const fs=require("node:fs"),path=require("node:path");fs.writeFileSync(path.join(process.env.LYTRONAUT_OWNER_EVIDENCE,"status.json"),JSON.stringify({testExitStatus:Number(process.env.TEST_EXIT_STATUS),partial:process.env.TEST_EXIT_STATUS!=="0",publicSynthetic:true})+"\n",{mode:0o600,flag:"wx"})'
    if [[ $? == 0 ]] && [[ "$(git rev-parse HEAD)" == "$LYTRONAUT_TEST_REVISION" ]]; then
        if git archive HEAD | gzip -1 > "$LYTRONAUT_OWNER_EVIDENCE/source.tar.gz"; then
            files=("$LYTRONAUT_OWNER_EVIDENCE/source.tar.gz" "$LYTRONAUT_OWNER_EVIDENCE/status.json")
            for name in application.wasm lytronaut_client.js lytronaut_client_bg.wasm lytronaut_client.d.ts lytronaut_client_bg.wasm.d.ts; do
                files+=("$LYTRONAUT_CLIENT/$name")
            done
            reports_ok=1
            for name in runtime.json runtime-large.json runtime-padded.json jpeg-stress.json; do
                report="$LYTRONAUT_OWNER_EVIDENCE/$name"
                if [[ -f "$report" ]]; then
                    if node -e 'const fs=require("node:fs");const file=process.argv[1];if(fs.lstatSync(file).isSymbolicLink()||fs.statSync(file).size>256*1024)process.exit(1);JSON.parse(fs.readFileSync(file,"utf8"))' "$report"; then
                        files+=("$report")
                    else reports_ok=0; fi
                fi
            done
            if [[ $reports_ok == 1 ]] && tar -czf "$partial" "${files[@]}"; then
                if node -e 'const fs=require("node:fs");if(fs.statSync("owner-evidence.tar.gz.partial").size>128*1024*1024)process.exit(1)'; then
                    if mv "$partial" "$archive"; then packaged=1; fi
                fi
            fi
        fi
    fi
    if [[ $packaged != 1 ]]; then
        rm -f "$partial" "$archive"
        if [[ $original == 0 ]]; then original=1; fi
        echo 'Public feature evidence packaging failed' >&2
    fi
    exit "$original"
}
trap package EXIT
bash scripts/live-photo-owner-suite.sh
# A suite failure stops here; its partial reports still enter the labelled archive.
node scripts/live-photo-jpeg-stress-test.cjs
