//! Profile classic scanning by stage and retain deterministic outputs for A/B checks.
//! Usage: cargo run --release -p qr-lab-bench --bin core_profile -- 11 output.json [fixture ...]
//! Fixtures are loaded before timing; two warmups precede repeated measurements.

use std::{fs, hint::black_box, path::PathBuf, time::Instant};

use qr_lab::{scan, Detections, LumaView, ScanOptions, StageTimings};
use serde::Serialize;

#[derive(Serialize)]
struct Record {
    name: String,
    median_us: [f64; 8],
    detections: Detections,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let repeats: usize = args.next().unwrap_or_else(|| "11".into()).parse()?;
    assert!(repeats > 0, "repeat count must be positive");
    let output = args.next().unwrap_or_else(|| "core-profile.json".into());
    let names: Vec<String> = args.collect();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut paths: Vec<_> = fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| {
            names.is_empty()
                || names
                    .iter()
                    .any(|name| path.file_stem().unwrap() == name.as_str())
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures selected");
    let options = ScanOptions {
        max_working_dim: 1280,
        refine: true,
    };
    let mut records = Vec::new();
    for path in paths {
        let meta: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        let width = meta["width"].as_u64().ok_or("missing width")? as usize;
        let height = meta["height"].as_u64().ok_or("missing height")? as usize;
        let pixels = fs::read(path.with_extension("luma"))?;
        let frame = LumaView::new(&pixels, width, height, width)?;
        for _ in 0..2 {
            black_box(scan(&frame, &options));
        }
        let mut samples: [Vec<u64>; 8] = std::array::from_fn(|_| Vec::with_capacity(repeats));
        let mut detections = None;
        for _ in 0..repeats {
            let start = Instant::now();
            let result = scan(black_box(&frame), black_box(&options));
            let wall = start.elapsed().as_nanos() as u64;
            let t = result.timings;
            for (values, value) in samples.iter_mut().zip([
                wall,
                t.tiles_ns,
                t.finders_ns,
                t.triplets_ns,
                t.version_ns,
                t.alignment_ns,
                t.sample_decode_ns,
                t.refine_ns,
            ]) {
                values.push(value);
            }
            detections = Some(result);
        }
        let median_us = samples.map(|mut values| {
            values.sort_unstable();
            values[repeats / 2] as f64 / 1_000.0
        });
        let mut detections = detections.unwrap();
        detections.timings = StageTimings::default();
        records.push(Record {
            name: path.file_stem().unwrap().to_string_lossy().into_owned(),
            median_us,
            detections,
        });
    }
    fs::write(
        &output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "repeats": repeats, "warmups": 2, "max_working_dim": 1280, "refine": true,
            "stage_order": ["wall", "tiles", "finders", "triplets", "version", "alignment", "sample_decode", "refine"],
            "fixtures": records,
        }))?,
    )?;
    eprintln!("wrote {output}");
    Ok(())
}
