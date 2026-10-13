#!/usr/bin/env python3
"""Execute the documented Compose deployments against a prebuilt local image.

usage: deployment-test.py IMAGE ARCHITECTURE STORAGE FIXTURES OUTPUT

STORAGE is `bind` (host directories prepared with the documented UID/GID and
modes; requires a Linux host whose filesystem the Docker engine shares
natively) or `volume` (named volumes inside the engine, e.g. a macOS VM).
FIXTURES holds the synthetic JPEGs from `just synthetic-fixtures`.

Runs, with synthetic data only: the SQLite Compose path, the bundled
PostgreSQL Compose path and an externally managed PostgreSQL; persistent
storage across stop/start, down/up and restart-policy restarts; permissions;
secrets redaction; and all four SQLite/PostgreSQL backup→restore
combinations, each restored copy started and browsed through the documented
verification override. Every resource it creates carries a unique prefix and
is removed afterwards; nothing else is touched.
"""
import hashlib
import json
import os
import pathlib
import platform
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.dont_write_bytecode = True  # keep the checkout clean when importing the fixture
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from postgres_fixture import PostgresFixture, pinned_image  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
APP_UID = "10001"
PG_UID = "70"
HOST = "127.0.0.1:3000"
image, architecture, storage, fixtures, output = sys.argv[1:6] if len(sys.argv) == 6 else [None] * 5
if storage not in ("bind", "volume") or architecture not in ("amd64", "arm64"):
    raise SystemExit(__doc__)
if storage == "bind" and platform.system() != "Linux":
    raise SystemExit("bind storage proof requires a Linux host; use `volume` inside a VM engine")
PREFIX = os.environ.get("LYTRONAUT_TEST_PREFIX", "lytronaut-deploy") + f"-{os.getpid()}-{secrets.token_hex(3)}"
PG_IMAGE, PG_VERSION_NUM = pinned_image()
report = {"image": image, "architecture": architecture, "storage": storage, "prefix": PREFIX,
          "postgresImage": PG_IMAGE, "checks": []}
SENSITIVE = []  # every generated password; must never appear in any captured output


def checked(name):
    report["checks"].append(name)
    print("deployment:", name, file=sys.stderr, flush=True)


def run(*args, check=True, timeout=300, env=None, stdin=None):
    result = subprocess.run(list(args), capture_output=True, text=True, timeout=timeout, env=env, input=stdin)
    if check and result.returncode:
        detail = (result.stdout + result.stderr)[-2000:]
        for value in SENSITIVE:
            detail = detail.replace(value, "[redacted]")
        raise AssertionError(f"{' '.join(args[:4])} failed ({result.returncode}): {detail}")
    return result.stdout


def docker(*args, **kwargs):
    return run("docker", *args, **kwargs)


def as_root(mount, script):
    """Prepare/remove test-owned host directories without host sudo."""
    docker("run", "--rm", "--network", "none", "--user", "0", "--entrypoint", "sh",
           "-v", f"{mount}:/x", PG_IMAGE, "-c", script)


class Project:
    def __init__(self, name, files, env):
        self.name, self.files = f"{PREFIX}-{name}", files
        self.env = dict(os.environ, LYTRONAUT_IMAGE=image, LYTRONAUT_PORT="0", **env)
        for key in ("LYTRONAUT_DATABASE_URL", "LYTRONAUT_DATABASE_SCHEMA", "COMPOSE_FILE", "COMPOSE_PROJECT_NAME"):
            if key not in env:
                self.env.pop(key, None)
        self.started = False

    def compose(self, *args, extra=(), env=None, **kwargs):
        files = sum((["-f", str(f)] for f in [*self.files, *extra]), [])
        return docker("compose", "--project-directory", str(ROOT), "--env-file", str(EMPTY_ENV),
                      "-p", self.name, *files, *args, env={**self.env, **(env or {})}, **kwargs)

    def stream(self, *args, stdin=None, stdout=None, timeout=600):
        """Binary stdin/stdout passthrough (for pg_dump/pg_restore through `exec -T`)."""
        files = sum((["-f", str(f)] for f in self.files), [])
        result = subprocess.run(["docker", "compose", "--project-directory", str(ROOT), "--env-file", str(EMPTY_ENV),
                                 "-p", self.name, *files, *args], stdin=stdin, stdout=stdout,
                                stderr=subprocess.PIPE, env=self.env, timeout=timeout)
        assert result.returncode == 0, (args[:4], result.stderr[-1000:])

    def up(self, extra=(), env=None, recreate=False):
        self.started = True
        self.compose("up", "-d", "--no-build", "--wait", "--wait-timeout", "300",
                     *(["--force-recreate", "lytronaut"] if recreate else []), extra=extra, env=env, timeout=420)
        return self

    def container(self, service="lytronaut"):
        identifier = self.compose("ps", "-a", "-q", service).strip()
        assert identifier, f"{service} not running"
        return identifier

    def inspect(self, service="lytronaut"):
        return json.loads(docker("inspect", self.container(service)))[0]

    def client(self):
        port = int(self.compose("port", "lytronaut", "3000").strip().rsplit(":", 1)[1])
        return Client(port)

    def down(self, volumes=False):
        self.compose("down", "--timeout", "60", *(["-v"] if volumes else []), "--remove-orphans", timeout=240)


