# Quick start

This is the shortest path to a synthetic local Lytronaut instance. It never uses
private photographs.

## Prerequisites

Install [mise](https://mise.jdx.dev/), then run:

```sh
mise trust
mise install
mise exec -- just setup
mise exec -- just demo
```

`just demo` prints a disposable synthetic library and serves the application at
<http://127.0.0.1:3000>. Import the printed directory. On Chrome for macOS, the
separate package control can select a recognized `.lytrolibrary` directly;
browser and platform behavior varies, so directory and ZIP import remain
available. Import is ready for browsing only when the progress display says
“Library ready.” For persistent or remote operation, continue with [Linux operation](operations/linux.md) and
[configuration](configuration.md).

## Verify a checkout

```sh
mise exec -- just check
```

This builds and tests the native service and Rust/WASM client, checks dependency
boundaries, and validates generated legal inventories.

Docker is needed for container deployment and PostgreSQL parity, not for the
SQLite-backed synthetic `just demo` path.
