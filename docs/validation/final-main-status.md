# Final-main delivery status

This page reconciles the delivered application baseline with historical source-bound reports.
It does not rewrite those reports or relabel their runtime evidence.

## Landed source and exact-main checks

- Main commit: `a3d0482896e6ca3361d7f900f0f0417cda20e4ac`
- Tree: `e65fe5975a5769e2bc1b04b8c3fd0f2e390a328b`
- [CI `39167915596`](https://github.com/SimplicityGuy/lytronaut/actions/runs/39167915596): success
- [Coverage `39167915581`](https://github.com/SimplicityGuy/lytronaut/actions/runs/39167915581): success
- [Linux containers `39167915561`](https://github.com/SimplicityGuy/lytronaut/actions/runs/39167915561): success
- PostgreSQL parity: 64 passed, zero failed or ignored
- Coverage: native 9,444/9,823 (96.14%), client 2,783/2,899 (96.00%), union 12,276/12,762 (96.19%) under the unchanged itemized GPU exception

## Acceptance scope retained

The production runtime evidence remains bound to `aa0073f`. Later source changes
only moved a `cfg(test)` module, corrected a test probe, and updated documents;
production item order matches after removing tests, but rebuilt artifact byte
identity is not claimed.

The [fixed A1/B1/B2/A2 run](focus-acceptance.md) passed without retries on the operator-selected loaded
host (load 10–14). Both B runs contained 24 genuine same-body EOF and same-image
decode overlap intervals and at least 19 pending thumbnails. Both baseline A
grid measurements were unavailable, so there is no comparative baseline-grid
pass or quiet-host claim. The initial enabled-focus interaction may supersede
the first derivative; the warm batch is separate.

| Run | Native p95 | Warm p95 | Grid p95 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| A1 baseline | 2.7 ms | 83.5 ms | unavailable | — |
| B1 final | 2.8 ms | 54.2 ms | 62.0 ms | 65.6 ms |
| B2 final | 2.9 ms | 80.9 ms | 76.8 ms | 85.7 ms |
| A2 baseline | 3.1 ms | 79.8 ms | unavailable | — |

The [private final verification](private-corpus-validation.md) passed 109 streams, 320/320 exports, cancellation and
recovery checks, with every original unchanged. The JPEG comparator used a
browser JPEG of the platform-decoded selected AVC frame; it does not establish
raw RGB, archival, or color parity. MOV pixel comparison covered the selected
key frame; other frames were checked for decode, count, geometry, and order. The
sealed test copy was deleted after review. The owner's exact Photos report was:
“great. I already verified the updated pair in Photos. Looked great.”

GPU compositor timing, opaque browser memory, VoiceOver, iOS, and universal HRD
conformance remain outside the accepted proof. The [independently reviewed HRD
result](hrd-validation.md) is deliberately narrow. No publishing or required-check enforcement was
enabled by the delivery.