class Client:
    def __init__(self, port):
        self.port = port

    def request(self, method, path, body=None, expected=200, headers=None):
        h = {"Host": HOST, **(headers or {})}
        if method not in ("GET", "HEAD"):
            h.update({"Origin": f"http://{HOST}", "X-Lytronaut-Intent": "mutation"})
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            h["Content-Type"] = "application/json"
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=body, headers=h, method=method)
        try:
            with urllib.request.urlopen(req, timeout=120) as response:
                status, data = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, data = error.code, error.read()
        assert status == expected, (method, path, status, data[:300])
        return data

    def json(self, method, path, body=None, expected=200):
        return json.loads(self.request(method, path, body, expected))

    def ready(self):
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            try:
                if self.json("GET", "/healthz")["ready"]:
                    return self
            except (OSError, AssertionError, ValueError):
                pass
            time.sleep(0.5)
        raise AssertionError("service not ready within 180s")

    def terminal(self, job):
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline:
            value = self.json("GET", f"/exports/{job}")
            if value["state"] in ("succeeded", "failed", "cancelled"):
                return value
            time.sleep(0.2)
        raise AssertionError("export job not terminal within 300s")

    def import_library(self):
        job = self.json("POST", "/imports", {"version": 1, "kind": "directory",
                                             "declared_bytes": sum(map(len, ORIGINALS.values())),
                                             "declared_files": len(ORIGINALS)}, expected=201)["id"]
        for path, data in ORIGINALS.items():
            self.request("PUT", f"/imports/{job}/file", data, expected=204,
                         headers={"X-Lytronaut-Entry-Type": "file", "X-Lytronaut-Path-Hex": path.encode().hex()})
        self.request("POST", f"/imports/{job}/finish", b"", expected=204)

    def snapshot(self):
        """Observable catalog: libraries, captures and exact original bytes of one capture."""
        libraries = self.json("GET", "/libraries")["items"]
        assert len(libraries) == 1 and libraries[0]["state"] == "ready", libraries
        library = libraries[0]["id"]
        captures = self.json("GET", f"/libraries/{library}/captures?limit=100")["items"]
        assert len(captures) == 30, len(captures)
        focus = next(c for c in captures if c["name"] == "Synthetic 000")
        detail = self.json("GET", f"/captures/{focus['id']}")
        digests = sorted(hashlib.sha256(self.request("GET", f"/assets/{a['id']}/download")).hexdigest()
                         for a in detail["originals"])
        assert set(digests) <= SOURCE_DIGESTS and digests, "originals differ from synthetic sources"
        return {"library": library, "captures": sorted((c["id"], c["name"]) for c in captures),
                "originals": digests}

    def backup(self):
        job = self.json("POST", "/exports", {"kind": "archive_originals", "version": 1})["id"]
        assert self.terminal(job)["state"] == "succeeded"
        return self.request("GET", f"/exports/{job}/download")

    def restore(self, bundle):
        job = self.json("POST", "/restores", bundle)["id"]
        state = self.terminal(job)
        assert state["state"] == "succeeded", state
        return job


