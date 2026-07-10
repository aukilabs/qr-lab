//! Scratch probe (experiment E4): sweep synthetic horizontal box smears
//! over a golden fixture and report where baseline dies and where the
//! Van Cittert rung recovers. Not part of any gate.
//!
//! Usage: cargo run --release -p qrk-core --example vc_probe -- near_00

use qrk_core::{scan, scan_robust, LumaView, ScanConfig, ScanOptions};

fn hbox_blur(d: &[u8], w: usize, h: usize, len: usize) -> Vec<u8> {
    let half = (len / 2) as isize;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0u32;
            for k in -half..=half {
                let sx = (x as isize + k).clamp(0, w as isize - 1) as usize;
                sum += d[y * w + sx] as u32;
            }
            out[y * w + x] = (sum / len as u32) as u8;
        }
    }
    out
}

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "near_00".into());
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap())
            .unwrap();
    let (w, h) = (
        meta["width"].as_u64().unwrap() as usize,
        meta["height"].as_u64().unwrap() as usize,
    );
    let m = meta["codes"][0]["module_size_px"].as_f64().unwrap();
    let luma = std::fs::read(dir.join(format!("{name}.luma"))).unwrap();
    let opts = ScanOptions {
        max_working_dim: 0,
        refine: false,
    };
    println!("{name}: module {m:.2}px");
    for len in [7, 9, 11, 13, 15, 17, 19, 21, 25] {
        let blurred = hbox_blur(&luma, w, h, len);
        let view = LumaView::new(&blurred, w, h, w).unwrap();
        let base = scan(&view, &opts);
        let full = scan_robust(&view, &opts, &ScanConfig::ROBUST_FULL_BENCHMARK);
        let via: Vec<String> = full
            .codes
            .iter()
            .map(|c| format!("{:?}", c.variant))
            .collect();
        println!(
            "L={len:2} ({:.2} modules): baseline {} robust-full {} via {:?}",
            len as f64 / m,
            base.codes.len(),
            full.codes.len(),
            via
        );
    }
}
