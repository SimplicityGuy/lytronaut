#!/usr/bin/env python3
"""Run actual PostgreSQL parity with exact counts; never silently skip a backend.

Uses synthetic fixtures only. Every case runs on PostgreSQL with no exclusions;
each group must report exactly its expected passes with zero failed, ignored or
measured tests, and the total is computed from this case list. `just
postgres-parity` supplies a disposable pinned server; this focused recipe is not
the full release/coverage gate.
"""
import os
import re
import signal
import subprocess
import sys
from urllib.parse import unquote, urlsplit


def main():
    if hasattr(os, "geteuid") and os.geteuid() == 0:
        # Fault tests revoke file permissions (chmod 0o000 on ownership markers and
        # journals) to prove fail-closed cleanup; root bypasses them, so a root run
        # would fail for reasons unrelated to the code under test.
        raise SystemExit("PostgreSQL parity must run as a non-root user: its restore "
                         "fault cases rely on file permissions that root bypasses")
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
        ("http-api", 7, ["-p", "lytronaut-service", "--test", "api", "--", "--skip", "live_photo_", "--test-threads=1"]),
        # Live Photo export create/download, DELETE of a ready pair, in-flight
        # cancellation and failed-plane cases, each against PostgreSQL.
        ("live-photo-api", 4, ["-p", "lytronaut-service", "--test", "api", "live_photo_", "--", "--test-threads=1"]),
        ("http-uploads", 4, ["-p", "lytronaut-service", "--test", "uploads", "--", "--test-threads=1"]),
        ("portable-matrix", 5, ["-p", "lytronaut-service", "--lib", "bundle::portable_bundle_tests::postgres_matrix_", "--", "--ignored", "--test-threads=1"]),
        # Includes a real SIGKILL of a child test process mid-restore; the child's
        # output goes to its fixture log, so exactly one result line is parsed here.
        ("restore-faults", 19, ["-p", "lytronaut-service", "--lib", "api::exports::restore_fault_tests::postgres_", "--", "--ignored", "--test-threads=1"]),
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
    expected_total = sum(expected for _, expected, _ in cases)
    if total != expected_total:
        raise SystemExit(f"PostgreSQL parity requires exactly {expected_total} actual passing tests")
    print(f"PostgreSQL parity: {total} passed; 0 failed; 0 ignored")


if __name__ == "__main__":
    main()
