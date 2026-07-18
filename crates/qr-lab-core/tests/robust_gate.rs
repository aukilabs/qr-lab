//! Plan 6 robustness-ladder gates.
//!
//! Gate 1 — parity: `scan_robust` with the all-off default config must
//! reproduce `scan`'s codes exactly (payloads AND corners) — the ladder adds
//! provenance, never behavior, when disabled.
//!
//! Gate 2 — no regression: the ROBUST_FAST ladder on the pristine golden
//! suite must decode exactly the ground-truth set on every fixture (no code
//! lost to dedup, no spurious extras added by any rung).
//!
//! Gate 3 — recovery: synthetically degraded frames that the BASELINE
//! scanner provably fails on must decode through the enabled recovery rungs
//! (this is the ladder earning its existence; the degradations mirror the
//! real-video failure modes — global contrast crush, a hard shadow, and a
//! multi-module motion smear for the Van Cittert deblur tier).

mod common;

use qr_lab_core::{scan, scan_robust, LumaView, ScanConfig, ScanOptions};

const OPTS: ScanOptions = ScanOptions {
    max_working_dim: 0,
    refine: false,
};

/// The degraded/video fixture pack is intentionally optional because its
/// generated image and luma assets are kept off the main branch. Developers
/// benchmarking robustness can use the `plan6-robustness` branch (or generate
/// the pack locally); ordinary checkouts retain all synthetic/unit gates.
fn optional_fixture(name: &str) -> Option<common::Fixture> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    (root.join(format!("{name}.json")).exists() && root.join(format!("{name}.luma")).exists())
        .then(|| common::load(name))
}

#[test]
fn gate1_default_config_is_bit_identical_to_scan() {
    for name in ["near_00", "inv_00", "multi_07", "far_03", "tilt45_02"] {
        let f = common::load(name);
        let view = f.view();
        let plain = scan(&view, &OPTS);
        let robust = scan_robust(&view, &OPTS, &ScanConfig::default());
        assert_eq!(robust.variants.len(), 1, "{name}: only baseline may run");
        assert_eq!(robust.codes.len(), plain.codes.len(), "{name}: code count");
        for (r, p) in robust.codes.iter().zip(plain.codes.iter()) {
            assert_eq!(r.code.payload, p.payload, "{name}: payload");
            assert_eq!(r.code.corners, p.corners, "{name}: corners bit-identical");
            // No downscale (max_working_dim: 0) ⇒ source px == working px.
            assert_eq!(r.corners_source, p.corners, "{name}: source-corner mapping");
            assert_eq!(r.stage, 0);
        }
    }
}

