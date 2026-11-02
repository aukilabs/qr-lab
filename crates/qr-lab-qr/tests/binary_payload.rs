use qr_lab_image::Size;
use qr_lab_qr::{
    decode_bits, scan, scan_calibrated, BitMatrix, CameraCalibration, CameraIntrinsics,
    LensDistortion, LumaView, ScanOptions, Scanner, ScannerConfig, SessionConfig,
};

const PAYLOAD: &[u8] = &[0, 255, 128, b'A', 195, b'('];

fn matrix(mirrored: bool) -> BitMatrix {
    let qr = qrcode::QrCode::new(PAYLOAD).unwrap();
    let mut bits = BitMatrix::new(qr.width());
    for y in 0..qr.width() {
        for x in 0..qr.width() {
            let pos = if mirrored { (y, x) } else { (x, y) };
            bits.set(x, y, qr[pos] == qrcode::Color::Dark);
        }
    }
    bits
}

#[test]
fn binary_payload_decodes_in_both_orientations() {
    for mirrored in [false, true] {
        let decoded = decode_bits(&matrix(mirrored)).expect("valid byte-mode QR");
        assert_eq!(decoded.payload_bytes, PAYLOAD);
        assert_eq!(decoded.payload, String::from_utf8_lossy(PAYLOAD));
        assert_eq!(decoded.mirrored, mirrored);
    }
}

#[test]
fn binary_payload_survives_scanning_refinement_and_sessions() {
    let bits = matrix(false);
    let scale = 8;
    let width = (bits.dim + 8) * scale;
    let stride = width + 7;
    let mut pixels = vec![37; stride * width];
    for y in 0..width {
        for x in 0..width {
            let (mx, my) = (x / scale, y / scale);
            let dark = mx >= 4
                && my >= 4
                && mx < bits.dim + 4
                && my < bits.dim + 4
                && bits.get(mx - 4, my - 4);
            pixels[y * stride + x] = if dark { 0 } else { 255 };
        }
    }
    let frame = LumaView::new(&pixels, width, width, stride).unwrap();
    let options = ScanOptions {
        max_working_dim: 160,
        refine: true,
    };
    let detected = scan(&frame, &options);
    assert_eq!(detected.codes.len(), 1);
    assert_eq!(detected.codes[0].payload_bytes, PAYLOAD);
    assert!(detected.codes[0].refined_corners.is_some());

    for config in [
        ScannerConfig::robust_fast().with_options(options),
        ScannerConfig::robust_fast()
            .with_options(options)
            .temporal(SessionConfig::default()),
    ] {
        let mut scanner = Scanner::new(config);
        for _ in 0..4 {
            let detected = scanner.scan(&frame);
            assert_eq!(detected.codes.len(), 1);
            assert_eq!(detected.codes[0].code.payload_bytes, PAYLOAD);
        }
        scanner.reset();
        assert_eq!(scanner.scan(&frame).codes[0].code.payload_bytes, PAYLOAD);
    }

    let camera = CameraCalibration::new(
        Size::new(width, width),
        CameraIntrinsics {
            fx: width as f64,
            fy: width as f64,
            cx: (width - 1) as f64 / 2.0,
            cy: (width - 1) as f64 / 2.0,
        },
        LensDistortion::None,
    )
    .unwrap();
    let calibrated = scan_calibrated(&frame, &options, &camera).unwrap();
    assert_eq!(calibrated.codes.len(), 1);
    assert_eq!(calibrated.codes[0].payload_bytes, PAYLOAD);
}
