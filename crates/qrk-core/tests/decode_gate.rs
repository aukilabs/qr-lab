//! Plan 4's decode gates (Global Constraints, gates 1-3):
//!
//! 1. All 81 golden fixtures: every ground-truth code decodes with payload
//!    exactly equal to `codes[].payload`, correct `version`, correct
//!    `mirrored` flag, and per-fixture decoded count == ground-truth count
//!    (no spurious extra decodes).
//! 2. Real captures (`fixtures/real/real_{1,2}.png`, via the `png` dev-dep,
//!    downscaled to max-dim 1280 with a test-local copy of the production
//!    NN formula): `real_1` decodes >=2 codes including both pinned R8.HR
//!    payloads; `real_2` decodes exactly the pinned R8.HR payload.
//!    `fixtures/real/video_f167.png` (Plan 4B: frame 167 of a real store-
//!    walkthrough video, `dmt_recording_2026-01-23_11-33-44.mp4` — the
//!    frame the Fix A/B investigation used as its worked example) decodes
//!    its pinned payload at the same working max-dim 1280.
//! 3. Arbitration: on `multi_*` fixtures the decoded-code count equals the
//!    ground-truth code count (no spurious extras) and every finder
//!    candidate is consumed by at most one decoded code.
//!
//! Per the plan's gate-failure protocol: on failure, report exact miss
//! lists (fixture, code, what went wrong) rather than loosening any
//! assertion.

mod common;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use qrk_core::{detect, luma_from_rgba, LumaView};

// --- Gate 1: all 81 golden fixtures ---

/// Group a fixture name into a reporting bucket by stripping trailing
/// counter suffixes: a literal `_<digits>` counter (`multi_07` -> `multi`,
/// `tilt45_00` -> `tilt45` — the `45` survives because it isn't a separate
/// `_`-delimited segment), and additionally a trailing `_v<digits>` version
/// tag before that (`ver_00_v1` -> `ver_00` -> `ver`). Not a general parser,
/// just enough to produce a readable per-family stats table for this
/// fixture suite's actual naming convention (verified against every real
/// name in `fixtures/*.json` during development).
fn fixture_prefix(name: &str) -> String {
    let mut s = name;
    if let Some(pos) = s.rfind('_') {
        let tail = &s[pos + 1..];
        if tail.len() > 1 && tail.starts_with('v') && tail[1..].bytes().all(|b| b.is_ascii_digit()) {
            s = &s[..pos];
        }
    }
    if let Some(pos) = s.rfind('_') {
        let tail = &s[pos + 1..];
        if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
            s = &s[..pos];
        }
    }
    s.to_string()
}

#[derive(Default)]
struct PrefixStats {
    fixtures: usize,
    codes_expected: usize,
    codes_matched: usize,
}

#[test]
fn gate_1_all_golden_fixtures_decode_exactly() {
    let fixtures = common::load_all();
    assert_eq!(fixtures.len(), 81, "expected exactly 81 golden fixtures in fixtures/");

    let mut failures: Vec<String> = Vec::new();
    let mut stats: BTreeMap<String, PrefixStats> = BTreeMap::new();

    for fx in &fixtures {
        let view = fx.view();
        let det = detect(&view);
        let entry = stats.entry(fixture_prefix(&fx.name)).or_default();
        entry.fixtures += 1;
        entry.codes_expected += fx.codes.len();

        for truth in &fx.codes {
            match det.codes.iter().find(|c| c.payload == truth.payload) {
                Some(c) => {
                    let mut ok = true;
                    if c.version != truth.version {
                        failures.push(format!(
                            "{}: code {:?} version mismatch: got {} want {}",
                            fx.name, truth.payload, c.version, truth.version
                        ));
                        ok = false;
                    }
                    if c.mirrored != truth.mirrored {
                        failures.push(format!(
                            "{}: code {:?} mirrored mismatch: got {} want {}",
                            fx.name, truth.payload, c.mirrored, truth.mirrored
                        ));
                        ok = false;
                    }
                    if ok {
                        entry.codes_matched += 1;
                    }
                }
                None => {
                    failures.push(format!(
                        "{}: code {:?} (v{}) was not decoded ({} candidate decode(s) present: {:?})",
                        fx.name,
                        truth.payload,
                        truth.version,
                        det.codes.len(),
                        det.codes.iter().map(|c| c.payload.as_str()).collect::<Vec<_>>(),
                    ));
                }
            }
        }

        let truth_payloads: std::collections::HashSet<&str> =
            fx.codes.iter().map(|c| c.payload.as_str()).collect();
        for c in &det.codes {
            if !truth_payloads.contains(c.payload.as_str()) {
                failures.push(format!("{}: spurious decode {:?} (not in ground truth)", fx.name, c.payload));
            }
        }

        if det.codes.len() != fx.codes.len() {
            failures.push(format!(
                "{}: decoded count mismatch: got {} want {}",
                fx.name,
                det.codes.len(),
                fx.codes.len()
            ));
        }
    }

    println!("\n=== Gate 1: per-prefix decode stats ===");
    println!("{:<12} {:>9} {:>14} {:>14}", "prefix", "fixtures", "codes_total", "codes_matched");
    for (prefix, s) in &stats {
        println!("{:<12} {:>9} {:>14} {:>14}", prefix, s.fixtures, s.codes_expected, s.codes_matched);
    }

    assert!(
        failures.is_empty(),
        "Gate 1 FAILED ({} issue(s) across {} fixtures):\n{}",
        failures.len(),
        fixtures.len(),
        failures.join("\n")
    );
}