#[test]
fn gate2_robust_fast_decodes_the_golden_suite_exactly() {
    let fixtures = common::load_golden();
    assert!(fixtures.len() >= 81);
    let mut failures = Vec::new();
    for f in &fixtures {
        let view = f.view();
        let robust = scan_robust(&view, &OPTS, &ScanConfig::ROBUST_FAST);
        // Every ground-truth payload decodes...
        for truth in &f.codes {
            let hit = robust.codes.iter().any(|c| c.code.payload == truth.payload);
            if !hit {
                failures.push(format!("{}: missing {:?}", f.name, truth.payload));
            }
        }
        // ...and nothing else does (dedup + rung noise check).
        if robust.codes.len() != f.codes.len() {
            failures.push(format!(
                "{}: {} decoded vs {} truth",
                f.name,
                robust.codes.len(),
                f.codes.len()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "robust-fast regressions:\n{}",
        failures.join("\n")
    );
}

/// Scale every pixel's distance from mid-gray by `num/den` (integer math,
/// deterministic) — a global contrast crush like deep glare / fog.
fn crush_contrast(luma: &[u8], num: i32, den: i32) -> Vec<u8> {
    luma.iter()
        .map(|&p| (128 + (p as i32 - 128) * num / den).clamp(0, 255) as u8)
        .collect()
}

#[test]
fn gate3a_contrast_crush_recovers_through_the_ladder() {
    let f = common::load("near_00");
    // 6/128 ≈ 0.047: ink/paper separation (235−25)·6/128 ≈ 9.8 gray levels —
    // under the default CONTRAST_FLOOR (12), so the baseline binarizer skips
    // every tile; over the low-floor rung's 6, so the ladder can reach it.
    let crushed = crush_contrast(&f.luma, 6, 128);
    let view = LumaView::new(&crushed, f.width, f.height, f.width).unwrap();

    let baseline = scan(&view, &OPTS);
    assert!(
        baseline.codes.is_empty(),
        "degradation too weak: baseline decoded {} codes — tighten the crush",
        baseline.codes.len()
    );

    let robust = scan_robust(&view, &OPTS, &ScanConfig::ROBUST_FAST);
    assert_eq!(
        robust.codes.len(),
        1,
        "ladder failed to recover the crushed code; variants: {:#?}",
        robust.variants
    );
    assert_eq!(robust.codes[0].code.payload, f.codes[0].payload);
    assert!(
        robust.codes[0].stage > 0,
        "recovery must come from a rung, not baseline"
    );
}

#[test]
fn gate3b_hard_shadow_fixture_recovers_through_the_ladder() {
    // shadow_04 (strength 0.75, sharp edge crossing the symbol) is a
    // measured baseline failure — the generated fixture equivalent of the
    // real-video partial-shadow failure mode. First green run: recovered by
    // the SauvolaThreshold rung (stage 3).
    let Some(f) = optional_fixture("shadow_04") else {
        return;
    };
    let view = f.view();

    let baseline = scan(&view, &OPTS);
    assert!(
        baseline.codes.is_empty(),
        "shadow_04 unexpectedly decodes at baseline ({} codes) — this gate \
         needs a fixture the baseline fails on",
        baseline.codes.len()
    );

    let robust = scan_robust(&view, &OPTS, &ScanConfig::ROBUST_FAST);
    assert_eq!(
        robust.codes.len(),
        1,
        "ladder failed to recover shadow_04; variants: {:#?}",
        robust.variants
    );
    assert_eq!(robust.codes[0].code.payload, f.codes[0].payload);
    assert!(
        robust.codes[0].stage > 0,
        "recovery must come from a rung, not baseline"
    );
}

/// Horizontal box smear of odd length `len` (integer math, deterministic) —
/// the exact line-PSF model of the Van Cittert deblur rung.
fn hbox_smear(luma: &[u8], w: usize, h: usize, len: usize) -> Vec<u8> {
    let half = (len / 2) as isize;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0u32;
            for k in -half..=half {
                let sx = (x as isize + k).clamp(0, w as isize - 1) as usize;
                sum += luma[y * w + sx] as u32;
            }
            out[y * w + x] = (sum / len as u32) as u8;
        }
    }
    out
}

#[test]
fn gate3c_multi_module_smear_recovers_through_van_cittert() {
    // near_00 has ~7.25 px modules; a 15 px box smear is ~2.07 modules —
    // one step past the baseline scanner's survival edge (the test asserts
    // that edge, not assumes it) and inside the deconvolution tier's
    // theoretical window (the box MTF stays invertible for features wider
    // than the smear; data cells are broadband, so a bracketed Van Cittert
    // inverse recovers decode-grade contrast). E4 measured recovery at
    // 2.0-2.4 modules on ~7 px module codes; deeper smears (mblur2's
    // 2.5-3.5 modules) sit past the MTF sign flip and need Wiener-class
    // inversion (Plan 6 phase 2).
    let f = common::load("near_00");
    let smeared = hbox_smear(&f.luma, f.width, f.height, 15);
    let view = LumaView::new(&smeared, f.width, f.height, f.width).unwrap();

    let baseline = scan(&view, &OPTS);
    assert!(
        baseline.codes.is_empty(),
        "smear too weak: baseline decoded {} codes — lengthen it",
        baseline.codes.len()
    );

    let robust = scan_robust(&view, &OPTS, &ScanConfig::ROBUST_FULL_BENCHMARK);
    assert_eq!(
        robust.codes.len(),
        1,
        "deblur tier failed to recover the smeared code; variants: {:#?}",
        robust.variants
    );
    assert_eq!(robust.codes[0].code.payload, f.codes[0].payload);
    assert!(
        matches!(
            robust.codes[0].variant,
            qr_lab_core::VariantKind::VanCittert { .. }
        ),
        "recovery must come from the Van Cittert rung, got {:?}",
        robust.codes[0].variant
    );
}

#[test]
fn gate3d_deeply_undersampled_code_recovers_through_4x_roi() {
    // lowres_06 is a perspective-rotated v1 code at 1.4 source
    // px/module. Baseline still forms enough finder/triplet evidence to
    // localize it, but its sampled module grid cannot pass Reed-Solomon.
    // The recovery remains ROI-only so easy and ordinary-resolution frames
    // never pay for a 4x whole-frame pass.
    let Some(f) = optional_fixture("lowres_06") else {
        return;
    };
    let view = f.view();

    let baseline = scan(&view, &OPTS);
    assert!(
        baseline.codes.is_empty(),
        "lowres_06 unexpectedly decodes at baseline; replace the fixture"
    );

    let robust = scan_robust(&view, &OPTS, &ScanConfig::ROBUST_FAST);
    assert_eq!(
        robust.codes.len(),
        1,
        "4x ROI failed: {:#?}",
        robust.variants
    );
    assert_eq!(robust.codes[0].code.payload, f.codes[0].payload);
    assert!(
        matches!(
            robust.codes[0].variant,
            qr_lab_core::VariantKind::UpscaledRoi { factor: 4 }
        ),
        "recovery must come from the 4x ROI, got {:?}",
        robust.codes[0].variant
    );
}

#[test]
fn gate4_degraded_suite_ratchet() {
    // Ratchet over the Plan 6 degraded families (robust-full, no early
    // exit): the ladder must keep decoding every code the expectation rules
    // mark decodable, and must not regress below the last green run's
    // measured totals: decode 48 / detect 53 of 61 codes, vs the baseline
    // scanner's 37/45. The 47/53 level dates to experiment E3's
    // evidence-sized morphological-closing shadow estimate and held
    // through the cumulative-pipeline rewrite (plan 6 §9); during that
    // rewrite mblur2_00 (deep-smear family, 0/5 by design) briefly
    // decoded via an unaligned UpscaledRoi{2} crop and flipped back when
    // recovery ROIs became tile-phase-aligned — a rounding-margin
    // single-code flip either way, traded deliberately for +2 real-video
    // frames (f0068, f0309; see the plan §9 numbers). Raising these
    // numbers after a real improvement is expected; lowering them is a
    // regression.
    let degraded: Vec<_> = common::load_all()
        .into_iter()
        .filter(|f| f.degraded)
        .collect();
    if degraded.is_empty() {
        return;
    }
    let (mut decoded, mut detected, mut expected_missing) = (0usize, 0usize, Vec::new());
    let mut total = 0usize;
    for f in &degraded {
        let view = f.view();
        let robust = scan_robust(&view, &OPTS, &ScanConfig::ROBUST_FULL_BENCHMARK);
        for truth in &f.codes {
            total += 1;
            let dec = robust.codes.iter().any(|c| c.code.payload == truth.payload);
            let center = quad_center(&truth.corners_px);
            let edge = mean_edge(&truth.corners_px);
            let det = dec
                || robust.triplet_evidence.iter().any(|p| {
                    let (dx, dy) = (p[0] - center[0], p[1] - center[1]);
                    (dx * dx + dy * dy).sqrt() < 0.75 * edge
                });
            decoded += dec as usize;
            detected += det as usize;
            if truth.expect_decode && !dec {
                expected_missing.push(format!("{}: {}", f.name, truth.payload));
            }
        }
    }
    assert!(
        expected_missing.is_empty(),
        "expected-decodable codes the ladder missed:\n{}",
        expected_missing.join("\n")
    );
    assert!(
        decoded >= 48,
        "degraded decode ratchet: {decoded}/{total} < 48"
    );
    assert!(
        detected >= 53,
        "degraded detect ratchet: {detected}/{total} < 53"
    );
}

fn quad_center(q: &[[f64; 2]; 4]) -> [f64; 2] {
    [
        (q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0,
        (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0,
    ]
}

fn mean_edge(q: &[[f64; 2]; 4]) -> f64 {
    let d = |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    (d(q[0], q[1]) + d(q[1], q[2]) + d(q[2], q[3]) + d(q[3], q[0])) / 4.0
}

#[test]
fn early_exit_fires_on_an_easy_frame() {
    let f = common::load("near_00");
    let view = f.view();
    let robust = scan_robust(&view, &OPTS, &ScanConfig::ROBUST_FAST);
    assert_eq!(
        robust.variants.len(),
        1,
        "easy frame must exit after baseline"
    );
    assert!(robust.early_exited);
    assert!(!robust.budget_exhausted);
}

#[test]
fn gate5_scan_robust_debug_matches_scan_robust_and_captures_the_ladder() {
    // The debug variant must be a pure superset: identical detections,
    // plus the baseline pass's full geometry and exactly one buffer
    // snapshot per variant that ran, in execution order.
    let Some(f) = optional_fixture("shadow_04") else {
        return;
    };
    let view = f.view();
    let cfg = ScanConfig::ROBUST_FULL_BENCHMARK;
    let plain = scan_robust(&view, &OPTS, &cfg);
    let debug = qr_lab_core::scan_robust_debug(&view, &OPTS, &cfg);

    assert_eq!(debug.detections.codes.len(), plain.codes.len());
    for (a, b) in debug.detections.codes.iter().zip(plain.codes.iter()) {
        assert_eq!(a.code.payload, b.code.payload);
        assert_eq!(a.variant, b.variant);
        assert_eq!(a.corners_source, b.corners_source);
    }
    assert_eq!(debug.detections.variants.len(), plain.variants.len());
    assert_eq!(
        debug.snapshots.len(),
        debug.detections.variants.len(),
        "one snapshot per executed variant"
    );
    for (snap, rec) in debug.snapshots.iter().zip(debug.detections.variants.iter()) {
        assert_eq!(snap.kind, rec.kind, "filmstrip order == execution order");
        assert!(snap.width.max(snap.height) <= 320, "thumbnail cap");
        assert_eq!(snap.luma.len(), snap.width * snap.height);
    }
    // The unified candidate union (one-pipeline contract): robust results
    // carry finders/triplets themselves, source px — no separate baseline
    // copy exists or is needed.
    assert!(
        !debug.detections.finders.is_empty(),
        "shadow_04 has finder candidates in the unified union"
    );
    assert!(
        !debug.detections.triplets.is_empty(),
        "shadow_04 decodes, so a triplet must be in the union"
    );
}

/// Load `fixtures/real/<name>.png` at FULL source resolution via the
/// production luma path — the same loader pattern `decode_gate.rs` uses
/// for `video_f167.png` (the committed-PNG precedent: `fixtures/real`
/// PNGs are committed; `.luma`/`.json` sidecars are not).
fn load_real_png_source(name: &str) -> Option<(Vec<u8>, usize, usize)> {
    use std::{fs::File, io::BufReader, path::PathBuf};
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/real")
        .join(format!("{name}.png"));
    let file = File::open(&path).ok()?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| panic!("{}: read_info: {e}", path.display()));
    let mut buf = vec![
        0u8;
        reader.output_buffer_size().unwrap_or_else(|| panic!(
            "{}: could not determine output buffer size",
            path.display()
        ))
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
            qr_lab_core::luma_from_rgba(&rgba, w, h)
        }
        png::ColorType::Rgba => qr_lab_core::luma_from_rgba(bytes, w, h),
        other => panic!("{}: unsupported color type {other:?}", path.display()),
    };
    Some((luma, w, h))
}

#[test]
fn gate6_video_f0070_cross_variant_pooling_recovers_detection_evidence() {
    // Frame 70 of dmt_recording 11-34-54 (1920x1440): the diagnosis D2
    // worked example of the cross-variant pooling gap. At working max-dim
    // 1280 the code sits at ~3.1 working px/module (4.7 source px/module)
    // and its three finders are only ever found by DISJOINT passes — the
    // baseline scan provably groups NOTHING — while the pooled union holds
    // the whole triple. The cumulative pipeline must assemble it. Decode
    // at 4.7 px/module handheld is rounding-margin fragile (D2's half-LSB
    // luma flip), so this gate pins DETECTION evidence, not decode.
    let Some((luma, w, h)) = load_real_png_source("video_f0070") else {
        return;
    };
    let view = LumaView::new(&luma, w, h, w).unwrap();
    let opts = ScanOptions {
        max_working_dim: 1280,
        refine: false,
    };

    let baseline = scan(&view, &opts);
    assert_eq!(
        baseline.codes.len(),
        0,
        "f0070 must not decode at baseline (else this gate needs a harder frame)"
    );
    assert_eq!(
        baseline.triplets.len(),
        0,
        "f0070's pooling-gap premise: no single-variant triplet at 1280"
    );

    let robust = scan_robust(&view, &opts, &ScanConfig::ROBUST_FAST);
    assert!(
        robust.finders.len() >= 3,
        "pooled union must hold the code's three finders, got {}: {:#?}",
        robust.finders.len(),
        robust.finders
    );
    assert!(
        !robust.triplets.is_empty(),
        "the cumulative pipeline must form >=1 unified triplet on f0070; variants: {:#?}",
        robust.variants
    );
}

#[test]
fn gate7_session_cross_frame_pooling_forms_the_triplet_under_rotation() {
    // The temporal analog of gate6: with rung rotation, no single FRAME
    // runs the full detect batch — the substrate/threshold/Sauvola passes
    // land on different frames. Replaying f0070 (a stationary code) through
    // a rotating ScanSession must still assemble the triplet, because each
    // frame's finders seed the next: cross-FRAME pooling reconstructs what
    // cross-VARIANT pooling does within a single full-ladder frame. Period 3
    // ⇒ a full rotation cycle spans 3 frames, so by frame ~4 the seed pool
    // holds every pass's contribution. (A truly moving code would fail the
    // seeds' current-frame leg-module re-validation and simply not pool —
    // the safety property; here the frame is identical each tick.)
    let Some((luma, w, h)) = load_real_png_source("video_f0070") else {
        return;
    };
    let view = LumaView::new(&luma, w, h, w).unwrap();
    let opts = ScanOptions {
        max_working_dim: 1280,
        refine: false,
    };
    let mut session = qr_lab_core::ScanSession::new(
        ScanConfig::ROBUST_FAST,
        qr_lab_core::SessionConfig {
            rotation_period: 3,
            pool_ttl_frames: 4,
        },
    );
    // Scan the same frame several times: rotation cycles the detect groups,
    // cross-frame seeds accumulate, and the triplet must form within a few
    // frames (well inside a real handheld dwell of >0.3 s at 10 fps).
    let mut formed_by = None;
    for f in 0..6 {
        let det = session.scan_frame(&view, &opts);
        if !det.triplets.is_empty() {
            formed_by = Some(f);
            break;
        }
    }
    assert!(
        formed_by.is_some(),
        "cross-frame pooling must form f0070's triplet within 6 rotated frames"
    );
    assert!(
        formed_by.unwrap() <= 4,
        "should form within one rotation cycle + margin, formed at frame {:?}",
        formed_by
    );
}
