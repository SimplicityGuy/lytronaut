# Required status checks

Every pull request and `main` push runs these GitHub Actions jobs. Each one
reuses the same Just recipe a developer runs locally, and all of them use
synthetic fixtures only.

| Check (job name) | Workflow | Recipe | Fails when |
|---|---|---|---|
| `check` | CI | `just fmt`, native Clippy, `just test build dependencies licenses` | format, Clippy, tests, builds, dependency boundaries or license notices fail |
| `wasm` | CI | WASM Clippy, `just wasm-build dependencies` | the Rust/WASM client fails Clippy or it or its wasm-bindgen glue does not build |
| `browser` | CI | `just browser-test`, owner lifecycle | the actual service and Rust/WASM client fail a browser flow |
| `persistence-parity` | CI | `just postgres-parity` | any strict PostgreSQL parity case fails |
| `large-bundle` | CI | `just large-bundle-test` | a bundle beyond 4 GiB does not round-trip with exact hashes |
| `coverage` | coverage | `just coverage` | native, WASM or union line coverage is below its floor, a browser script's profile is missing or below its recorded line floor, or the GPU exemption guard fails ([coverage](coverage.md)) |
| `container-amd64`, `container-arm64` | Linux containers | `just container-*`, `just deployment-test` | the image, runtime checks or any Compose deployment/restore combination fails on that native architecture |

## Adopting them as required checks

A workflow failure is visible on the pull request but blocks merging only once
the checks above are marked required on `main`. Repository settings for
SimplicityGuy repositories are managed declaratively in the homelab OpenTofu
configuration (`homelab/infra/opentofu/github`), not by hand and not by this
repository.

1. Confirm all checks are green on the exact `main` commit you are protecting.
2. Confirm rulesets are available: `gh api repos/SimplicityGuy/lytronaut/rulesets`
   returns a JSON list (it returned `[]` on 2026-10-13 after the repository became
   public; while it was private the same call returned 403, a plan limitation).
3. Add a `main` ruleset requiring the job names in the table (exact names, so a
   renamed job does not silently stop being required) through the homelab
   configuration, with its review, saved plan and apply process.
4. Re-run a pull request and confirm the merge box lists each required check.

Until step 3 is applied, required checks are **not** enforced; do not describe
them as enforced. Changing repository visibility, billing or plan is never part
of adopting checks.
