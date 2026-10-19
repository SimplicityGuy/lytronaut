# Instrumented sidebar request witness

> Historical source- and date-bound record. See the [delivered application baseline](final-main-status.md) for current status.

This receipt records the follow-up to the actual-main coverage failure. It does
not replace the original `7fde864` evidence of 20 uninstrumented sidebar passes.
All data is public synthetic fixture data, and application logic is unchanged.

## Observed failure

Main `069dec5` coverage run `38969963018` failed at `second-capture`: the selected
grid row, viewer name, image alt text, completed 640 × 360 image, changed stage
and static-ready status matched, but the Puppeteer request-completion fence
never settled. That early exit also caused the coverage floors to fail; the
floors and frozen exceptions were not changed.

Diagnostic-only source `1995de3` retained the original acceptance behavior and
added bounded lifecycle records. Instrumented run `38975833260`, attempt 1,
completed repetitions 1–9 and failed repetition 10 without retry. It observed
one exact post-arm GET object with HTTP 200 headers, `fromCache: false`,
`requestfinished: false`, and `requestfailed: net::ERR_ABORTED`, while the same
real UI assertions were satisfied. Cache or an absent request was therefore
not the observed explanation. This trace proves that a CDP terminal event is
an unsuitable sole witness for this application-consumption assertion; it does
not claim every cancelled request completed successfully.

## Corrected witness

The public test installs a Fetch observer before loading the real Rust/WASM
client. Each selection arms a new token. Only an exact post-arm logical GET
invocation for that capture can contribute a witness; a pre-arm pending Fetch
cannot attach its later result to the new action.

The observer reads a clone, leaving the response branch consumed by Rust
unchanged. The clone must reach EOF, remain within a fixed 256 KiB accumulator,
parse as valid metadata and match the expected capture ID. Only that identity,
invocation number, byte count and focus-role boolean are retained after reading. At most
eight matching invocations may be observed per action; overflow fails closed.
HTTP errors, truncated JSON, wrong identities, excessive bodies, actual body
cancellation and superseded tokens cannot provide a passing witness.

The existing stage-change, selected-row, viewer, exact image geometry, alt text
and ready-status assertions still prove the actual product selection. Thumbnail
and viewer calls share the metadata endpoint, so observing a body alone does not
replace those assertions. Strict Puppeteer completion remains required for
mapped paging and the intentionally held current response; obsolete paging
requests must still abort, and the explicit rejected-body exception remains
limited to error statuses. No timeout was increased and no network retry was
added.

## Validation stages

The diagnostic source passed ten lightweight Node tests. The authored fix adds
seven Fetch-observer regressions, including the observed complete Fetch body
followed by a late CDP abort, strict pre-arm/superseded exclusion and proof that
observer cleanup does not cancel the application response branch. These tests
run through the canonical Node test recipe.

The opt-in coverage input accepts only 0 or 20–50 consecutive instrumented
sidebar suites. A failure stops the loop immediately. The regular full browser
suites and the unchanged 95% coverage gates still run after successful
repetitions; the repetition loop does not replace them.

Final-source instrumented repetition, full coverage and canonical submission are
separate required stages. Their source-keyed receipts, original failure logs and
hash manifests live outside disposable worktrees in delivery evidence. This
authored snapshot does not claim those later stages have already passed.
