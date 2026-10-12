#!/usr/bin/env python3
"""Run actual PostgreSQL parity with exact counts; never silently skip a backend.

Uses synthetic fixtures only. Portable backup API coverage is pending lytronaut-1ga;
this focused recipe is not the full release/coverage gate.
"""
import os
import re
import signal
import subprocess
import sys
from urllib.parse import unquote, urlsplit


def main():
    url = os.environ.get("LYTRONAUT_TEST_POSTGRES_URL")
    if not url:
        raise SystemExit("LYTRONAUT_TEST_POSTGRES_URL must identify a disposable test database")
    try:
        parsed = urlsplit(url)
        if parsed.scheme not in ("postgres", "postgresql") or not parsed.hostname:
            raise ValueError
        password = unquote(parsed.password or "")
    except ValueError:
        raise SystemExit("invalid disposable PostgreSQL configuration") from None

    env = dict(os.environ, LYTRONAUT_TEST_BACKEND="postgres")
    cases = [
        ("storage", 7, ["-p", "lytronaut-storage", "--lib", "database::tests::postgres_", "--", "--ignored", "--test-threads=1"]),
        ("desktop", 1, ["-p", "lytronaut-storage", "--test", "desktop", "postgres_real_intake_adaptation_originals_conflicts_concurrent_reimport_and_reopen", "--", "--exact", "--ignored", "--test-threads=1"]),
        # Exact sixth scenario exclusion is temporary: PG portable backup remains
        # pending 1ga. It stays active in the independent default SQLite suite.
        ("http-api", 5, ["-p", "lytronaut-service", "--test", "api", "--", "--test-threads=1", "--skip", "public_lossless_bundle_unknown_bytes_catalog_restore_tamper_conflict_and_interruption"]),
    ]
    total = 0
    for name, expected, args in cases:
        print(f"PostgreSQL {name}: expecting {expected} actual tests", flush=True)
        try:
            with subprocess.Popen(["cargo", "test", "--locked", *args], env=env,
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, start_new_session=True) as process:
                try:
                    output, _ = process.communicate(timeout=1200)
                except subprocess.TimeoutExpired:
                    # Terminate only this command's own process group, including
                    # compiler/test children, rather than leave work running.
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.communicate(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.communicate()
                    raise SystemExit(f"PostgreSQL {name} exceeded its bounded timeout") from None
                returncode = process.returncode
        except OSError:
            raise SystemExit(f"PostgreSQL {name} could not start Cargo") from None
        # Do not expose deployment credentials even when a driver/test fails.
        output = output.replace(url, "[redacted database URL]")
        if password:
            output = output.replace(password, "[redacted password]")
        sys.stdout.write(output)
        summaries = re.findall(r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", output)
        if returncode or len(summaries) != 1:
            raise SystemExit(f"PostgreSQL {name} failed or did not emit exactly one test result")
        outcome, passed, failed, ignored, measured, _filtered = summaries[0]
        if outcome != "ok" or tuple(map(int, (passed, failed, ignored, measured))) != (expected, 0, 0, 0):
            raise SystemExit(f"PostgreSQL {name} count/zero-skip assertion failed")
        total += int(passed)
    if total != 13:
        raise SystemExit("PostgreSQL parity requires exactly thirteen actual passing tests")
    print("PostgreSQL parity: 13 passed; 0 failed; 0 ignored (portable backup pending 1ga)")


if __name__ == "__main__":
    main()
