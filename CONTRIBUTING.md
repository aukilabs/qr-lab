# Contributing to QRKit

Thank you for helping improve QRKit. This guide covers the shared repository
workflow; component-specific details live in the README inside each component.

## Before you start

- Search the [issue tracker](https://github.com/aukilabs/qrkit/issues) before
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
git clone https://github.com/aukilabs/qrkit.git
cd qrkit
rustup show
cargo test --workspace --release
```

Install only the additional tooling needed for the area you are changing:

- Debug UI/WASM: Node.js 22.12 or newer, npm, and `wasm-pack`. Running
  `just ui` installs npm dependencies and builds the WASM package.
- Python: Python 3.9 or newer and `uv`, or Maturin. Run `just python-test` for
  the isolated wheel integration suite.
- Android: Android NDK and `cargo-ndk`.
- iOS: macOS, Xcode, and the iOS Rust targets used by
  `scripts/build-native-ios.sh`.

## Making changes

Follow the existing module boundaries:

- Put generic image storage in `qrkit-image`, geometry in `qrkit-geometry`,
  and reusable processing in `qrkit-imgproc`.
- Keep QR-specific detection and decoding in `qrkit-qr`.
- Preserve `qrk-core`, C ABI, WASM envelope, and mobile compatibility unless a
  breaking change has been explicitly agreed. See
  [docs/qrkit/stability.md](docs/qrkit/stability.md).
- Document public Rust APIs and add tests for new behavior and bug fixes.
- Avoid committing generated build output. The `.gitignore` covers the common
  Rust, Node, Python, Android, iOS, and WASM outputs.

Run Rust formatting before submitting changes:

```bash
cargo fmt --all
```

## Tests and checks

Run the checks relevant to the files you changed. For core Rust changes, the
minimum is:

```bash
cargo fmt --all --check
cargo test --workspace --release
```

Additional suites:

```bash
just ui-test              # TypeScript/Vitest unit tests
just ui-build             # WASM build, type-check, production UI build
just python-test          # built-wheel Python integration tests
just qrkit-deps           # crate boundaries and feature combinations
just qrkit-abi            # C header and exported-symbol compatibility
./scripts/check-wasm.sh   # default and qr-gen WASM configurations
```

Native changes should also be built for the affected platform. Android
artifacts must pass `just expo-android-check` after `just expo-android`.

### Fixtures

The fixture set is a regression contract, not sample decoration. It is not
committed: every clone regenerates it deterministically with
`tools/fixtures/generate.py --out ../../fixtures --seed 7` (see the
[fixture guide](tools/fixtures/README.md) before changing the generator).
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

See [docs/qrkit/benchmarks.md](docs/qrkit/benchmarks.md) for the reference
methodology.

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

By submitting a contribution, you agree that it may be distributed under the
repository's [MIT License](LICENSE).