def permissions(project, service, paths):
    out = docker("exec", project.container(service), "stat", "-c", "%u:%g:%a", *paths)
    return out.split()


def verify_restore(project, client, job, expected, backend):
    """The documented verification: start the same service on the isolated copy."""
    store = f"/data/work/staging.exports/{job}.restored/store"
    descriptor = json.loads(docker("exec", project.container(), "cat", f"{store}/restore.json"))
    assert descriptor["backend"] == backend, descriptor
    override = SCRATCH / f"verify-{job}.yaml"
    override.write_text("services:\n  lytronaut:\n    environment:\n"
                        f"      LYTRONAUT_DATA: {store}\n"
                        f"      LYTRONAUT_ORIGINALS: {store}/originals\n"
                        f"      LYTRONAUT_STAGING: {store}/staging\n")
    env = {"LYTRONAUT_DATABASE_SCHEMA": descriptor["schema"]} if backend == "postgres" else None
    project.up(extra=[override], env=env, recreate=True)
    restored = project.client().ready().snapshot()
    assert restored == expected, (restored, expected)
    project.up(recreate=True)  # back to the active catalog, unchanged
    assert project.client().ready().snapshot() == ACTIVE[project.name]


def persistent_restarts(project, service_checks):
    client = project.client().ready()
    before = client.snapshot()
    project.compose("stop", "--timeout", "60", timeout=180)
    for service in service_checks:
        state = project.inspect(service)["State"]
        assert state["Status"] == "exited" and state["ExitCode"] == 0, (service, state)
    project.up()
    assert project.client().ready().snapshot() == before
    checked(f"{project.name}: graceful stop exit 0 and restart on the same storage")
    project.down()
    project.up()
    assert project.client().ready().snapshot() == before
    checked(f"{project.name}: down/up recreated containers on persistent storage")
    # unless-stopped: an application exit is restarted by the engine.
    container = project.container()
    docker("exec", container, "sh", "-c", "kill -TERM 1")
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        info = json.loads(docker("inspect", container))[0]
        if info["RestartCount"] >= 1 and info["State"].get("Health", {}).get("Status") == "healthy":
            break
        time.sleep(1)
    else:
        raise AssertionError("restart policy did not restart the application")
    assert project.inspect()["HostConfig"]["RestartPolicy"]["Name"] == "unless-stopped"
    assert project.client().ready().snapshot() == before
    checked(f"{project.name}: restart policy restarted an exited application healthy")
    return before


def redaction_audit(projects):
    texts = []
    for project in projects:
        texts.append(project.compose("config"))
        texts.append(project.compose("logs", "--no-color", timeout=120))
        for identifier in project.compose("ps", "-a", "-q").split():
            texts.append(docker("inspect", identifier))
    texts.append(docker("image", "inspect", image))
    texts.append(json.dumps(report))
    for value in SENSITIVE:
        assert not any(value in text for text in texts), "generated database password leaked"
    checked("no generated password in compose config, container config/env, logs, image or report")


def write_secret(directory, name, value, owner, mode):
    path = directory / name
    path.write_text(value)
    path.chmod(0o600)
    as_root(directory, f"chown {owner} /x/{name} && chmod {mode} /x/{name}")


def storage_env(name, extra=()):
    if storage == "volume":
        return {}
    base = SCRATCH / name
    base.mkdir(mode=0o700)
    dirs = {"LYTRONAUT_CATALOG_DIR": "catalog", "LYTRONAUT_ORIGINALS_DIR": "originals",
            "LYTRONAUT_WORK_DIR": "work"}
    script = " && ".join(f"install -d -m 0700 -o {APP_UID} -g {APP_UID} /x/{d}" for d in dirs.values())
    if "postgres" in extra:
        script += f" && install -d -m 0700 -o {PG_UID} -g {PG_UID} /x/postgres"
        dirs["LYTRONAUT_POSTGRES_DIR"] = "postgres"
    as_root(base, script)
    return {key: str(base / value) for key, value in dirs.items()}


def host_permissions(env):
    if storage != "bind":
        return
    for key, path in env.items():
        info = os.stat(path)
        owner = int(PG_UID if key == "LYTRONAUT_POSTGRES_DIR" else APP_UID)
        assert (info.st_uid, info.st_gid, info.st_mode & 0o777) == (owner, owner, 0o700), (key, info)
    checked("host-selected bind directories keep documented owner and 0700 mode")


