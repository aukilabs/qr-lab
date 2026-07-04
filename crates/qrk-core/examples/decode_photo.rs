//! Decode QR payloads from a PNG photo at a chosen working resolution.
//! Usage: cargo run --release -p qrk-core --example decode_photo -- <path.png> [max_dim=1280]
//! (png is a dev-dependency, so run via `--example`.)
//!
//! Plan 5 Task 6: switched from a manual pre-downscale + `detect()` to
//! [`qrk_core::scan`] on the FULL-resolution source view. The old manual
//! downscale (still visible in git history) built only a working-resolution
//! buffer and handed `detect()` that alone — it discarded the source-
//! resolution pixels before `qrk_core` ever saw them, which silently
//! defeated Plan 5 Tasks 1-3 entirely (Rust-owned downscale, source-
//! resolution module sampling, and subpixel refinement all need the
//! ORIGINAL source view alongside the working one). Concretely:
//! `IMG_4832.png @1280` — the gate 3 fixture this whole feature was built
//! for — decoded 0 codes under the old manual-downscale path (verified
//! before this change) and decodes 1 (`HTTPS://R8.HR/OU3QBPE14BY`) under
//! `scan`, since only `scan` samples that far/small code's modules from the
//! full-resolution source instead of the pre-downscaled working copy.
//!
//! `refine` is always on (not a flag): this is a diagnostic tool, not a
//! perf-sensitive hot path, and the whole point of running it post-Plan-5
//! is to see the refined-corner + source-sampling numbers together with
//! the rest of the pipeline's timings — an off-by-default flag here would
//! just be one more thing to remember to pass.
use std::fs::File;
use std::io::BufReader;

use qrk_core::{scan, ScanOptions};

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

    // `scan` (Plan 5 Task 1) owns the downscale itself now, against the
    // FULL-resolution `view` below — see this file's module doc for why
    // that matters (source-resolution sampling + refinement both need the
    // original pixels, not a pre-downscaled copy).
    let view = qrk_core::LumaView::new(&luma, w, h, w).unwrap();
    let opts = ScanOptions { max_working_dim: max_dim as u32, refine: true };
    let det = scan(&view, &opts);
    println!(
        "{path} @source {w}x{h}, working max_dim={max_dim} (source_scale={:.4}): \
         {} finders, {} triplets, {} decoded",
        det.source_scale,
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
            "    corners (working px) TL({:.1},{:.1}) TR({:.1},{:.1}) BR({:.1},{:.1}) BL({:.1},{:.1})",
            c.corners[0][0], c.corners[0][1], c.corners[1][0], c.corners[1][1],
            c.corners[2][0], c.corners[2][1], c.corners[3][0], c.corners[3][1]
        );
        if let Some(rc) = c.refined_corners {
            println!(
                "    refined (source px) TL({:.2},{:.2}) TR({:.2},{:.2}) BR({:.2},{:.2}) BL({:.2},{:.2})",
                rc[0][0], rc[0][1], rc[1][0], rc[1][1], rc[2][0], rc[2][1], rc[3][0], rc[3][1]
            );
        }
    }
    println!(
        "timings us: tiles={} finders={} triplets={} version={} alignment={} sample_decode={} refine={}",
        det.timings.tiles_ns / 1000,
        det.timings.finders_ns / 1000,
        det.timings.triplets_ns / 1000,
        det.timings.version_ns / 1000,
        det.timings.alignment_ns / 1000,
        det.timings.sample_decode_ns / 1000,
        det.timings.refine_ns / 1000,
    );
}