// --- Gate 2: real captures ---

/// Test-local copy of `debug-ui/src/scanner/downscale.ts`'s `downscaleRgba`
/// NN formula (round the destination dims by `maxDim / max(w,h)`, then
/// nearest-neighbor sample with `floor` source indices), ported to a
/// single-channel luma buffer: NN selection picks one source pixel with no
/// blending, so downscaling before or after RGB->luma conversion produces
/// an identical result pixel-for-pixel — this operates on the
/// already-converted luma buffer purely for convenience.
fn downscale_luma_nn(luma: &[u8], w: usize, h: usize, max_dim: usize) -> (Vec<u8>, usize, usize) {
    let longest = w.max(h);
    if max_dim == 0 || longest <= max_dim {
        return (luma.to_vec(), w, h);
    }
    let dst_w = (((w * max_dim) as f64) / longest as f64).round().max(1.0) as usize;
    let dst_h = (((h * max_dim) as f64) / longest as f64).round().max(1.0) as usize;
    let mut out = vec![0u8; dst_w * dst_h];
    for y in 0..dst_h {
        let sy = ((y * h) / dst_h).min(h - 1);
        for x in 0..dst_w {
            let sx = ((x * w) / dst_w).min(w - 1);
            out[y * dst_w + x] = luma[sy * w + sx];
        }
    }
    (out, dst_w, dst_h)
}

fn fixtures_real_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real")
}

/// Load `fixtures/real/<name>.png`, convert to luma (grayscale/gray+alpha
/// passthrough, RGB(A) via the production `luma_from_rgba` — padding a
/// synthetic 255 alpha byte onto plain RGB, since `luma_from_rgba` requires
/// 4-byte pixels), then NN-downscale to max-dim 1280 (the pinned real-
/// capture gate resolution).
fn load_real_capture(name: &str) -> (Vec<u8>, usize, usize) {
    let path = fixtures_real_dir().join(format!("{name}.png"));
    let file = File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().unwrap_or_else(|e| panic!("{}: read_info: {e}", path.display()));
    let mut buf = vec![
        0u8;
        reader
            .output_buffer_size()
            .unwrap_or_else(|| panic!("{}: could not determine output buffer size", path.display()))
    ];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| panic!("{}: next_frame: {e}", path.display()));
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];

    let luma = match info.color_type {
        png::ColorType::Grayscale => bytes.to_vec(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).map(|p| p[0]).collect(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(w * h * 4);
            for px in bytes.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            luma_from_rgba(&rgba, w, h)
        }
        png::ColorType::Rgba => luma_from_rgba(bytes, w, h),
        png::ColorType::Indexed => {
            panic!("{}: indexed PNGs are not handled by this gate's loader", path.display())
        }
    };

    downscale_luma_nn(&luma, w, h, 1280)
}

