#!/usr/bin/env python3
"""Run the ignored >4 GiB bundle proof explicitly; never skip it silently.

The test round-trips one original larger than 4 GiB (sparse at the source)
through backup and isolated restore and recomputes every SHA-256. It needs
about 9 GiB of real scratch space in TMPDIR, so it runs only in the dedicated
CI job (or by hand), in an optimized test build so hashing stays fast.
"""
import os
import re
import shutil
import subprocess
import sys

NAME = "bundle::portable_bundle_tests::large_bundle_beyond_4_gib_round_trips_losslessly"
NEEDED = 10 * 1024**3


def main():
    scratch = os.environ.get("TMPDIR", "/tmp")
    free = shutil.disk_usage(scratch).free
    print(f"large bundle scratch: {free // 1024**3} GiB free in TMPDIR", flush=True)
    if free < NEEDED:
        raise SystemExit("large bundle proof needs at least 10 GiB free in TMPDIR")
    result = subprocess.run(
        ["cargo", "test", "--locked", "--release", "-p", "lytronaut-service", "--lib",
         NAME, "--", "--ignored", "--exact", "--nocapture"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=3600,
    )
    sys.stdout.write(result.stdout)
    summaries = re.findall(r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;", result.stdout)
    if result.returncode or summaries != [("ok", "1", "0", "0")]:
        raise SystemExit("large bundle proof did not run exactly one passing test")
    print("large bundle proof: 1 passed; 0 failed; 0 ignored")


if __name__ == "__main__":
    main()
