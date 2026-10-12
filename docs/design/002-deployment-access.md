# ADR 002: Access control belongs to deployment

Status: accepted by the operator on 2026-10-12 UTC.

The original implementation plan inferred a required single-administrator login
for this self-hosted personal library. The operator did not request accounts and
explicitly selected: “Remove built-in login; use deployment-level access control.”

Lytronaut will open directly into the library application. It will not provide
account registration, administrator-password onboarding, a login screen, or
application-managed authentication sessions. Deployment owns access control,
including private ingress or reverse-proxy authentication when configured by the
operator. This does not introduce a replacement mandatory account flow.

The existing packaging bead, **lytronaut-j2u — Remove built-in login and package
CPU-only Linux self-hosting**, implements the correction centrally: native and
Rust/WASM application entry, obsolete authentication state and dependencies,
existing-store migration, backup/restore schema, browser/API tests, documentation
and both Linux runtime checks. The active GPU, codec and export developers retain
their feature scope while this integration correction is pending. The historical
authentication bead remains recorded as merged work whose product requirement
has been superseded; its history is not erased.

Meaningful Host/Origin and mutation-boundary protections remain appropriate for
the deployment contract and must work without a login-dependent credential.
Forwarded identity headers do not implicitly become trusted authentication.
Source libraries, immutable originals, catalog associations and exports remain
unchanged by this decision.

**lytronaut-cjq — Validate the supported MVP and document private self-hosted
operation** verifies the final login-free product, reconciles earlier assumptions
and measures the required 95% native plus actual Rust/WASM application line
coverage. Removing accounts is an explicit product-scope correction, not a
coverage exclusion. All remaining application behavior still requires meaningful
tests and passing GitHub Actions on the final revision.
