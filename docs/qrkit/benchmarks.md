# QRKit modularization benchmarks

Measured 2026-07-10 on an Apple M2 Max running macOS 15.7.7 with
`rustc 1.95.0`. All scanner measurements used release mode, one frame per
invocation inside the existing fixture harness, 1280 maximum working dimension,
and the generated golden fixture suite (`tools/fixtures/generate.py --seed 7`)
containing 93 expected QR payloads.

## Scanner extraction comparison

Baseline revision: `bbac779` (immediately before QRKit modularization).

Command shape:

```bash
cargo run --release -p qrk-bench --bin qrk-bench -- \
  --config baseline --config robust-fast --max-dim 1280 --quiet
```

The old and new binaries were run in alternating order to reduce thermal and
scheduler bias. Values below are medians across repeated complete-suite runs.

| Scanner | Metric | Before | QRKit | Change |
|---|---:|---:|---:|---:|
| baseline | mean | 4.298 ms | 4.410 ms | +2.62% |
| baseline | p95 | 6.774 ms | 6.844 ms | +1.03% |
| robust-fast | mean, 10 alternating runs | 5.243 ms | 5.282 ms | +0.74% |
| robust-fast | p95, 10 alternating runs | 8.212 ms | 8.342 ms | +1.58% |

Both builds detected and decoded 93/93 expected codes with no output regression.
The result is within the plan's 3% extraction tolerance.

## Standalone operator benchmark

Command:

```bash
cargo run -p qrkit-imgproc --release --example operator_bench
```

The benchmark uses a 1280x720 Gray8 image, a length-7 line PSF, three Van
Cittert iterations, ten warmups, and fifty measured calls with one reused output
and `DeblurWorkspace`.

| Operator | Mean | p50 | p95 |
|---|---:|---:|---:|
| Van Cittert line restoration | 13.742 ms | 13.672 ms | 14.358 ms |

This is a scalar host measurement, not a mobile latency promise. The benchmark
prints its full protocol so device-specific runs can be compared honestly.

## Quality gates

`crates/qrkit-imgproc/tests/deblur_quality.rs` independently applies known line
PSFs to procedural barcode stripes and a checker marker. The matched restoration
must increase recovered foreground/background amplitude. The existing robust QR
gate separately requires a multi-module-smear frame that baseline scanning cannot
decode to recover through the Van Cittert ladder variant.

Pixel MAE is retained as a diagnostic but is not the acceptance metric for these
binary patterns: inverse filtering deliberately overshoots edges, so MAE can
worsen while threshold separation and downstream decoding improve.

## Cross-target build evidence

- iOS: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, and
  `x86_64-apple-ios` release-mobile static libraries built successfully.
- Android API 24: arm64-v8a and x86_64 release-mobile shared libraries built
  successfully with NDK 28.0.13004108.
- Android LOAD alignment: every segment reported `0x4000` (16 KB).
- WASM: default and `qr-gen` feature configurations passed
  `scripts/check-wasm.sh`.

## Python binding overhead

Measured 2026-07-11 on the same Apple M2 Max with CPython 3.12.7, NumPy
2.2.6, and the release `aukilabs-qrkit` wheel. Timings use
`time.perf_counter_ns`, ten scanner warmups followed by 50 measured calls, and
five operator warmups followed by 20 measured calls.

| Python call | Input | Mean | p50 | p95 |
|---|---:|---:|---:|---:|
| `scan(..., preset="robust_fast")` | committed `near_00`, 1280x720 | 5.410 ms | 5.384 ms | 5.696 ms |
| `ImageProcessor.van_cittert(..., len=7, iterations=3)` | same 1280x720 luma | 16.384 ms | 16.317 ms | 17.099 ms |

These are end-to-end Python calls: they include the race-safe NumPy input copy,
GIL detach/reattach, native execution, output array creation, and scanner-result
conversion into Python dictionaries. The scanner decoded the expected payload
on every measured call.
