//! Decode QR payloads from a PNG photo at a chosen working resolution.
//! Usage: cargo run --release -p qrk-core --example decode_photo -- <path.png> [max_dim=1280]
//! (png is a dev-dependency, so run via `--example`.)
use std::fs::File;
use std::io::BufReader;

fn main() {
    let path = std::env::args().nth(1).expect("png path");
    let max_dim: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(1280);

    let decoder = png::Decoder::new(BufReader::new(File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    let luma: Vec<u8> = match info.color_type {
        png::ColorType::Grayscale => buf[..w * h].to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(w * h * 4);
            for p in buf[..w * h * 3].chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            qrk_core::luma_from_rgba(&rgba, w, h)
        }
        png::ColorType::Rgba => qrk_core::luma_from_rgba(&buf[..w * h * 4], w, h),
        other => panic!("unsupported png color type {other:?}"),
    };

    // Production nearest-neighbor downscale (same rounding as the JNI/debug-ui path).
    let (dw, dh, data) = if max_dim > 0 && w.max(h) > max_dim {
        let s = max_dim as f64 / w.max(h) as f64;
        let (dw, dh) = ((w as f64 * s).round() as usize, (h as f64 * s).round() as usize);
        let mut out = vec![0u8; dw * dh];
        for y in 0..dh {
            let sy = ((y as f64 / s) as usize).min(h - 1);
            for x in 0..dw {
                let sx = ((x as f64 / s) as usize).min(w - 1);
                out[y * dw + x] = luma[sy * w + sx];
            }
        }
        (dw, dh, out)
    } else {
        (w, h, luma)
    };

    let view = qrk_core::LumaView::new(&data, dw, dh, dw).unwrap();
    let det = qrk_core::detect(&view);
    println!(
        "{path} @{dw}x{dh}: {} finders, {} triplets, {} decoded",
        det.finders.len(),
        det.triplets.len(),
        det.codes.len()
    );
    for c in &det.codes {
        println!(
            "  v{} ecc={} mirrored={} inverted={} dim={} payload={:?}",
            c.version, c.ecc, c.mirrored, c.inverted, c.dimension, c.payload
        );
        println!(
            "    corners TL({:.1},{:.1}) TR({:.1},{:.1}) BR({:.1},{:.1}) BL({:.1},{:.1})",
            c.corners[0][0], c.corners[0][1], c.corners[1][0], c.corners[1][1],
            c.corners[2][0], c.corners[2][1], c.corners[3][0], c.corners[3][1]
        );
    }
    println!(
        "timings us: tiles={} finders={} triplets={} version={} alignment={} sample_decode={}",
        det.timings.tiles_ns / 1000,
        det.timings.finders_ns / 1000,
        det.timings.triplets_ns / 1000,
        det.timings.version_ns / 1000,
        det.timings.alignment_ns / 1000,
        det.timings.sample_decode_ns / 1000
    );
}
