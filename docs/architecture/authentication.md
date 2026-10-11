# Local credentials and authenticated HTTP boundary

Lytronaut has one administrator account. There is no registration, anonymous
album/asset sharing, password recovery endpoint, telemetry or source-library
access in this authentication layer. Historical `public=1` flags have no effect.
Authentication does not claim that upload/catalog/viewer features exist yet.

## First run and restart

`just serve` binds only `127.0.0.1:3000`. `LYTRONAUT_DATA` names the application
state directory (default `data`, ignored by git); it must be a real mode-0700
private directory on Unix. The application creates mode-0600 `catalog.db` and
`bootstrap.secret`. Existing permissive files or symlinks fail closed; it never
changes permissions on imported libraries. Windows ACL support is not yet
implemented and startup fails closed on non-Unix systems.

The bootstrap secret is 32 OS-random bytes encoded as 64 lowercase hexadecimal
characters. Read it locally, never put it in a URL, source capture, routine log,
command-line argument, or shared browser storage. Submit JSON
`{"bootstrap":"<local secret>","username":"<name>","password":"<password>"}`
to `POST /auth/setup`, with `Origin: http://127.0.0.1:3000` and
`X-Lytronaut-Intent: setup`. Names use 1–64 ASCII letters/digits/underscore/hyphen;
passwords use 12–1024 UTF-8 bytes. The browser UI will provide this flow in its
own bead; this service exposes the HTTP contract only.

Setup returns 204 once. SQLite atomically installs an Argon2id PHC hash and a
SHA-256 username hash, and consumes the bootstrap hash. Racing or repeated setup
cannot overwrite the administrator. Setup denial uses the same redacted 401
response. There is no remote reset. Keep the application database when restarting.
Pending setup must use the same local bootstrap file; a missing/mismatched or
partially written file fails closed. If interrupted before creating a pending
authentication record, stop the service and remove an empty secret file locally
before restarting. Once a pending hash exists, do not delete or replace the file:
restore its original secret from local state/backup. Never delete a populated
catalog to fix authentication.

After successful setup, the file is no longer read or trusted on restart and
its content can be removed locally. Its value cannot authenticate again.
Backups contain password and session hashes and require the same private access
policy as the live catalog; no session plaintext is persisted.

## Browser/session conventions

`POST /auth/login` accepts JSON `{"username":"<name>","password":"<password>"}`,
exact `Origin`, and `X-Lytronaut-Intent: login`. Wrong username and wrong password
both run Argon2id verification and return identical redacted 401 JSON. Malformed
or overlarge credentials are rejected without reflecting input. Authentication
bodies are capped at 4096 bytes. Argon2id defaults are v19, 19456 KiB memory,
two iterations and one lane; hashing runs off the async executor and is serialized.

There are at most five setup attempts and five login attempts, respectively, in
each 60-second global window. Budgets reserve attempts atomically before hashing,
include successful attempts, persist across restart, and do not depend on IP,
username or forwarded headers. Limit responses are 429 with `Retry-After: 60`.
A global budget suits a single local administrator; an exposed deployment also
needs edge admission limits to limit denial of service and connections.

Login returns 204 with a freshly generated opaque 256-bit session cookie;
client-supplied cookies are never adopted. Only SHA-256 token hashes are stored.
Sessions expire after eight hours with no sliding refresh; at most 64 durable
sessions remain. `GET /auth/session` returns `{version:1,csrf:"<token>"}` for the
current session. Keep its CSRF token in browser memory. It is derived from the
HttpOnly bearer token with a versioned domain prefix and remains usable across
restart. Do not use localStorage or URLs for either token.

All state-changing session requests require the configured Origin and
`X-Lytronaut-Csrf` matching this session. `POST /auth/logout` durably deletes the
session hash and expires the cookie; subsequent replay returns 401. Cookies are
`Path=/; HttpOnly; SameSite=Strict`, without Domain. Exact loopback HTTP uses
`lytronaut_session`; HTTPS uses `__Host-lytronaut_session; Secure`. Requests and
responses use no CORS allowlist. Responses, including failures, use `no-store`,
`nosniff`, and `no-referrer`. No auth response contains account names, identifiers,
paths, serials or underlying errors.

## Origin and reverse proxy policy

