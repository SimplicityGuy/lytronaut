# Deployment access boundary

Lytronaut opens directly into the library workspace. There are no application
accounts, passwords, bootstrap setup, sessions or authentication endpoints.
The operator owns access control: the Compose example binds to loopback, while
private ingress or an authenticating reverse proxy may enforce an external policy.
Historical source-library `public=1` flags have no effect on that policy.

Every HTTP path, including assets and health, requires the exact Host authority
configured by `LYTRONAUT_ORIGIN`. A supplied Origin must match that origin;
mutations always require both that Origin and `X-Lytronaut-Intent: mutation`.
The header is explicit request intent, not a credential. Browser forms cannot
supply it, no CORS policy grants cross-origin requests, and cross-site Fetch
metadata fails closed. Non-browser clients use the same explicit headers:

```sh
curl -H 'Origin: http://127.0.0.1:3000' \
  -H 'X-Lytronaut-Intent: mutation' -H 'Content-Type: application/json' \
  --data '{"version":1,"kind":"directory","declared_bytes":1024,"declared_files":1}' \
  http://127.0.0.1:3000/imports
```

Forwarded identity, host and origin headers are never trusted. A reverse proxy
must preserve the configured external Host and Origin and enforce its own access
policy before forwarding requests. HTTPS enables browser secure-context APIs;
loopback HTTP also permits local use. Plain HTTP on non-loopback origins is
rejected by configuration. All responses are no-store with nosniff and
no-referrer headers. Paths and queries retain bounded validation; streaming
handlers additionally enforce encoded bytes, metadata and storage budgets.

Opening an existing application catalog removes obsolete account/session/attempt
tables transactionally. Startup unlinks the fixed `bootstrap.secret` artifact
from the application's private data directory without following symlinks; a
directory at that filename fails closed. Source-library databases and immutable
originals are never passed to this migration. No credential artifact is recreated.

Current backup bundles contain no authentication tables. Legacy bundles accept
only the exact known schema, including the three old table definitions, then
scrub those tables in the isolated restore copy after integrity and association
checks. Unexpected views, triggers, generated columns, virtual tables or schema
changes still fail closed. A restored store requires no administrator setup.

See [Linux operation](../operations/linux.md), [ADR 002](../design/002-deployment-access.md)
and the actual HTTP regression tests in `crates/lytronaut-service/tests/security.rs`.
