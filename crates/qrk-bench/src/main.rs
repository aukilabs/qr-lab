//! Plan 6 benchmark harness: runs the scanner (any [`ScanConfig`] preset)
//! over the fixture suite and/or a directory of real PNG frames and emits
//! machine-readable JSON — per-fixture records plus per-family / per-config
//! aggregates. This binary owns everything the core deliberately does not:
//! ground-truth matching, rate aggregation, and output formatting.
//!
//! Usage:
//!   cargo run --release -p qrk-bench -- \
//!     [--fixtures DIR] [--filter PREFIX] [--config baseline|robust-fast|robust-full] \
//!     [--max-dim N] [--refine] [--real DIR_OF_PNGS] [--out FILE.json] [--quiet] \
//!     [--upscale-kernel bilinear|catmull]
//!
//! `--upscale-kernel catmull` is the E5c interpolator A/B switch: it swaps
//! the ladder's fixed-2x upscale kernel for Catmull-Rom bicubic via the
//! doc(hidden) `scan_robust_with_kernel` seam. Measurement-only — the
//! production entry point always uses bilinear.
//!
//! `--config` may repeat; default runs baseline AND robust-full so a single
//! invocation yields the headline comparison. Exit code is always 0 on a
//! completed run — thresholds/gating belong to tests, not the harness.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use qrk_core::{
    scan_robust_with_kernel, LumaView, RobustDetections, ScanConfig, ScanOptions, UpscaleKernel,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone)]
