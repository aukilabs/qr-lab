# QR Lab developer documentation

Current reference:

- [`calibrated-scanning.md`](calibrated-scanning.md) — known camera intrinsics,
  lens distortion, source-coordinate corners, and validation
- [`fisheye-stress-test-2026-10-16.md`](fisheye-stress-test-2026-10-16.md) — distortion,
  angle and edge sweep, measured decoding/corner limits, reproducible harness
- [`architecture-decisions.md`](architecture-decisions.md) — package,
  coordinate, allocation, compatibility, and binding decisions
- [`migration.md`](migration.md) — existing `qr-lab-core` users and new modular
  consumers
- [`stability.md`](stability.md) — stable APIs vs internal details
- [`benchmarks.md`](benchmarks.md) — quality, latency, and cross-target evidence
- [`../../bindings/python/README.md`](../../bindings/python/README.md) —
  Python/NumPy installation and usage

Historical design notes and implementation plans (not current API docs) live
under [`../superpowers/`](../superpowers/).