def main():
    global SCRATCH, EMPTY_ENV, ORIGINALS, SOURCE_DIGESTS, ACTIVE
    ACTIVE = {}
    scratch_root = os.environ.get("LYTRONAUT_TEST_SCRATCH") or None
    SCRATCH = pathlib.Path(tempfile.mkdtemp(prefix=f"{PREFIX}-", dir=scratch_root))
    EMPTY_ENV = SCRATCH / "empty.env"
    EMPTY_ENV.write_text("")
    projects, fixture = [], None

    def interrupted(signum, _frame):
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        # Synthetic library only.
        source = pathlib.Path(fixtures)
        for name in ["synthetic.jpg", *[f"synthetic.focus{n}.jpg" for n in range(3)]]:
            shutil.copy(source / name, SCRATCH / name)
        subprocess.run([sys.executable, str(ROOT / "scripts/synthetic-library.py"), str(SCRATCH)], check=True, timeout=60)
        library = SCRATCH / "Synthetic.lytrolibrary"
        ORIGINALS = {str(p.relative_to(library)): p.read_bytes() for p in sorted(library.rglob("*")) if p.is_file()}
        SOURCE_DIGESTS = {hashlib.sha256(v).hexdigest() for v in ORIGINALS.values()}

        info = json.loads(docker("image", "inspect", image))[0]
        assert info["Architecture"] == architecture, info["Architecture"]
        engine = docker("info", "--format", "{{.Architecture}}").strip()
        normalized = {"x86_64": "amd64", "aarch64": "arm64"}.get(engine, engine)
        report.update(host=f"{platform.system()} {platform.machine()}", engineArchitecture=engine,
                      imageArchitecture=info["Architecture"], emulated=normalized != architecture,
                      imageID=info["Id"])

        # 1. Default SQLite Compose path.
        sqlite_env = storage_env("sqlite")
        sqlite = Project("sqlite", [ROOT / "compose.yaml"], sqlite_env)
        projects.append(sqlite)
        sqlite.up()
        client = sqlite.client().ready()
        assert "LYTRONAUT_DATABASE_URL" not in json.dumps(sqlite.inspect()["Config"]["Env"])
        assert permissions(sqlite, "lytronaut", ["/data/catalog", "/data/originals", "/data/work"]) == \
            [f"{APP_UID}:{APP_UID}:700"] * 3
        client.import_library()
        ACTIVE[sqlite.name] = client.snapshot()
        owners = docker("exec", sqlite.container(), "find", "/data/catalog", "/data/originals", "/data/work",
                        "-printf", "%U:%G\n")
        assert set(owners.split()) == {f"{APP_UID}:{APP_UID}"}, set(owners.split())
        checked("SQLite Compose: healthy, 10001:10001 0700 storage, synthetic import and exact originals")
        host_permissions(sqlite_env)
        persistent_restarts(sqlite, ["lytronaut"])
        sqlite_bundle = sqlite.client().ready().backup()

        # 2. Bundled PostgreSQL Compose path with file secrets.
        secrets_dir = SCRATCH / "secrets"
        secrets_dir.mkdir(mode=0o700)
        app_password = secrets.token_hex(16) + "%@:/?#&'\" +"
        bootstrap_password = secrets.token_hex(24)
        SENSITIVE.extend([app_password, bootstrap_password, app_password.split("%")[0]])
        # CRLF ending: the application reader and the role bootstrap must both strip it.
        write_secret(secrets_dir, "lytronaut-database-password", app_password + "\r\n", f"{APP_UID}:{PG_UID}", "0440")
        write_secret(secrets_dir, "postgres-bootstrap-password", bootstrap_password, f"{PG_UID}:{PG_UID}", "0400")
        postgres_env = storage_env("postgres", extra=("postgres",))
        postgres = Project("postgres", [ROOT / "compose.yaml", ROOT / "compose.postgres.yaml"],
                           {**postgres_env, "LYTRONAUT_SECRETS_DIR": str(secrets_dir)})
        projects.append(postgres)
        postgres.up()
        pg = postgres.container("postgres")
        version = docker("exec", pg, "psql", "--no-psqlrc", "-U", "postgres", "-d", "lytronaut", "-Atqc",
                         "SHOW server_version_num").strip()
        assert version == str(PG_VERSION_NUM), version
        report["postgresServerVersionNum"] = int(version)
        report["postgresMachine"] = docker("exec", pg, "uname", "-m").strip()
        role = docker("exec", pg, "psql", "--no-psqlrc", "-U", "postgres", "-d", "lytronaut", "-Atqc",
                      "SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls, "
                      "has_schema_privilege('lytronaut','public','CREATE') FROM pg_roles WHERE rolname='lytronaut'").strip()
        assert role == "f|f", role
        assert postgres.inspect("postgres")["Config"]["User"] == "70:70"
        assert permissions(postgres, "postgres", ["/var/lib/postgresql/18/docker"]) == [f"{PG_UID}:{PG_UID}:700"]
        client = postgres.client().ready()
        client.import_library()
        ACTIVE[postgres.name] = client.snapshot()
        tables = docker("exec", pg, "psql", "--no-psqlrc", "-U", "postgres", "-d", "lytronaut", "-Atqc",
                        "SELECT count(*) FILTER (WHERE schemaname='lytronaut'), count(*) FILTER (WHERE schemaname='public') "
                        "FROM pg_tables").strip()
        owned, public = map(int, tables.split("|"))
        assert owned > 0 and public == 0, tables
        checked(f"PostgreSQL {version} Compose: healthy restricted role, owned schema only, synthetic import")
        host_permissions(postgres_env)
        persistent_restarts(postgres, ["lytronaut", "postgres"])
        checked("bundled PostgreSQL and the application both accept a CRLF-terminated password secret")
        # Compose-controlled PostgreSQL restart restarts the dependent application.
        application = postgres.container()
        started = json.loads(docker("inspect", application))[0]["State"]["StartedAt"]
        postgres.compose("restart", "postgres", timeout=240)
        restarted = json.loads(docker("inspect", application))[0]["State"]["StartedAt"]
        assert restarted != started, "application was not restarted with PostgreSQL"
        postgres.up()
        assert postgres.client().ready().snapshot() == ACTIVE[postgres.name]
        checked("Compose restart of PostgreSQL restarted the dependent application, which reconnected")
        # Engine-level crash: the postmaster dies without Compose; the restart policy
        # brings PostgreSQL back and the running application reconnects by itself.
        database = postgres.container("postgres")
        application_started = json.loads(docker("inspect", application))[0]["State"]["StartedAt"]
        # The exec may itself be cut off when the container's main process exits.
        docker("exec", database, "pg_ctl", "stop", "-m", "immediate", "-D", "/var/lib/postgresql/18/docker",
               check=False)
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            info = json.loads(docker("inspect", database))[0]
            if info["RestartCount"] >= 1 and info["State"].get("Health", {}).get("Status") == "healthy":
                break
            time.sleep(1)
        else:
            raise AssertionError("PostgreSQL was not restarted after an engine crash")
        assert postgres.client().ready().snapshot() == ACTIVE[postgres.name]
        # Record, rather than assume, whether recovery needed an application restart.
        in_place = json.loads(docker("inspect", application))[0]["State"]["StartedAt"] == application_started
        report["applicationReconnectedInPlaceAfterEngineCrash"] = in_place
        checked("PostgreSQL engine crash: restart policy recovered it and the application serves the catalog "
                + ("(reconnected in place)" if in_place else "(after its own restart)"))
        postgres_bundle = postgres.client().ready().backup()

        # Documented recovery, literally: stop the application, pg_dump through
        # `exec -T`, lose the PostgreSQL location, start a fresh cluster, drop its
        # empty bootstrap schema, pg_restore as the role through `exec -T`, start.
        dump = SCRATCH / "lytronaut-catalog.dump"
        postgres.compose("stop", "lytronaut", timeout=120)
        with open(dump, "wb") as out:
            postgres.stream("exec", "-T", "postgres", "pg_dump", "-U", "postgres", "-d", "lytronaut",
                            "--format=custom", stdout=out)
        assert dump.stat().st_size > 0
        postgres.compose("down", "--timeout", "60", timeout=240)
        if storage == "bind":
            as_root(SCRATCH / "postgres", f"rm -rf /x/postgres && install -d -m 0700 -o {PG_UID} -g {PG_UID} /x/postgres")
        else:
            docker("volume", "rm", f"{postgres.name}_postgres")
        postgres.compose("up", "-d", "--no-build", "--wait", "--wait-timeout", "300", "postgres", timeout=420)
        postgres.compose("exec", "-T", "postgres", "psql", "--no-psqlrc", "-v", "ON_ERROR_STOP=1",
                         "-U", "postgres", "-d", "lytronaut", "-c", "DROP SCHEMA lytronaut")
        with open(dump, "rb") as source:
            postgres.stream("exec", "-T", "postgres", "pg_restore", "-U", "postgres", "-d", "lytronaut",
                            "--no-owner", "--no-acl", "--role=lytronaut", "--exit-on-error", stdin=source)
        postgres.up()
        assert postgres.client().ready().snapshot() == ACTIVE[postgres.name]
        owner = postgres.compose("exec", "-T", "postgres", "psql", "--no-psqlrc", "-U", "postgres", "-d", "lytronaut",
                                 "-Atqc", "SELECT nspowner::regrole FROM pg_namespace WHERE nspname='lytronaut'").strip()
        assert owner == "lytronaut", owner
        checked("documented pg_dump/pg_restore recovery onto a fresh bundled cluster; application browses it")

        # Misconfiguration fails closed without exposing the credential or URL.
        wrong = SCRATCH / "wrong-password"
        wrong.write_text("synthetic-wrong-password")
        wrong.chmod(0o644)
        SENSITIVE.append("synthetic-wrong-password")
        for url, expected in (("postgres://lytronaut@postgres:5432/lytronaut?sslmode=disable",
                               "application database unavailable"),
                              ("postgres://lytronaut:synthetic-wrong-password@postgres:5432/lytronaut?sslmode=disable",
                               "invalid application database configuration")):
            name = f"{PREFIX}-misconfigured"
            result = subprocess.run(["docker", "run", "--rm", "--name", name, "--network", f"{postgres.name}_database",
                                     "--read-only", "--user", f"{APP_UID}:{APP_UID}", "--cap-drop=ALL",
                                     "-e", f"LYTRONAUT_DATABASE_URL={url}", "-e", "LYTRONAUT_DATABASE_SCHEMA=lytronaut",
                                     "-e", "LYTRONAUT_DATABASE_PASSWORD_FILE=/run/secrets/wrong",
                                     "-v", f"{wrong}:/run/secrets/wrong:ro", image],
                                    capture_output=True, text=True, timeout=120)
            logged = result.stdout + result.stderr
            assert result.returncode == 1, (result.returncode, logged[-500:])
            assert expected in logged and "synthetic-wrong-password" not in logged, logged[-500:]
            assert "postgres://" not in logged and "@postgres" not in logged, logged[-500:]
        checked("wrong or ambiguous database credentials refuse startup with a redacted message")

        # 3. All four backup→restore combinations, each copy started and browsed.
        for target, bundle, origin in [(sqlite, sqlite_bundle, sqlite), (sqlite, postgres_bundle, postgres),
                                       (postgres, sqlite_bundle, sqlite), (postgres, postgres_bundle, postgres)]:
            job = target.client().ready().restore(bundle)
            backend = "postgres" if target is postgres else "sqlite"
            verify_restore(target, target.client(), job, ACTIVE[origin.name], backend)
            source_backend = "postgres" if origin is postgres else "sqlite"
            checked(f"restore {source_backend}->{backend}: isolated copy started and browsed identically")

        # 4. Externally managed PostgreSQL configured only by URL + secret file.
        network = f"{PREFIX}-external-network"
        fixture = PostgresFixture(f"{PREFIX}-external", database="lytronaut", role="lytronaut",
                                  schema="lytronaut", sentinel=True, network=network, publish=False)
        fixture.start()
        SENSITIVE.append(fixture.password)
        external_secrets = SCRATCH / "external-secrets"
        external_secrets.mkdir(mode=0o700)
        write_secret(external_secrets, "lytronaut-database-password", fixture.password, f"{APP_UID}:{APP_UID}", "0400")
        attach = SCRATCH / "external-network.yaml"
        attach.write_text("services:\n  lytronaut:\n    networks: [default, external_database]\n"
                          f"networks:\n  external_database:\n    external: true\n    name: {network}\n")
        external_env = storage_env("external")
        external = Project("external", [ROOT / "compose.yaml", ROOT / "compose.postgres-external.yaml", attach],
                           {**external_env, "LYTRONAUT_SECRETS_DIR": str(external_secrets),
                            "LYTRONAUT_DATABASE_URL": fixture.url(host=fixture.name, password=False)})
        projects.append(external)
        external.up()
        client = external.client().ready()
        client.import_library()
        ACTIVE[external.name] = client.snapshot()
        persistent_restarts(external, ["lytronaut"])
        job = external.client().ready().restore(sqlite_bundle)
        verify_restore(external, external.client(), job, ACTIVE[sqlite.name], "postgres")
        sentinel = fixture.superuser_sql("SELECT marker FROM public.lytronaut_parity_sentinel").strip()
        public = fixture.superuser_sql("SELECT count(*) FROM pg_tables WHERE schemaname='public'").strip()
        assert (sentinel, public) == ("synthetic-persistence-parity", "1"), (sentinel, public)
        checked("external PostgreSQL by credential-free URL + secret file: import, restarts, restore; public untouched")
        # Documented external recovery: dump, prepare an empty database (role and
        # grants, no schema), restore as the role, point the URL at it and start.
        external.compose("stop", "lytronaut", timeout=120)
        dump = SCRATCH / "external-catalog.dump"
        with open(dump, "wb") as out:
            result = subprocess.run(["docker", "exec", fixture.name, "pg_dump", "-U", "postgres", "-d", "lytronaut",
                                     "--format=custom"], stdout=out, stderr=subprocess.PIPE, timeout=600)
        assert result.returncode == 0 and dump.stat().st_size > 0, result.stderr[-500:]
        fixture.superuser_sql("CREATE DATABASE lytronaut_recovered")
        docker("exec", fixture.name, "psql", "--no-psqlrc", "-v", "ON_ERROR_STOP=1", "-U", "postgres",
               "-d", "lytronaut_recovered", "-c", "REVOKE CREATE ON SCHEMA public FROM PUBLIC",
               "-c", "REVOKE ALL ON DATABASE lytronaut_recovered FROM PUBLIC",
               "-c", "GRANT CONNECT, CREATE, TEMPORARY ON DATABASE lytronaut_recovered TO lytronaut")
        with open(dump, "rb") as source:
            result = subprocess.run(["docker", "exec", "-i", fixture.name, "pg_restore", "-U", "postgres",
                                     "-d", "lytronaut_recovered", "--no-owner", "--no-acl", "--role=lytronaut",
                                     "--exit-on-error"], stdin=source, stderr=subprocess.PIPE, timeout=600)
        assert result.returncode == 0, result.stderr[-500:]
        recovered_url = fixture.url(host=fixture.name, password=False).replace("/lytronaut?", "/lytronaut_recovered?")
        external.up(env={"LYTRONAUT_DATABASE_URL": recovered_url}, recreate=True)
        assert external.client().ready().snapshot() == ACTIVE[external.name]
        checked("documented external pg_dump/pg_restore recovery into an empty database; application browses it")

        redaction_audit(projects)
        out = pathlib.Path(output)
        out.mkdir(parents=True, exist_ok=True)
        (out / "deployment.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
        print(f"Deployment validation: {len(report['checks'])} checks passed", file=sys.stderr)
    finally:
        for project in reversed(projects):
            if project.started:
                try:
                    project.down(volumes=True)
                except Exception as error:  # keep removing the remaining owned resources
                    print(f"cleanup of {project.name} failed: {error}", file=sys.stderr)
        if fixture:
            fixture.close()
        subprocess.run(["docker", "rm", "-f", f"{PREFIX}-misconfigured"], capture_output=True)
        if storage == "bind" and SCRATCH.exists():
            as_root(SCRATCH, "rm -rf /x/sqlite /x/postgres /x/external /x/secrets /x/external-secrets")
        shutil.rmtree(SCRATCH, ignore_errors=True)


if __name__ == "__main__":
    main()
