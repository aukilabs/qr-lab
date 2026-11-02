use qr_lab_qr::{scan, LumaView, ScanOptions};

const BINARY_PAYLOAD: &[u8] = &[0, 255, 128, 65, 195, 40];

fn assert_corners(actual: [[f64; 2]; 4], expected: [[f64; 2]; 4]) {
    let errors = std::array::from_fn::<_, 4, _>(|i| {
        (actual[i][0] - expected[i][0]).hypot(actual[i][1] - expected[i][1])
    });
    assert!(
        errors.iter().all(|error| *error < 2.0),
        "corner errors: {errors:?}"
    );
}

#[test]
fn captured_large_finder_camera_frame_has_accurate_corners() {
    // Synthetic binary QR injected through serve-sim, captured at the native
    // scanLuma boundary. Keep the camera's resampling and letterboxing intact.
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "data/large-finder-camera.png"
    )));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    assert_eq!((info.width, info.height), (1280, 720));
    let view = LumaView::new(&pixels, 1280, 720, 1280).unwrap();
    let detected = scan(
        &view,
        &ScanOptions {
            max_working_dim: 1080,
            refine: true,
        },
    );
    assert_eq!(detected.codes.len(), 1);
    let code = &detected.codes[0];
    assert_eq!(code.payload_bytes, BINARY_PAYLOAD);
    assert_eq!(code.corner_refined, [true; 4]);
    // Half-pixel boundaries from the injected axis-aligned module region.
    assert_corners(
        code.refined_corners.unwrap(),
        [[378.5, 98.5], [900.5, 98.5], [900.5, 620.5], [378.5, 620.5]],
    );
}

#[test]
fn large_finders_survive_flat_tile_interiors_in_both_polarities() {
    for payload in [BINARY_PAYLOAD, &b"large finder regression"[..]] {
        let qr = qrcode::QrCode::new(payload).unwrap();
        // Finder cores straddle the 48-pixel threshold neighborhood size.
        for scale in [16, 20, 24, 32] {
            for inverted in [false, true] {
                // Exercise every alignment to the 16-pixel tile grid on both axes.
                for offset in 0..16 {
                    let side = (qr.width() + 8) * scale + 16;
                    let ox = 4 * scale + offset;
                    let oy = 4 * scale + (offset * 7) % 16;
                    let mut pixels = vec![if inverted { 0 } else { 255 }; side * side];
                    for y in 0..qr.width() * scale {
                        for x in 0..qr.width() * scale {
                            let dark =
                                (qr[(x / scale, y / scale)] == qrcode::Color::Dark) != inverted;
                            pixels[(y + oy) * side + x + ox] = if dark { 0 } else { 255 };
                        }
                    }
                    let view = LumaView::new(&pixels, side, side, side).unwrap();
                    let detected = scan(
                        &view,
                        &ScanOptions {
                            max_working_dim: side as u32,
                            refine: true,
                        },
                    );
                    let context = format!(
                        "version {:?}, scale {scale}, inverted {inverted}, offset {offset}",
                        qr.version()
                    );
                    assert_eq!(detected.codes.len(), 1, "{context}");
                    assert_eq!(detected.codes[0].payload_bytes, payload, "{context}");
                    let lo = [ox as f64 - 0.5, oy as f64 - 0.5];
                    let hi = [
                        lo[0] + (qr.width() * scale) as f64,
                        lo[1] + (qr.width() * scale) as f64,
                    ];
                    let inset = 3.5 * scale as f64;
                    for expected in [
                        [lo[0] + inset, lo[1] + inset],
                        [hi[0] - inset, lo[1] + inset],
                        [lo[0] + inset, hi[1] - inset],
                    ] {
                        assert!(
                            detected.finders.iter().any(|f| {
                                f.inverted == inverted
                                    && (f.x - expected[0]).hypot(f.y - expected[1]) < 1.0
                            }),
                            "finder center {expected:?}: {context}"
                        );
                    }
                    // V1 also exercises the corner symptom without introducing
                    // the separate alignment-pattern path used by higher versions.
                    if qr.width() == 21 {
                        assert_corners(
                            detected.codes[0].refined_corners.unwrap(),
                            [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
                        );
                    }
                }
            }
        }
    }
}
