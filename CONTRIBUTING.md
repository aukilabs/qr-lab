# Contributing to QR Lab

Thank you for helping improve QR Lab. This guide covers the shared repository
workflow; component-specific details live in the README inside each component.

Be respectful and constructive in issues, pull requests, and discussions. We
assume good faith and prioritize technical clarity over formality.

Security issues should be reported privately — see [SECURITY.md](SECURITY.md).

## Before you start

- Search the [issue tracker](https://github.com/aukilabs/qr-lab/issues) before
  opening a duplicate bug or feature request.
- For a large feature, public API change, new dependency, or scanner-pipeline
  redesign, open an issue first so the approach can be agreed before substantial
  implementation work begins.
- Keep changes focused. Unrelated refactors make scanner-quality and performance
  regressions harder to review.

## Development setup

Required for the core workspace:

- Git
- Rust 1.87 or newer with Cargo and `rustfmt`
- [`just`](https://just.systems/) for the repository recipes (recommended)

Clone the repository and verify the Rust workspace:

```bash
git clone https://github.com/aukilabs/qr-lab.git
cd qr-lab
rustup show
cargo test --workspace --release
```

Install only the additional tooling needed for the area you are changing:

- **Debug UI / WASM:** Node.js 22.12 or newer, npm, and `wasm-pack`. Running
  `just ui` installs npm dependencies and builds the WASM package.
- **Python:** Python 3.9 or newer and `uv`, or Maturin. Run `just python-test` for
  the isolated wheel integration suite.
- **Android:** Android NDK and `cargo-ndk`.
- **iOS:** macOS, Xcode, and the iOS Rust targets used by
  `scripts/build-native-ios.sh`.

### Golden fixtures

Fixture-driven tests expect generated data under `fixtures/`. Generate once after
clone (deterministic with seed `7`):

```bash
cd tools/fixtures
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
.venv/bin/python generate.py --out ../../fixtures --seed 7
```

See [tools/fixtures/README.md](tools/fixtures/README.md).

## Making changes

Follow the existing module boundaries:

- Put generic image storage in `qr-lab-image`, geometry in `qr-lab-geometry`,
  and reusable processing in `qr-lab-imgproc`.
- Keep QR-specific detection and decoding in `qr-lab-qr`.
- Preserve `qr-lab-core`, the C ABI (`qrk_*` symbols), WASM envelope, and mobile
  compatibility unless a breaking change has been explicitly agreed. See
  [docs/qr-lab/stability.md](docs/qr-lab/stability.md).
- **Document every public item** (`///` rustdoc on crates, structs, fields,
  methods, and free functions). Library crates enable `#![warn(missing_docs)]`.
- Add tests for new behavior and bug fixes.
- Avoid committing generated build output. The `.gitignore` covers the common
  Rust, Node, Python, Android, iOS, and WASM outputs.

Format Rust before submitting:

```bash
cargo fmt --all
```

## Tests and checks

Run the checks relevant to the files you changed. For core Rust changes, the
minimum is:

```bash
just ci                   # format + workspace tests (matches GitHub Actions)
# or:
cargo fmt --all --check
cargo test --workspace --release
```

Additional suites:

```bash
just ui-test              # TypeScript/Vitest unit tests
just ui-build             # WASM build, type-check, production UI build
just python-test          # built-wheel Python integration tests
just qr-lab-deps          # crate boundaries and feature combinations
just qr-lab-abi           # C header and exported-symbol compatibility
./scripts/check-wasm.sh   # default and qr-gen WASM configurations
```

Native changes should also be built for the affected platform. Android
artifacts must pass `just expo-android-check` after `just expo-android`.

### Fixture generator changes

Generator changes should pass:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r tools/fixtures/requirements.txt
python -m pytest tools/fixtures
```

Keep generation deterministic — same seed must produce byte-identical
`.png`, `.luma`, and `.json` output with the pinned requirements.

### Performance-sensitive changes

Scanner, image-processing, allocation, and native-binding changes can affect
latency even when functional tests pass. Compare before and after results in a
release build and include the command, hardware, and measurements in the pull
request. The standard fixture benchmark is:

```bash
just bench
```

See [docs/qr-lab/benchmarks.md](docs/qr-lab/benchmarks.md) for the reference
methodology.

## API documentation

When you add or change a public Rust item:

1. Write a short rustdoc comment explaining intent, units, and failure modes.
2. Prefer examples on high-level entry points (`Scanner`, `Gray8View`, operators).
3. Run `cargo doc -p <crate> --no-deps` and fix any missing-docs warnings.

Python public functions and classes in `bindings/python/python/qr_lab/` should
keep clear docstrings. The C header `crates/qr-lab-ffi/include/qrk.h` should
document parameters, ownership, and free responsibilities.

## Pull requests

A pull request should:

- explain the problem and the chosen solution;
- identify compatibility or coordinate-system effects;
- list the exact checks run and any checks that could not be run;
- include before/after measurements for performance-sensitive work;
- include screenshots or short recordings for visible debug UI or Expo changes;
- update README, API, migration, or stability documentation when behavior or
  supported workflows change;
- add an entry under `Unreleased` in `CHANGELOG.md` for user-visible changes.

### Suggested PR checklist

- [ ] `cargo fmt --all --check`
- [ ] `cargo test --workspace --release` (or the subset that matches the change)
- [ ] Public API documented
- [ ] `CHANGELOG.md` updated when user-visible
- [ ] Fixtures regenerated only when the generator intentionally changes

## License

By submitting a contribution, you agree that it may be distributed under the
repository's [MIT License](LICENSE).
