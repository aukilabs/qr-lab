//! Per-variant cost breakdown on domain gold-obs frames.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;

use qrkit::{luma_from_rgba, scan_robust, LumaView, ScanConfig, ScanOptions};

fn load_png(path: &Path) -> Option<(Vec<u8>, usize, usize)> {
    let file = File::open(path).ok()?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    let luma = match info.color_type {
        png::ColorType::Grayscale => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(w * h * 4);
            for px in bytes.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            luma_from_rgba(&rgba, w, h)
        }
        png::ColorType::Rgba => luma_from_rgba(bytes, w, h),
        _ => return None,
    };
    Some((luma, w, h))
}

fn obs_frames(scan_dir: &Path) -> Vec<usize> {
    let mut frames = Vec::new();
    let mut ts_to_idx = BTreeMap::new();
    for line in BufReader::new(File::open(scan_dir.join("Frames.csv")).unwrap()).lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let mut p = line.split(',');
        let ts: f64 = p.next().unwrap().parse().unwrap();
        let idx: usize = p
            .next()
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        ts_to_idx.insert((ts * 1e6).round() as i64, idx);
        frames.push((ts, idx));
    }
    let mut set = std::collections::BTreeSet::new();
    for line in BufReader::new(File::open(scan_dir.join("Observations.csv")).unwrap()).lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let ts: f64 = line.split(',').next().unwrap().parse().unwrap();
        let key = (ts * 1e6).round() as i64;
        let idx = ts_to_idx.get(&key).copied().unwrap_or_else(|| {
            frames
                .iter()
                .min_by(|a, b| (a.0 - ts).abs().total_cmp(&(b.0 - ts).abs()))
                .map(|x| x.1)
                .unwrap()
        });
        set.insert(idx);
    }
    set.into_iter().collect()
}

fn kind_key(s: &str) -> String {
    s.split('{').next().unwrap_or(s).trim().to_string()
}

fn main() {
    let domain =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real/full-domain-data");
    let frames_root = PathBuf::from("/tmp/domain_gold");
    let max_dim: u32 = 1280;
    let opts = ScanOptions {
        max_working_dim: max_dim,
        refine: false,
    };
    let cfg = ScanConfig::ROBUST_FAST;

    let mut cost: BTreeMap<String, (u64, f64, usize)> = BTreeMap::new(); // n, total_ms, new_codes
    let mut frame_ms = Vec::new();
    let mut buckets = [0usize; 5]; // <10, 10-20, 20-33, 33-50, >=50

    let mut scans: Vec<_> = std::fs::read_dir(&domain)
        .unwrap()
        .filter_map(|e| {
            let p = e.ok()?.path();
            p.is_dir().then_some(p)
        })
        .collect();
    scans.sort();

    for scan in &scans {
        let name = scan.file_name().unwrap().to_string_lossy().into_owned();
        for idx in obs_frames(scan) {
            let path = frames_root
                .join(&name)
                .join("all_frames")
                .join(format!("f{:05}.png", idx + 1));
            let Some((luma, w, h)) = load_png(&path) else {
                continue;
            };
            let view = LumaView::new(&luma, w, h, w).unwrap();
            let t0 = Instant::now();
            let det = scan_robust(&view, &opts, &cfg);
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            frame_ms.push(ms);
            if ms < 10.0 {
                buckets[0] += 1;
            } else if ms < 20.0 {
                buckets[1] += 1;
            } else if ms < 33.0 {
                buckets[2] += 1;
            } else if ms < 50.0 {
                buckets[3] += 1;
            } else {
                buckets[4] += 1;
            }
            for v in &det.variants {
                let k = kind_key(&format!("{:?}", v.kind));
                let e = cost.entry(k).or_insert((0, 0.0, 0));
                e.0 += 1;
                e.1 += v.total_ns as f64 / 1e6;
                e.2 += v.new_codes;
            }
        }
    }
    frame_ms.sort_by(|a, b| a.total_cmp(b));
    let n = frame_ms.len();
    let mean = frame_ms.iter().sum::<f64>() / n as f64;
    let p50 = frame_ms[(n as f64 * 0.5) as usize];
    let p95 = frame_ms[((n as f64 * 0.95) as usize).min(n - 1)];
    println!(
        "frames={n} mean={mean:.1} p50={p50:.1} p95={p95:.1} max={:.1}",
        frame_ms[n - 1]
    );
    println!(
        "buckets <10={} 10-20={} 20-33={} 33-50={} >=50={}",
        buckets[0], buckets[1], buckets[2], buckets[3], buckets[4]
    );
    println!("\nvariant cost (sorted by total_ms):");
    let mut rows: Vec<_> = cost.into_iter().collect();
    rows.sort_by(|a, b| b.1 .1.total_cmp(&a.1 .1));
    println!(
        "{:<28} {:>6} {:>10} {:>8} {:>8}",
        "kind", "n", "total_ms", "mean", "new"
    );
    for (k, (n, tot, nc)) in rows {
        println!("{k:<28} {n:>6} {tot:>10.0} {:>8.2} {nc:>8}", tot / n as f64);
    }
}