`LYTRONAUT_ORIGIN` is one exact origin with no trailing slash/path/query/userinfo.
Default is `http://127.0.0.1:3000`. Plain HTTP is accepted only for literal loopback
hosts (`127.0.0.1`, `localhost`, `[::1]`). Production exposure uses an HTTPS origin,
for example `https://library.example`, with TLS terminated by a local reverse
proxy forwarding to loopback. The proxy must preserve this external Host and
Origin, and must not rewrite cookie attributes. Its host admission policy must
allow only the configured host. The service never trusts `Forwarded`,
`X-Forwarded-Host`, `X-Forwarded-Proto` or an asserted client IP. These headers
cannot switch cookie security or defeat Host/origin checks. The operator controls
the local proxy; do not expose the plaintext backend port.

Host must exactly match the configured authority on every request (including
health). If an Origin is present, it must exactly match. Mutations require Origin;
`Sec-Fetch-Site: cross-site` and `same-site` fail closed. CLI clients must supply
the same headers as the browser. Public health is exact `GET /healthz` and contains
only generic readiness/capability state; all other paths/methods, including HEAD,
unknown paths, original/download/upload/job families and fallbacks, require a
valid session. Setup/login are exact POST exceptions with custom intent headers.

## Extension contract for API and storage development

Build product routes as `Router<AppState>` and pass them to
`router_with_routes(state, application_routes)`. The final constructor installs
one guard over the complete router and fallback; `router(state)` is the empty
product equivalent. Never append routes or merge an unguarded router after this
constructor. Product handlers can extract `Extension<Authenticated>`; possession
of an opaque source ID, database album public flag or capability DTO is never
an authorization decision. Auth tables have their own `auth_` namespace inside
the application catalog and must be retained by storage migrations.

The boundary rejects path percent encoding, backslashes and literal dot/dot-dot
segments after authentication. Route paths use plain opaque IDs from the shared
contract; values may be encoded in query parameters. Storage must still enforce
its own containment rules before filesystem access, checked lengths, safe archive
entries and immutable-source policy. State mutations require CSRF automatically,
including routes not implemented when this bead landed.

Real loopback HTTP tests cover protected extensions and fallback families,
bootstrap race/replay, local-file permissions/restart/corruption, credential
response equality, durable throttle/session restart, fixation avoidance,
CSRF/logout/expiry, traversal and HTTP/TLS cookie and proxy policy. No personal
capture or external service is involved.

APIs verified against primary upstream documentation:
[Axum middleware](https://docs.rs/axum/0.8.9/axum/middleware/fn.from_fn_with_state.html),
[RustCrypto Argon2 0.5.3](https://docs.rs/argon2/0.5.3/argon2/),
and [getrandom 0.4.3](https://docs.rs/getrandom/0.4.3/getrandom/fn.fill.html).
Argon2 0.5.3 retains its stable password-hash 0.5 API (0.6 is available); getrandom
0.4.3 shares the existing workspace graph and its OS-fill API. These are deliberate
version pins, with the entire resolved graph and notices checked by `just licenses`.

The later client bead may use `router_with_client_assets(state, product_routes,
ClientAssets { html, javascript, wasm, css })` to serve trusted build output
without login. Only exact GET/HEAD `/`, `/client/lytronaut_client.js`,
`/client/lytronaut_client_bg.wasm`, and `/client/style.css` are exempt. Assets are
in-memory generated application bytes with fixed content types; there is no
public directory mount, arbitrary public-route API, or source-file path. Never
populate `ClientAssets` from uploads or the library. All other `/client/*` and
product routes/fallbacks remain guarded. This enables the future login/bootstrap
UI to load while original and derivative pixels remain private.

The 4096-byte `DefaultBodyLimit` is the default for buffered extractors. A future
bounded upload method may attach a route-local
`DefaultBodyLimit::max(explicit_limit)` or `DefaultBodyLimit::disable()` BEFORE
passing its product router into the final constructor. This does not exempt
authentication or CSRF. Raw streaming `Body` does not enforce this extractor
limit: upload handlers must independently count actual streamed bytes and enforce
storage admission/concurrency/file-count limits, aborting without partial product
state on excess. This bead provides no upload handler or fake upload capability.

Client startup wiring: build with `just wasm-build`; supply only the generated
`dist/lytronaut_client.js` and `dist/lytronaut_client_bg.wasm` bytes, plus the
application-owned HTML and CSS, to `ClientAssets`. Reference the module at
`/client/lytronaut_client.js`; wasm-bindgen resolves its adjacent WASM filename
at `/client/lytronaut_client_bg.wasm`. Call this constructor in service startup
in place of `router` when the client is implemented. No asset directory browsing
or imported source path is involved.
