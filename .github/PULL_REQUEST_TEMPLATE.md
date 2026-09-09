## Summary

<!-- What problem does this solve, and how? -->

## Compatibility

<!-- Coordinate-system, ABI (`qrk_*`), WASM envelope, Python, or Expo effects? None is fine. -->

## Checks

- [ ] `cargo fmt --all --check`
- [ ] Tests relevant to this change (note which: `cargo test --workspace --release`, `just python-test`, `just ui-test`, `just qr-lab-abi`, …)
- [ ] Public API documented (`///` rustdoc / Python docstrings / `qrk.h`)
- [ ] `CHANGELOG.md` updated when user-visible
- [ ] Fixtures regenerated only when the generator intentionally changes

Performance-sensitive changes should include before/after measurements, hardware, and the command used. Visible debug UI or Expo changes should include a screenshot or short recording.
