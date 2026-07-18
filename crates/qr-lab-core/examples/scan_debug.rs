//! Throwaway diagnostic: print all finder candidates for a fixture.
//! Usage: cargo run --release -p qr-lab-core --example scan_debug -- <name> [scale_max_dim]
use std::fs;

fn main() {
    let name = std::env::args().nth(1).expect("fixture name");
    let max_dim: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(0);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(format!("{dir}/{name}.json")).unwrap()).unwrap();
    let (w, h) = (
        meta["width"].as_u64().unwrap() as usize,
        meta["height"].as_u64().unwrap() as usize,
    );
    let luma = fs::read(format!("{dir}/{name}.luma")).unwrap();

    // Optional nearest-neighbor downscale to max_dim (production-style).
    let (dw, dh, data) = if max_dim > 0 && w.max(h) > max_dim {
        let s = max_dim as f64 / w.max(h) as f64;
        let (dw, dh) = (
            (w as f64 * s).round() as usize,
            (h as f64 * s).round() as usize,
        );
        let mut out = vec![0u8; dw * dh];
        for y in 0..dh {
            for x in 0..dw {
                let sx = (x as f64 / s) as usize;
                let sy = (y as f64 / s) as usize;
                out[y * dw + x] = luma[sy.min(h - 1) * w + sx.min(w - 1)];
            }
        }
        (dw, dh, out)
    } else {
        (w, h, luma)
    };

    let view = qr_lab_core::LumaView::new(&data, dw, dh, dw).unwrap();
    let grid = qr_lab_core::TileGrid::build(&view);
    let finders = qr_lab_core::find_finders(&view, &grid);
    println!("{name} @{dw}x{dh}: {} finder candidates", finders.len());
    for f in &finders {
        println!(
            "  ({:.1},{:.1}) module={:.2} inverted={} hits={}",
            f.x, f.y, f.module, f.inverted, f.hits
        );
    }
    let trips = qr_lab_core::group_triplets(&view, &grid, &finders);
    println!("{} triplets", trips.len());
    for t in &trips {
        println!(
            "  tl=({:.0},{:.0}) tr=({:.0},{:.0}) bl=({:.0},{:.0}) dim={} snap_err={:.2} inv={}",
            t.tl[0],
            t.tl[1],
            t.tr[0],
            t.tr[1],
            t.bl[0],
            t.bl[1],
            t.dimension,
            t.snap_error,
            t.inverted
        );
    }
}
