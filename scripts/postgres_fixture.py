#!/usr/bin/env python3
"""Disposable, task-owned PostgreSQL for tests; never a shared or user database.

Starts the exact pinned official image from compose.postgres.yaml as one
uniquely named container with its own named volume (and optional network),
bootstraps a restricted application role through the in-container superuser
socket, and removes exactly those resources afterwards.

CLI: postgres_fixture.py -- COMMAND...   runs COMMAND with
LYTRONAUT_TEST_POSTGRES_URL naming the disposable database, then cleans up.
"""
import os
import pathlib
import re
import secrets
import signal
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
TIMEOUT = 120


def pinned_image():
    """The single pinned release+digest, shared with the bundled Compose service."""
    text = (ROOT / "compose.postgres.yaml").read_text()
    match = re.search(r"^\s+image: (postgres:(\d+)\.(\d+)-alpine@sha256:[0-9a-f]{64})$", text, re.M)
    if not match:
        raise SystemExit("compose.postgres.yaml must pin postgres:<release>-alpine@sha256:<digest>")
    return match.group(1), int(match.group(2)) * 10000 + int(match.group(3))


def docker(*args, stdin=None, check=True, timeout=TIMEOUT, env=None):
    result = subprocess.run(["docker", *args], input=stdin, text=True, capture_output=True,
                            timeout=timeout, env=env)
    if check and result.returncode:
        # Never echo SQL input; it may contain a generated password.
        raise SystemExit(f"docker {args[0]} failed: {result.stderr.strip()[:500]}")
    return result.stdout


class PostgresFixture:
    """One container + one named volume (+ optional private network), all prefixed."""

    def __init__(self, prefix, database="lytronaut_parity", role="lytronaut_parity_app",
                 schema=None, sentinel=True, network=None, publish=True):
        self.image, self.version_num = pinned_image()
        self.name = f"{prefix}-postgres"
        self.volume = f"{self.name}-data"
        self.network = network
        self.database, self.role, self.schema = database, role, schema
        self.sentinel, self.publish = sentinel, publish
        self.password = secrets.token_hex(24)
        self.created = []

    def start(self):
        if self.network:
            docker("network", "create", "--internal", self.network)
            self.created.append(("network", self.network))
        docker("volume", "create", self.volume)
        self.created.append(("volume", self.volume))
        bootstrap = secrets.token_hex(24)
        run = ["run", "-d", "--name", self.name,
               "--network", self.network or "bridge",
               "--cpus", "1", "--memory", "1g", "--pids-limit", "256",
               "--security-opt", "no-new-privileges:true",
               "-v", f"{self.volume}:/var/lib/postgresql",
               "-e", f"POSTGRES_DB={self.database}",
               # Value taken from this process's environment, not the argument list.
               "-e", "POSTGRES_PASSWORD"]
        if self.publish:
            run += ["-p", "127.0.0.1::5432"]
        self.created.append(("container", self.name))
        docker(*run, self.image, env=dict(os.environ, POSTGRES_PASSWORD=bootstrap))
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            # TCP readiness only exists after the image's initialization phase.
            probe = docker("exec", self.name, "pg_isready", "--host=127.0.0.1", f"--dbname={self.database}",
                           check=False, timeout=10)
            if "accepting connections" in probe:
                break
            time.sleep(0.5)
        else:
            raise SystemExit("disposable PostgreSQL did not become ready")
        sql = [
            "SET log_min_error_statement = panic;",
            "SET log_statement = none;",
            f"CREATE ROLE {self.role} LOGIN PASSWORD '{self.password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;",
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC;",
            f"REVOKE ALL ON DATABASE {self.database} FROM PUBLIC;",
            f"GRANT CONNECT, CREATE, TEMPORARY ON DATABASE {self.database} TO {self.role};",
        ]
        if self.schema:
            sql.append(f"CREATE SCHEMA {self.schema} AUTHORIZATION {self.role};")
        if self.sentinel:
            sql += [
                "CREATE TABLE public.lytronaut_parity_sentinel(marker text PRIMARY KEY);",
                "INSERT INTO public.lytronaut_parity_sentinel VALUES ('synthetic-persistence-parity');",
                f"GRANT USAGE ON SCHEMA public TO {self.role};",
                f"GRANT SELECT ON public.lytronaut_parity_sentinel TO {self.role};",
            ]
        docker("exec", "-i", self.name, "psql", "--no-psqlrc", "--quiet", "-v", "ON_ERROR_STOP=1",
               "-U", "postgres", "-d", self.database, stdin="\n".join(sql) + "\n")
        actual = self.superuser_sql("SHOW server_version_num").strip()
        if actual != str(self.version_num):
            raise SystemExit(f"unexpected PostgreSQL server_version_num {actual}; pinned {self.version_num}")
        print(f"Disposable PostgreSQL {self.image} server_version_num {actual}", file=sys.stderr, flush=True)
        return self

    def superuser_sql(self, statement):
        return docker("exec", self.name, "psql", "--no-psqlrc", "-U", "postgres", "-d", self.database,
                      "-Atqc", statement)

    def host_port(self):
        mapping = docker("port", self.name, "5432/tcp").splitlines()[0]
        return int(mapping.rsplit(":", 1)[1])

    def url(self, host=None, port=None, password=True):
        host = host or "127.0.0.1"
        port = port or (self.host_port() if host == "127.0.0.1" else 5432)
        credential = f":{self.password}" if password else ""
        return f"postgres://{self.role}{credential}@{host}:{port}/{self.database}?sslmode=disable"

    def close(self):
        for kind, name in reversed(self.created):
            if kind == "container":
                docker("rm", "-f", "-v", name, check=False)
            else:
                docker(kind, "rm", name, check=False)
        self.created.clear()


def main():
    if len(sys.argv) < 3 or sys.argv[1] != "--":
        raise SystemExit("usage: postgres_fixture.py -- COMMAND [ARG...]")
    prefix = os.environ.get("LYTRONAUT_TEST_PREFIX", "lytronaut-test") + f"-{os.getpid()}-{secrets.token_hex(3)}"
    fixture = PostgresFixture(prefix)

    def interrupted(signum, _frame):
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        fixture.start()
        env = dict(os.environ, LYTRONAUT_TEST_POSTGRES_URL=fixture.url())
        with subprocess.Popen(sys.argv[2:], env=env, start_new_session=True) as child:
            try:
                code = child.wait(timeout=int(os.environ.get("LYTRONAUT_TEST_TIMEOUT", "5400")))
            except BaseException:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                raise
        sys.exit(code)
    finally:
        fixture.close()


if __name__ == "__main__":
    main()
