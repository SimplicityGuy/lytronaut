//! The actual `lytronaut-service` executable: container health probe, local
//! diagnostics commands, startup failures and graceful SIGINT shutdown.
//! Synthetic temporary state only; paths never appear in output.
use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

const BINARY: &str = env!("CARGO_BIN_EXE_lytronaut-service");

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        let root = std::env::temp_dir().join(format!("lytronaut-cli-{:x}", u128::from_le_bytes(b)));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let client = root.join("client");
        fs::create_dir(&client).unwrap();
        fs::write(
            client.join("lytronaut_client.js"),
            b"export default async function init() {}",
        )
        .unwrap();
        fs::write(client.join("lytronaut_client_bg.wasm"), b"\0asm\x01\0\0\0").unwrap();
        Self(root)
    }
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn command(scratch: &Scratch, port: u16) -> Command {
    let mut c = Command::new(BINARY);
    c.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("LYTRONAUT_DATA", scratch.data())
        .env("LYTRONAUT_CLIENT", scratch.0.join("client"))
        .env("LYTRONAUT_ORIGIN", format!("http://127.0.0.1:{port}"))
        .env("LYTRONAUT_LISTEN", format!("127.0.0.1:{port}"));
    // Keep instrumented builds writing their profiles.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        c.env("LLVM_PROFILE_FILE", profile);
    }
    c
}
fn healthcheck(scratch: &Scratch, port: u16, env: &[(&str, &str)]) -> bool {
    let mut c = command(scratch, port);
    c.arg("--healthcheck").envs(env.iter().copied());
    c.status().unwrap().success()
}
fn run(c: &mut Command) -> Output {
    c.stdin(Stdio::null()).output().unwrap()
}
fn interrupt(child: &mut Child) -> std::process::ExitStatus {
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "service ignored SIGINT");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn healthcheck_follows_the_live_service_and_sigint_exits_cleanly() {
    let scratch = Scratch::new();
    let port = free_port();
    // Nothing is listening yet: the probe fails closed.
    assert!(!healthcheck(&scratch, port, &[]));
    let mut service = command(&scratch, port)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while !healthcheck(&scratch, port, &[]) {
        assert!(
            service.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        assert!(Instant::now() < deadline, "service never became healthy");
        std::thread::sleep(Duration::from_millis(100));
    }
    // An unspecified listen address is probed on loopback.
    let any = format!("0.0.0.0:{port}");
    assert!(healthcheck(&scratch, port, &[("LYTRONAUT_LISTEN", &any)]));
    // Misconfiguration never reports healthy.
    for env in [
        [("LYTRONAUT_ORIGIN", "http://[bad")],
        [("LYTRONAUT_ORIGIN", "/relative-only")],
        [("LYTRONAUT_LISTEN", "not-an-address")],
    ] {
        assert!(!healthcheck(&scratch, port, &env), "{env:?}");
    }
    let other = format!("127.0.0.1:{}", free_port());
    assert!(!healthcheck(
        &scratch,
        port,
        &[("LYTRONAUT_LISTEN", &other)]
    ));

    let status = interrupt(&mut service);
    assert!(
        status.success(),
        "graceful shutdown must exit 0, got {status:?}"
    );
    let mut stderr = String::new();
    std::io::Read::read_to_string(service.stderr.as_mut().unwrap(), &mut stderr).unwrap();
    assert!(stderr.contains("Lytronaut listening"));
    assert!(!stderr.contains(scratch.0.to_str().unwrap()));
    // The private data directory was created with owner-only permissions.
    assert_eq!(
        fs::metadata(scratch.data()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(!healthcheck(&scratch, port, &[]));
}

fn assert_startup_failure(output: &Output, scratch: &Scratch) {
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Lytronaut startup or service failed; check local configuration"),
        "{stderr}"
    );
    assert!(!stderr.contains(scratch.0.to_str().unwrap()));
}

#[test]
fn startup_failures_exit_nonzero_with_redacted_guidance() {
    let scratch = Scratch::new();
    let port = free_port();
    // The data path is a regular file, not a private directory.
    fs::write(scratch.data(), b"not a directory").unwrap();
    assert_startup_failure(&run(&mut command(&scratch, port)), &scratch);
    fs::remove_file(scratch.data()).unwrap();

    // A PostgreSQL password without a PostgreSQL URL is a configuration error,
    // reported without the secret path.
    let secret = scratch.0.join("secret");
    fs::write(&secret, b"synthetic-password\n").unwrap();
    let output = run(command(&scratch, port).env("LYTRONAUT_DATABASE_PASSWORD_FILE", &secret));
    assert_startup_failure(&output, &scratch);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Lytronaut: "), "{stderr}");
    assert!(!stderr.contains("synthetic-password"));

    // Missing browser client assets and an unusable listen address.
    let output = run(command(&scratch, port).env("LYTRONAUT_CLIENT", scratch.0.join("absent")));
    assert_startup_failure(&output, &scratch);
    let output = run(command(&scratch, port).env("LYTRONAUT_LISTEN", "nonsense"));
    assert_startup_failure(&output, &scratch);
    // An occupied port.
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = occupied.local_addr().unwrap().to_string();
    let output = run(command(&scratch, port).env("LYTRONAUT_LISTEN", &busy));
    assert_startup_failure(&output, &scratch);
}

fn report(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn diagnostics_commands_report_aggregates_only() {
    let scratch = Scratch::new();
    let port = free_port();
    let source = scratch.0.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("synthetic-private-name"), b"preserved bytes").unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/inner"), b"12345").unwrap();
    // A relative source resolves against the working directory.
    let output = run(command(&scratch, port)
        .current_dir(&scratch.0)
        .args(["inspect", "source"]));
    assert!(output.status.success());
    let inspected = report(&output);
    assert_eq!(inspected["status"], "inspected");
    assert_eq!(inspected["operation"], "inspect");
    assert_eq!(
        (inspected["files"].as_u64(), inspected["bytes"].as_u64()),
        (Some(2), Some(20))
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        !text.contains("synthetic-private-name") && !text.contains(scratch.0.to_str().unwrap())
    );

    let output = run(command(&scratch, port).args([Path::new("import"), &source]));
    assert!(output.status.success());
    assert_eq!(report(&output)["status"], "preserved_only");

    for args in [
        vec!["inspect"],
        vec!["unknown-operation", "x"],
        vec!["a", "b", "c"],
    ] {
        let output = run(command(&scratch, port).args(&args));
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert_eq!(
            report(&output),
            serde_json::json!({"version":1,"status":"failed","code":"invalid_or_unavailable_input"})
        );
    }
    // Importing nothing is refused rather than creating an empty library.
    let empty = scratch.0.join("empty");
    fs::create_dir(&empty).unwrap();
    let output = run(command(&scratch, port).args([Path::new("import"), &empty]));
    assert_eq!(output.status.code(), Some(1));
    // A missing source is an aggregate failure, not a path disclosure.
    let output =
        run(command(&scratch, port).args([Path::new("inspect"), &scratch.0.join("missing")]));
    assert_eq!(output.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("missing"));
}