struct CodeTruth {
    payload: String,
    #[allow(dead_code)]
    version: u32,
    module_size_px: f64,
    corners_px: [[f64; 2]; 4],
    #[serde(default = "default_true")]
    expect_detect: bool,
    #[serde(default = "default_true")]
    expect_decode: bool,
    #[serde(default)]
    difficulty: u8,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct Meta {
    name: String,
    width: usize,
    height: usize,
    codes: Vec<CodeTruth>,
    #[serde(default)]
    degradations: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct CodeResult {
    payload: String,
    expect_detect: bool,
    expect_decode: bool,
    difficulty: u8,
    module_size_px: f64,
    detected: bool,
    decoded: bool,
    /// Stage/variant that decoded it (Debug of VariantKind), if any.
    variant: Option<String>,
    stage: Option<u8>,
    /// Mean refined-corner error vs ground truth in source px, when both
    /// refinement ran and the code decoded.
    corner_err_px: Option<f64>,
}

#[derive(Serialize)]
struct FixtureResult {
    name: String,
    family: String,
    config: String,
    degraded: bool,
    codes: Vec<CodeResult>,
    spurious_decodes: usize,
    variants_run: usize,
    early_exited: bool,
    budget_exhausted: bool,
    total_ms: f64,
    /// Per-variant (kind, total_ms, new_codes) in execution order.
    variants: Vec<(String, f64, usize)>,
    /// Unified (cross-variant, deduplicated) candidate counts — the
    /// detection-evidence numbers the real-video criteria track.
    unified_finders: usize,
    unified_triplets: usize,
}

#[derive(Serialize, Default, Clone)]
struct Rates {
    codes: usize,
    expected_detect: usize,
    expected_decode: usize,
    detected: usize,
    decoded: usize,
    decoded_expected: usize,
    /// Decoded despite expect_decode == false — ladder headroom wins.
    decoded_bonus: usize,
    spurious: usize,
    mean_total_ms: f64,
    p95_total_ms: f64,
    mean_variants: f64,
    early_exit_rate: f64,
}

#[derive(Serialize)]
struct Output {
    fixtures: Vec<FixtureResult>,
    /// (config, family) → aggregate rates. Family "ALL" and "ALL_DEGRADED"
    /// are cross-family rollups.
    summary: BTreeMap<String, Rates>,
    /// (config, variant kind) → codes that variant contributed first.
    variant_yield: BTreeMap<String, usize>,
}

fn family_of(name: &str) -> String {
    // <family>_<NN>[_vNN] — strip trailing numeric segments.
    let mut parts: Vec<&str> = name.split('_').collect();
    while parts.len() > 1 {
        let last = parts.last().unwrap();
        let numeric = last.chars().all(|c| c.is_ascii_digit())
            || (last.starts_with('v') && last[1..].chars().all(|c| c.is_ascii_digit()));
        if numeric {
            parts.pop();
        } else {
            break;
        }
    }
    parts.join("_")
}

fn quad_center(q: &[[f64; 2]; 4]) -> [f64; 2] {
    [
        (q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0,
        (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0,
    ]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn mean_edge(q: &[[f64; 2]; 4]) -> f64 {
    (dist(q[0], q[1]) + dist(q[1], q[2]) + dist(q[2], q[3]) + dist(q[3], q[0])) / 4.0
}

fn config_by_name(name: &str) -> ScanConfig {
    match name {
        "baseline" => ScanConfig::BASELINE,
        "robust-fast" => ScanConfig::ROBUST_FAST,
        "robust-full" => ScanConfig::ROBUST_FULL_BENCHMARK,
        other => panic!("unknown config {other:?} (baseline|robust-fast|robust-full)"),
    }
}

fn evaluate(
    name: &str,
    config_name: &str,
    truths: &[CodeTruth],
    degraded: bool,
    det: &RobustDetections,
    wall_ms: f64,
) -> FixtureResult {
    let mut matched = vec![false; det.codes.len()];
    let codes = truths
        .iter()
        .map(|t| {
            let t_center = quad_center(&t.corners_px);
            let t_edge = mean_edge(&t.corners_px).max(1.0);
            let hit = det.codes.iter().enumerate().find(|(i, c)| {
                !matched[*i]
                    && c.code.payload == t.payload
                    && dist(quad_center(&c.corners_source), t_center) < t_edge
            });
            let (variant, stage, corner_err, decoded) = match hit {
                Some((i, c)) => {
                    matched[i] = true;
                    let err = c.refined_corners_source.map(|rc| {
                        rc.iter()
                            .zip(t.corners_px.iter())
                            .map(|(a, b)| dist(*a, *b))
                            .sum::<f64>()
                            / 4.0
                    });
                    (Some(format!("{:?}", c.variant)), Some(c.stage), err, true)
                }
                None => (None, None, None, false),
            };
            // Detection: any triplet evidence (or decoded code) within the
            // truth quad's circumradius.
            let detected = decoded
                || det
                    .triplet_evidence
                    .iter()
                    .any(|p| dist(*p, t_center) < 0.75 * t_edge);
            CodeResult {
                payload: t.payload.clone(),
                expect_detect: t.expect_detect,
                expect_decode: t.expect_decode,
                difficulty: t.difficulty,
                module_size_px: t.module_size_px,
                detected,
                decoded,
                variant,
                stage,
                corner_err_px: corner_err,
            }
        })
        .collect::<Vec<_>>();
    let spurious = matched.iter().filter(|&&m| !m).count();
    FixtureResult {
        name: name.to_string(),
        family: family_of(name),
        config: config_name.to_string(),
        degraded,
        codes,
        spurious_decodes: spurious,
        variants_run: det.variants.len(),
        early_exited: det.early_exited,
        budget_exhausted: det.budget_exhausted,
        total_ms: wall_ms,
        variants: det
            .variants
            .iter()
            .map(|v| {
                (
                    format!("{:?}", v.kind),
                    v.total_ns as f64 / 1e6,
                    v.new_codes,
                )
            })
            .collect(),
        unified_finders: det.finders.len(),
        unified_triplets: det.triplets.len(),
    }
}

fn aggregate(results: &[FixtureResult]) -> (BTreeMap<String, Rates>, BTreeMap<String, usize>) {
    let mut groups: BTreeMap<String, Vec<&FixtureResult>> = BTreeMap::new();
    for r in results {
        groups
            .entry(format!("{} / {}", r.config, r.family))
            .or_default()
            .push(r);
        groups
            .entry(format!("{} / ALL", r.config))
            .or_default()
            .push(r);
        if r.degraded {
            groups
                .entry(format!("{} / ALL_DEGRADED", r.config))
                .or_default()
                .push(r);
        }
    }
    let mut summary = BTreeMap::new();
    for (key, rs) in groups {
        let mut rates = Rates::default();
        let mut times: Vec<f64> = rs.iter().map(|r| r.total_ms).collect();
        times.sort_by(f64::total_cmp);
        rates.mean_total_ms = times.iter().sum::<f64>() / times.len().max(1) as f64;
        rates.p95_total_ms = times[((times.len() as f64 * 0.95) as usize).min(times.len() - 1)];
        rates.mean_variants =
            rs.iter().map(|r| r.variants_run as f64).sum::<f64>() / rs.len().max(1) as f64;
        rates.early_exit_rate =
            rs.iter().filter(|r| r.early_exited).count() as f64 / rs.len().max(1) as f64;
        for r in rs {
            rates.spurious += r.spurious_decodes;
            for c in &r.codes {
                rates.codes += 1;
                rates.expected_detect += c.expect_detect as usize;
                rates.expected_decode += c.expect_decode as usize;
                rates.detected += c.detected as usize;
                rates.decoded += c.decoded as usize;
                if c.decoded {
                    if c.expect_decode {
                        rates.decoded_expected += 1;
                    } else {
                        rates.decoded_bonus += 1;
                    }
                }
            }
        }
        summary.insert(key, rates);
    }
    let mut variant_yield = BTreeMap::new();
    for r in results {
        for (kind, _ms, new_codes) in &r.variants {
            if *new_codes > 0 {
                *variant_yield
                    .entry(format!("{} / {}", r.config, kind))
                    .or_insert(0) += new_codes;
            }
        }
    }
    (summary, variant_yield)
}

fn load_luma_png(path: &Path) -> (Vec<u8>, usize, usize) {
    let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    // The +128 rounding matches `qrk_core::luma_from_rgba` EXACTLY — the
    // production wasm/FFI ingest path. Diagnosis D4 measured the truncating
    // variant flipping 3 real video frames from decoded to not-decoded (a
    // half-LSB darker luma), silently understating baseline recall and
    // making bench numbers non-comparable with production.
    let luma = match info.color_type {
        png::ColorType::Grayscale => buf[..w * h].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3]
            .chunks_exact(3)
            .map(|p| ((77 * p[0] as u32 + 150 * p[1] as u32 + 29 * p[2] as u32 + 128) >> 8) as u8)
            .collect(),
        png::ColorType::Rgba => buf[..w * h * 4]
            .chunks_exact(4)
            .map(|p| ((77 * p[0] as u32 + 150 * p[1] as u32 + 29 * p[2] as u32 + 128) >> 8) as u8)
            .collect(),
        other => panic!("{path:?}: unsupported color type {other:?}"),
    };
    (luma, w, h)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut filter: Option<String> = None;
    let mut configs: Vec<String> = Vec::new();
    let mut max_dim: u32 = 0;
    let mut refine = false;
    let mut out_path: Option<PathBuf> = None;
    let mut real_dir: Option<PathBuf> = None;
    let mut quiet = false;
    let mut skip_fixtures = false;
    let mut kernel = UpscaleKernel::Bilinear;
    // Diagnostic (--dump-evidence): print every fixture's triplet-evidence
    // centroids, per-variant detection counts, and decoded-code source
    // corners to stderr.
    let mut dump_evidence = false;
    // --session[=PERIOD]: scan the --real frames (in sorted/frame order)
    // through ONE ScanSession per config instead of independent per-frame
    // scan_robust calls — measures the temporal amortization (rung
    // rotation + cross-frame pooling). Robust configs only; ignored for
    // the fixture suite (still-image inputs).
    let mut session_period: Option<u32> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            s if s == "--session" || s.starts_with("--session=") => {
                session_period = Some(
                    s.strip_prefix("--session=")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(3),
                );
            }
            "--fixtures" => {
                i += 1;
                fixtures_dir = PathBuf::from(&args[i]);
            }
            "--no-fixtures" => skip_fixtures = true,
            "--filter" => {
                i += 1;
                filter = Some(args[i].clone());
            }
            "--config" => {
                i += 1;
                configs.push(args[i].clone());
            }
            "--max-dim" => {
                i += 1;
                max_dim = args[i].parse().unwrap();
            }
            "--refine" => refine = true,
            "--out" => {
                i += 1;
                out_path = Some(PathBuf::from(&args[i]));
            }
            "--real" => {
                i += 1;
                real_dir = Some(PathBuf::from(&args[i]));
            }
            "--quiet" => quiet = true,
            "--dump-evidence" => dump_evidence = true,
            "--upscale-kernel" => {
                i += 1;
                kernel = match args[i].as_str() {
                    "bilinear" => UpscaleKernel::Bilinear,
                    "catmull" => UpscaleKernel::CatmullRom,
                    other => panic!("unknown upscale kernel {other:?} (bilinear|catmull)"),
                };
            }
            other => panic!("unknown arg {other:?}"),
        }
        i += 1;
    }
    if configs.is_empty() {
        configs = vec!["baseline".into(), "robust-full".into()];
    }
    let opts = ScanOptions {
        max_working_dim: max_dim,
        refine,
    };

    let mut results: Vec<FixtureResult> = Vec::new();

    if !skip_fixtures {
        let mut names: Vec<String> = fs::read_dir(&fixtures_dir)
            .expect("fixtures dir")
            .filter_map(|e| {
                let p = e.unwrap().path();
                (p.extension()? == "json")
                    .then(|| p.file_stem().unwrap().to_str().unwrap().to_string())
            })
            .filter(|n| filter.as_ref().is_none_or(|f| n.starts_with(f.as_str())))
            .collect();
        names.sort();

        for name in &names {
            let meta: Meta = serde_json::from_str(
                &fs::read_to_string(fixtures_dir.join(format!("{name}.json"))).unwrap(),
            )
            .unwrap_or_else(|e| panic!("{name}.json: {e}"));
            let luma = fs::read(fixtures_dir.join(format!("{name}.luma")))
                .unwrap_or_else(|e| panic!("{name}.luma: {e}"));
            assert_eq!(luma.len(), meta.width * meta.height, "{name}: luma size");
            let view = LumaView::new(&luma, meta.width, meta.height, meta.width).unwrap();
            for cfg_name in &configs {
                let cfg = config_by_name(cfg_name);
                let t0 = Instant::now();
                let det = scan_robust_with_kernel(&view, &opts, &cfg, kernel);
                let wall_ms = t0.elapsed().as_secs_f64() * 1e3;
                results.push(evaluate(
                    &meta.name,
                    cfg_name,
                    &meta.codes,
                    meta.degradations.is_some(),
                    &det,
                    wall_ms,
                ));
            }
        }
    }

    // Real frames: no ground truth — decode counts and timings only,
    // reported as family "real" with a single synthetic truth per decoded
    // payload (detected == decoded).
    if let Some(dir) = real_dir {
        let mut pngs: Vec<PathBuf> = fs::read_dir(&dir)
            .expect("real dir")
            .filter_map(|e| {
                let p = e.unwrap().path();
                (p.extension()? == "png").then_some(p)
            })
            .collect();
        pngs.sort();
        // Session mode iterates config-outer / frame-inner so each config's
        // ScanSession persists across the sorted (frame-ordered) sequence;
        // the default per-frame mode keeps frame-outer (order-independent).
        let frame_pairs: Vec<(String, Vec<u8>, usize, usize)> = pngs
            .iter()
            .map(|p| {
                let (luma, w, h) = load_luma_png(p);
                (
                    format!("real/{}", p.file_stem().unwrap().to_str().unwrap()),
                    luma,
                    w,
                    h,
                )
            })
            .collect();
        for cfg_name in &configs {
            let cfg = config_by_name(cfg_name);
            let mut session = session_period.map(|p| {
                qrk_core::ScanSession::new(
                    cfg,
                    qrk_core::SessionConfig {
                        rotation_period: p,
                        ..Default::default()
                    },
                )
            });
            for (name, luma, w, h) in &frame_pairs {
                let view = LumaView::new(luma, *w, *h, *w).unwrap();
                let t0 = Instant::now();
                let det = match session.as_mut() {
                    Some(s) => s.scan_frame(&view, &opts),
                    None => scan_robust_with_kernel(&view, &opts, &cfg, kernel),
                };
                let wall_ms = t0.elapsed().as_secs_f64() * 1e3;
                if dump_evidence {
                    eprintln!("{name} [{cfg_name}] evidence: {:?}", det.triplet_evidence);
                    for v in &det.variants {
                        eprintln!(
                            "  variant {:?}: finders={} triplets={} codes={} new={}",
                            v.kind, v.finders, v.triplets, v.codes, v.new_codes
                        );
                    }
                    for c in &det.codes {
                        eprintln!(
                            "  code {:?} via {:?} corners {:?}",
                            c.code.payload, c.variant, c.corners_source
                        );
                    }
                }
                let truths: Vec<CodeTruth> = det
                    .codes
                    .iter()
                    .map(|c| CodeTruth {
                        payload: c.code.payload.clone(),
                        version: c.code.version,
                        module_size_px: mean_edge(&c.corners_source)
                            / c.code.dimension.max(1) as f64,
                        corners_px: c.corners_source,
                        expect_detect: true,
                        expect_decode: true,
                        difficulty: 0,
                    })
                    .collect();
                results.push(evaluate(name, cfg_name, &truths, false, &det, wall_ms));
            }
        }
    }

    let (summary, variant_yield) = aggregate(&results);
    if !quiet {
        eprintln!(
            "{:<38} {:>5} {:>7} {:>7} {:>7} {:>6} {:>8} {:>8} {:>6}",
            "config / family",
            "codes",
            "det",
            "dec",
            "dec-exp",
            "bonus",
            "mean-ms",
            "p95-ms",
            "exit%"
        );
        for (key, r) in &summary {
            eprintln!(
                "{:<38} {:>5} {:>7} {:>7} {:>5}/{:<3} {:>4} {:>8.2} {:>8.2} {:>5.0}%",
                key,
                r.codes,
                r.detected,
                r.decoded,
                r.decoded_expected,
                r.expected_decode,
                r.decoded_bonus,
                r.mean_total_ms,
                r.p95_total_ms,
                r.early_exit_rate * 100.0
            );
        }
        eprintln!("\nvariant first-decode yield:");
        for (k, n) in &variant_yield {
            eprintln!("  {k}: {n}");
        }
    }
    let output = Output {
        fixtures: results,
        summary,
        variant_yield,
    };
    let json = serde_json::to_string_pretty(&output).unwrap();
    match out_path {
        Some(p) => {
            fs::write(&p, &json).unwrap();
            eprintln!("wrote {}", p.display());
        }
        None => println!("{json}"),
    }
}