#[test]
fn gate_2_real_captures_decode_pinned_payloads() {
    const O9MKM: &str = "HTTPS://R8.HR/O9MKM1ZO3W5";
    const YLXFAP: &str = "HTTPS://R8.HR/YLXFAP2B1A8";

    let (luma1, w1, h1) = load_real_capture("real_1");
    let view1 = LumaView::new(&luma1, w1, h1, w1).unwrap();
    let det1 = detect(&view1);
    let payloads1: Vec<&str> = det1.codes.iter().map(|c| c.payload.as_str()).collect();
    println!("real_1 @{w1}x{h1}: decoded {} code(s): {payloads1:?}", det1.codes.len());
    assert!(
        det1.codes.len() >= 2,
        "real_1: expected >=2 decoded codes, got {}: {payloads1:?}",
        det1.codes.len()
    );
    for want in [O9MKM, YLXFAP] {
        assert!(
            payloads1.contains(&want),
            "real_1: expected payload {want:?} among decoded codes, got {payloads1:?}"
        );
    }

    let (luma2, w2, h2) = load_real_capture("real_2");
    let view2 = LumaView::new(&luma2, w2, h2, w2).unwrap();
    let det2 = detect(&view2);
    let payloads2: Vec<&str> = det2.codes.iter().map(|c| c.payload.as_str()).collect();
    println!("real_2 @{w2}x{h2}: decoded {} code(s): {payloads2:?}", det2.codes.len());
    assert_eq!(
        payloads2,
        vec![YLXFAP],
        "real_2: expected exactly [{YLXFAP:?}], got {payloads2:?}"
    );
}

/// Plan 4B: real-video-capture robustness gate. Frame 167 of
/// `dmt_recording_2026-01-23_11-33-44.mp4` (a real store-walkthrough
/// video) — the same sticker as the neighboring frame 171, which already
/// decoded pre-Plan-4B, but frame 167's tile-threshold bits alone carry 12
/// scattered errors against the RS-validated truth (root-cause
/// investigation), too many for v1-L to correct. Fix A (trace honesty) is
/// not directly observable via `detect()`'s return value (it only changes
/// what a `Trace` records on top of the same decode), so this gate pins
/// Fix B (the reference-threshold + sharpening decode round) end to end:
/// this must decode at the same working max-dim (1280) the rest of this
/// gate uses, exactly like `real_1`/`real_2` above.
#[test]
fn gate_2b_video_frame167_decodes_at_working_resolution() {
    const R8HR: &str = "HTTPS://R8.HR/6EQ44PPYZJN";

    let (luma, w, h) = load_real_capture("video_f167");
    let view = LumaView::new(&luma, w, h, w).unwrap();
    let det = detect(&view);
    let payloads: Vec<&str> = det.codes.iter().map(|c| c.payload.as_str()).collect();
    println!("video_f167 @{w}x{h}: decoded {} code(s): {payloads:?}", det.codes.len());
    assert!(
        payloads.contains(&R8HR),
        "video_f167: expected payload {R8HR:?} among decoded codes, got {payloads:?}"
    );
}

// --- Gate 3: arbitration on multi_* fixtures ---

#[test]
fn gate_3_arbitration_on_multi_fixtures() {
    let mut failures: Vec<String> = Vec::new();
    let mut stats: BTreeMap<String, PrefixStats> = BTreeMap::new();
    let mut checked = 0usize;

    for fx in common::load_all() {
        if !fx.name.starts_with("multi_") {
            continue;
        }
        checked += 1;
        let view = fx.view();
        let det = detect(&view);
        let entry = stats.entry(fixture_prefix(&fx.name)).or_default();
        entry.fixtures += 1;
        entry.codes_expected += fx.codes.len();
        entry.codes_matched += det.codes.len().min(fx.codes.len());

        if det.codes.len() != fx.codes.len() {
            failures.push(format!(
                "{}: decoded count {} != ground-truth count {} (payloads decoded: {:?})",
                fx.name,
                det.codes.len(),
                fx.codes.len(),
                det.codes.iter().map(|c| c.payload.as_str()).collect::<Vec<_>>(),
            ));
        }

        let mut seen_finders: std::collections::HashMap<usize, Vec<&str>> = std::collections::HashMap::new();
        for c in &det.codes {
            for &fi in &c.finder_indices {
                seen_finders.entry(fi).or_default().push(&c.payload);
            }
        }
        for (finder_idx, payloads) in &seen_finders {
            if payloads.len() > 1 {
                failures.push(format!(
                    "{}: finder index {finder_idx} consumed by {} decoded codes: {payloads:?}",
                    fx.name,
                    payloads.len()
                ));
            }
        }
    }

    assert!(checked > 0, "no multi_* fixtures found — gate 3 would vacuously pass");

    println!("\n=== Gate 3: per-prefix arbitration stats ===");
    println!("{:<12} {:>9} {:>14} {:>14}", "prefix", "fixtures", "codes_total", "codes_matched");
    for (prefix, s) in &stats {
        println!("{:<12} {:>9} {:>14} {:>14}", prefix, s.fixtures, s.codes_expected, s.codes_matched);
    }

    assert!(
        failures.is_empty(),
        "Gate 3 FAILED ({} issue(s) across {checked} multi_* fixtures):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
