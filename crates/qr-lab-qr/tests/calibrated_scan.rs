use qr_lab_geometry::PerspectiveTransform;
use qr_lab_image::Size;
use qr_lab_qr::{
    scan, scan_calibrated, CalibratedScanner, CameraCalibration, CameraIntrinsics, LensDistortion,
    LumaView, ScanOptions,
};

const SIZE: Size = Size::new(641, 479);
const QUAD: [[f64; 2]; 4] = [
    [350.0, 115.0],
    [540.0, 150.0],
    [520.0, 340.0],
    [330.0, 305.0],
];

fn calibration(model: LensDistortion) -> CameraCalibration {
    CameraCalibration::new(
        SIZE,
        CameraIntrinsics {
            fx: 230.0,
            fy: 235.0,
            cx: 320.0,
            cy: 239.0,
        },
        model,
    )
    .unwrap()
}

// Independent inverse projection used only by the synthetic camera. Production
// never calls this or inversely resamples a pre-rendered pinhole QR image.
fn unproject(c: &CameraCalibration, p: [f64; 2]) -> Option<[f64; 2]> {
    let k = c.intrinsics();
    let (x, y) = ((p[0] - k.cx) / k.fx, (p[1] - k.cy) / k.fy);
    let rd = x.hypot(y);
    let scale = match c.distortion() {
        LensDistortion::None => 1.0,
        LensDistortion::Fisheye(d) => {
            let polynomial = |t: f64| {
                t + d[0] * t.powi(3) + d[1] * t.powi(5) + d[2] * t.powi(7) + d[3] * t.powi(9)
            };
            if rd >= polynomial(std::f64::consts::FRAC_PI_2) {
                return None;
            }
            let theta = if d == [0.0; 4] {
                rd
            } else {
                let (mut lo, mut hi) = (0.0, std::f64::consts::FRAC_PI_2);
                for _ in 0..40 {
                    let mid = (lo + hi) / 2.0;
                    if polynomial(mid) < rd {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                (lo + hi) / 2.0
            };
            if rd < 1e-12 {
                1.0
            } else {
                theta.tan() / rd
            }
        }
        LensDistortion::BrownConrady([k1, k2, p1, p2, k3]) => {
            assert_eq!([p1, p2], [0.0; 2]);
            // Test models are monotonic: invert their radial polynomial.
            let polynomial = |r: f64| r + k1 * r.powi(3) + k2 * r.powi(5) + k3 * r.powi(7);
            let (mut lo, mut hi) = (0.0, rd.max(1.0));
            while polynomial(hi) < rd {
                hi *= 2.0;
            }
            for _ in 0..40 {
                let r = (lo + hi) / 2.0;
                if polynomial(r) < rd {
                    lo = r;
                } else {
                    hi = r;
                }
            }
            if rd < 1e-12 {
                1.0
            } else {
                (lo + hi) / (2.0 * rd)
            }
        }
    };
    let r = c.rectified_intrinsics();
    Some([r.fx * x * scale + r.cx, r.fy * y * scale + r.cy])
}

fn project_truth(c: &CameraCalibration, p: [f64; 2]) -> [f64; 2] {
    let r = c.rectified_intrinsics();
    let (x, y) = ((p[0] - r.cx) / r.fx, (p[1] - r.cy) / r.fy);
    let radius = x.hypot(y);
    let scale = match c.distortion() {
        LensDistortion::None => 1.0,
        LensDistortion::Fisheye(d) => {
            if radius == 0.0 {
                1.0
            } else {
                let t = radius.atan();
                (t + d[0] * t.powi(3) + d[1] * t.powi(5) + d[2] * t.powi(7) + d[3] * t.powi(9))
                    / radius
            }
        }
        LensDistortion::BrownConrady(d) => {
            1.0 + d[0] * radius.powi(2) + d[1] * radius.powi(4) + d[4] * radius.powi(6)
        }
    };
    let k = c.intrinsics();
    [k.cx + k.fx * x * scale, k.cy + k.fy * y * scale]
}

// Supersample the QR's actual curved footprint, with integer-centered pixels.
// Row padding deliberately differs from the image background.
fn render(
    c: &CameraCalibration,
    quads: &[[[f64; 2]; 4]],
    version: i16,
    inverted: bool,
) -> (Vec<u8>, usize, Vec<String>) {
    let stride = SIZE.width + 7;
    let payloads: Vec<_> = (0..quads.len()).map(|i| format!("lens-qr-{i}")).collect();
    let codes: Vec<_> = payloads
        .iter()
        .map(|p| {
            qrcode::QrCode::with_version(
                p.as_bytes(),
                qrcode::Version::Normal(version),
                qrcode::EcLevel::M,
            )
            .unwrap()
        })
        .collect();
    let transforms: Vec<_> = quads
        .iter()
        .map(|q| PerspectiveTransform::square_to_quad(*q).unwrap().inverse())
        .collect();
    let mut pixels = vec![93; stride * SIZE.height];
    for y in 0..SIZE.height {
        for x in 0..SIZE.width {
            let mut light = 0;
            for dy in [-0.375, -0.125, 0.125, 0.375] {
                for dx in [-0.375, -0.125, 0.125, 0.375] {
                    let dark = unproject(c, [x as f64 + dx, y as f64 + dy]).is_some_and(|p| {
                        transforms.iter().zip(&codes).any(|(inv, qr)| {
                            let [u, v] = inv.map(p[0], p[1]);
                            (0.0..1.0).contains(&u)
                                && (0.0..1.0).contains(&v)
                                && qr[(
                                    (u * qr.width() as f64) as usize,
                                    (v * qr.width() as f64) as usize,
                                )] == qrcode::Color::Dark
                        })
                    });
                    light += u32::from(dark == inverted);
                }
            }
            pixels[y * stride + x] = ((light * 255 + 8) / 16) as u8;
        }
    }
    (pixels, stride, payloads)
}

#[test]
fn decodes_perspective_and_lens_distortion_with_source_corners() {
    for (model, version, inverted) in [
        (LensDistortion::None, 2, false),
        (LensDistortion::Fisheye([0.0; 4]), 1, false),
        (LensDistortion::Fisheye([0.0; 4]), 3, true),
        (
            LensDistortion::Fisheye([0.04, -0.005, 0.001, 0.0]),
            3,
            false,
        ),
        (
            LensDistortion::BrownConrady([0.18, 0.0, 0.0, 0.0, 0.0]),
            3,
            false,
        ),
        (
            LensDistortion::BrownConrady([-0.12, 0.025, 0.0, 0.0, 0.0]),
            3,
            false,
        ),
    ] {
        let c = calibration(model);
        let (pixels, stride, payloads) = render(&c, &[QUAD], version, inverted);
        let frame = LumaView::new(&pixels, SIZE.width, SIZE.height, stride).unwrap();
        let truth = QUAD.map(|p| project_truth(&c, p));
        for cap in [0, 420] {
            let result = scan_calibrated(
                &frame,
                &ScanOptions {
                    max_working_dim: cap,
                    refine: true,
                },
                &c,
            )
            .unwrap();
            assert_eq!(result.codes.len(), 1, "{model:?}, v{version}, cap={cap}");
            let code = &result.codes[0];
            assert_eq!(code.payload, payloads[0]);
            assert_eq!(code.payload_bytes, payloads[0].as_bytes());
            assert_eq!(code.inverted, inverted);
            let refined = code.refined_corners_source.expect("refinement ran");
            assert!(code.corner_refined.iter().all(|r| *r));
            let max_error = refined
                .iter()
                .zip(truth)
                .map(|(p, t)| (p[0] - t[0]).hypot(p[1] - t[1]))
                .fold(0.0, f64::max);
            eprintln!("{model:?} v{version} cap={cap}: max corner error {max_error:.4}px");
            assert!(
                max_error < 0.6,
                "{model:?}, v{version}, cap={cap}, error={max_error}"
            );
        }
    }
}

#[test]
fn corrected_field_of_view_multicode_and_map_reuse() {
    let c = calibration(LensDistortion::Fisheye([0.0; 4]))
        .with_rectified_intrinsics(CameraIntrinsics {
            fx: 170.0,
            fy: 180.0,
            cx: 320.0,
            cy: 239.0,
        })
        .unwrap();
    let other = QUAD.map(|[x, y]| [x - 290.0, y + 10.0]);
    let (pixels, stride, payloads) = render(&c, &[QUAD, other], 2, false);
    let view = LumaView::new(&pixels, SIZE.width, SIZE.height, stride).unwrap();
    let mut scanner = CalibratedScanner::new(
        c,
        ScanOptions {
            max_working_dim: 500,
            refine: true,
        },
    )
    .unwrap();
    for _ in 0..2 {
        let result = scanner.scan(&view).unwrap();
        assert_eq!(result.codes.len(), 2);
        for (payload, quad) in payloads.iter().zip([QUAD, other]) {
            let code = result.codes.iter().find(|c| &c.payload == payload).unwrap();
            let refined = code.refined_corners_source.unwrap();
            for (p, t) in refined.iter().zip(quad.map(|p| project_truth(&c, p))) {
                assert!((p[0] - t[0]).hypot(p[1] - t[1]) < 0.6);
            }
        }
    }
}

#[test]
fn pinhole_no_refinement_and_wrong_frame_size() {
    let c = calibration(LensDistortion::None);
    let (pixels, stride, _) = render(&c, &[QUAD], 2, false);
    let view = LumaView::new(&pixels, SIZE.width, SIZE.height, stride).unwrap();
    let options = ScanOptions::default();
    let ordinary = scan(&view, &options);
    let calibrated = scan_calibrated(&view, &options, &c).unwrap();
    assert_eq!(ordinary.codes.len(), 1);
    assert_eq!(calibrated.codes.len(), 1);
    assert_eq!(
        ordinary.codes[0].payload_bytes,
        calibrated.codes[0].payload_bytes
    );
    assert_eq!(
        ordinary.codes[0].corners,
        calibrated.codes[0].corners_source
    );
    assert!(calibrated.codes[0].refined_corners_source.is_none());
    let wrong = LumaView::new(&pixels, SIZE.width - 1, SIZE.height, stride).unwrap();
    assert!(scan_calibrated(&wrong, &options, &c).is_err());
    let mut reusable = CalibratedScanner::new(c, options).unwrap();
    assert!(reusable.scan(&wrong).is_err());
    let blank = vec![255; SIZE.width * SIZE.height];
    let blank = LumaView::new(&blank, SIZE.width, SIZE.height, SIZE.width).unwrap();
    assert!(reusable.scan(&blank).unwrap().codes.is_empty());
}

#[test]
fn mirrored_fisheye_payload_is_preserved() {
    let c = calibration(LensDistortion::Fisheye([0.0; 4]));
    let mirrored = [QUAD[0], QUAD[3], QUAD[2], QUAD[1]];
    let (pixels, stride, payloads) = render(&c, &[mirrored], 2, false);
    let view = LumaView::new(&pixels, SIZE.width, SIZE.height, stride).unwrap();
    let result = scan_calibrated(&view, &ScanOptions::default(), &c).unwrap();
    assert_eq!(result.codes.len(), 1);
    assert_eq!(result.codes[0].payload, payloads[0]);
    assert!(result.codes[0].mirrored);
}

#[test]
fn higher_version_alignment_regions_use_calibrated_source_samples() {
    let c = calibration(LensDistortion::Fisheye([0.0; 4]));
    let quad = [[220.0, 55.0], [590.0, 85.0], [550.0, 430.0], [180.0, 400.0]];
    let (pixels, stride, payloads) = render(&c, &[quad], 7, false);
    let view = LumaView::new(&pixels, SIZE.width, SIZE.height, stride).unwrap();
    let result = scan_calibrated(
        &view,
        &ScanOptions {
            max_working_dim: 0,
            refine: true,
        },
        &c,
    )
    .unwrap();
    assert_eq!(result.codes.len(), 1);
    let code = &result.codes[0];
    assert_eq!(code.payload, payloads[0]);
    assert_eq!(code.version, 7);
    for (p, t) in code
        .refined_corners_source
        .unwrap()
        .iter()
        .zip(quad.map(|p| project_truth(&c, p)))
    {
        assert!((p[0] - t[0]).hypot(p[1] - t[1]) < 0.6);
    }
}

#[test]
#[ignore = "manual release benchmark; run with --ignored --nocapture --test-threads=1"]
fn calibrated_latency() {
    use std::{hint::black_box, time::Instant};
    let options = ScanOptions {
        max_working_dim: 420,
        refine: true,
    };
    for model in [
        LensDistortion::None,
        LensDistortion::Fisheye([0.0; 4]),
        LensDistortion::BrownConrady([0.18, 0.0, 0.0, 0.0, 0.0]),
    ] {
        let c = calibration(model);
        let (pixels, stride, _) = render(&c, &[QUAD], 3, false);
        let view = LumaView::new(&pixels, SIZE.width, SIZE.height, stride).unwrap();
        let build = Instant::now();
        let mut calibrated = CalibratedScanner::new(c, options).unwrap();
        let build_ms = build.elapsed().as_secs_f64() * 1000.0;
        for _ in 0..5 {
            black_box(scan(&view, &options));
            black_box(calibrated.scan(&view).unwrap());
        }
        let mut raw_ms = Vec::new();
        let mut calibrated_ms = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            black_box(scan(black_box(&view), &options));
            raw_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            black_box(calibrated.scan(black_box(&view)).unwrap());
            calibrated_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        raw_ms.sort_by(f64::total_cmp);
        calibrated_ms.sort_by(f64::total_cmp);
        let raw = scan(&view, &options);
        let corrected = calibrated.scan(&view).unwrap();
        let truth = QUAD.map(|p| project_truth(&c, p));
        let error = |corners: [[f64; 2]; 4]| {
            corners
                .iter()
                .zip(truth)
                .map(|(p, t)| (p[0] - t[0]).hypot(p[1] - t[1]))
                .fold(0.0, f64::max)
        };
        eprintln!("{model:?}: map={build_ms:.3}ms, raw p50/p95={:.3}/{:.3}ms ({} codes, max error {:?}), calibrated p50/p95={:.3}/{:.3}ms ({} codes, max error {:?})",
            raw_ms[50], raw_ms[95], raw.codes.len(), raw.codes.first().and_then(|c| c.refined_corners).map(error),
            calibrated_ms[50], calibrated_ms[95], corrected.codes.len(), corrected.codes.first().and_then(|c| c.refined_corners_source).map(error));
    }
}
