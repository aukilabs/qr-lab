//! Gold-standard evaluation against GPU-scanner domain recordings.
//!
//! Each `fixtures/real/full-domain-data/dmt_scan_*` folder holds:
//! - `recording.mp4` — camera stream (typically 1920×1440 @ 10 fps)
//! - `Frames.csv` — `timestamp,recording.mp4:frame_index` per frame
//! - `Observations.csv` — GPU scanner hits: timestamp, shortId, pose, 4 image corners
//! - `Manifest.json` — portal shortIds + physical sizes
//!
//! The GPU observations are the acceptance bar: on every frame (and every
//! shortId) the GPU decoded, the CPU scanner should decode the same
//! shortId (payload contains it, case-insensitive) — ideally more, never
//! fewer. Timing is reported so the 30 fps mobile budget is visible.
//!
//! Usage:
//!   # Pre-extract frames once (ffmpeg 1-indexed f00001.png = frame 0):
//!   #   ffmpeg -i recording.mp4 -vsync 0 -q:v 3 /tmp/domain_gold/<scan>/all_frames/f%05d.png
//!   cargo run --release -p qrk-bench --bin domain_eval -- \
//!     [--domain DIR] [--frames-root DIR] [--config baseline|robust-fast|robust-full] \
//!     [--max-dim N] [--session[=PERIOD]] [--obs-only] [--out FILE.json] [--quiet]
//!
//! `--obs-only` only scans frames the GPU observed (fast recall check).
//! Without it, every extracted frame is scanned (recall + extras).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;

use qrk_core::{
    luma_from_rgba, scan_robust, LumaView, ScanConfig, ScanOptions, ScanSession, SessionConfig,
};
use serde::Serialize;

#[derive(Clone, Debug)]
struct GoldObs {
    short_id: String,
    /// Four image-plane corners from the GPU scanner (source px).
    corners: [[f64; 2]; 4],
}

#[derive(Clone, Debug)]
struct ScanGold {
    name: String,
    n_frames: usize,
    /// frame_index → GPU observations on that frame
    by_frame: BTreeMap<usize, Vec<GoldObs>>,
}

#[derive(Serialize)]
struct FrameHit {
    frame: usize,
    gpu_ids: Vec<String>,
    cpu_payloads: Vec<String>,
    /// GPU shortIds recovered by CPU on this frame.
    matched: Vec<String>,
    /// GPU shortIds the CPU missed on this frame.
    missed: Vec<String>,
    /// CPU payloads with no matching GPU observation on this frame.
    extras: Vec<String>,
    total_ms: f64,
    unified_finders: usize,
    unified_triplets: usize,
}

#[derive(Serialize, Default)]
struct ScanReport {
    scan: String,
    config: String,
    n_frames_scanned: usize,
    n_gpu_obs: usize,
    n_gpu_obs_frames: usize,
    /// Observation-level recall: matched GPU (frame, shortId) / all GPU obs.
    obs_matched: usize,
    obs_missed: usize,
    obs_recall: f64,
    /// Frame-level: frames with ≥1 GPU obs where CPU decoded ≥1 matching shortId.
    frame_matched: usize,
    frame_missed: usize,
    frame_recall: f64,
    /// Frames (in the scanned set) where CPU decoded something GPU did not.
    extra_decode_frames: usize,
    extra_decode_count: usize,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    /// Distinct shortIds GPU saw vs CPU recovered at least once.
    gpu_unique_ids: usize,
    cpu_unique_ids_matched: usize,
    missed_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sample_misses: Vec<String>,
    /// Matched observations with both GPU and CPU corners: mean corner
    /// error (px) after best cyclic/reflect alignment of the quads.
    corner_n: usize,
    corner_mean_err_px: f64,
    corner_p95_err_px: f64,
    /// Refined-corner subset (when `ScanOptions::refine` is on).
    refined_corner_n: usize,
    refined_corner_mean_err_px: f64,
}

#[derive(Serialize)]
struct Report {
    configs: Vec<ScanReport>,
    totals: Vec<ScanReport>,
}

fn load_png_luma(path: &Path) -> Option<(Vec<u8>, usize, usize)> {
    let file = File::open(path).ok()?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    let luma = match info.color_type {
        png::ColorType::Grayscale => bytes.to_vec(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).map(|p| p[0]).collect(),
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

fn parse_gold(scan_dir: &Path) -> Option<ScanGold> {
    let frames_path = scan_dir.join("Frames.csv");
    let obs_path = scan_dir.join("Observations.csv");
    if !frames_path.exists() || !obs_path.exists() {
        return None;
    }
    let mut frames: Vec<(f64, usize)> = Vec::new();
    for line in BufReader::new(File::open(&frames_path).ok()?).lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split(',');
        let ts: f64 = parts.next()?.parse().ok()?;
        let ref_s = parts.next()?;
        let idx: usize = ref_s.rsplit(':').next()?.parse().ok()?;
        frames.push((ts, idx));
    }
    if frames.is_empty() {
        return None;
    }
    let ts_to_idx: HashMap<i64, usize> = frames
        .iter()
        // microsecond bucket for exact float match noise
        .map(|(ts, idx)| ((ts * 1e6).round() as i64, *idx))
        .collect();
    let mut by_frame: BTreeMap<usize, Vec<GoldObs>> = BTreeMap::new();
    for line in BufReader::new(File::open(&obs_path).ok()?).lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 17 {
            continue;
        }
        let ts: f64 = cols[0].parse().ok()?;
        let short_id = cols[1].to_string();
        let key = (ts * 1e6).round() as i64;
        let idx = if let Some(&i) = ts_to_idx.get(&key) {
            i
        } else {
            // nearest timestamp
            frames
                .iter()
                .min_by(|a, b| (a.0 - ts).abs().total_cmp(&(b.0 - ts).abs()))
                .map(|(_, i)| *i)?
        };
        let mut corners = [[0.0; 2]; 4];
        for k in 0..4 {
            corners[k][0] = cols[9 + 2 * k].parse().ok()?;
            corners[k][1] = cols[10 + 2 * k].parse().ok()?;
        }
        by_frame
            .entry(idx)
            .or_default()
            .push(GoldObs { short_id, corners });
    }
    Some(ScanGold {
        name: scan_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        n_frames: frames.len(),
        by_frame,
    })
}

fn config_by_name(name: &str) -> ScanConfig {
    match name {
        "baseline" => ScanConfig::BASELINE,
        "robust-fast" => ScanConfig::ROBUST_FAST,
        "robust-full" => ScanConfig::ROBUST_FULL_BENCHMARK,
        other => panic!("unknown config {other}"),
    }
}

fn payload_matches_short_id(payload: &str, short_id: &str) -> bool {
    // GPU shortIds are the portal id suffix; payloads are typically
    // HTTPS://R8.HR/<shortId> (case varies).
    payload.to_ascii_uppercase().contains(&short_id.to_ascii_uppercase())
}

/// Mean corner-to-corner distance after choosing the best cyclic rotation
/// and optional reverse of `cpu` to align with `gpu` (both 4-corner quads
/// in source px). Corner order from either scanner is not guaranteed to
/// share a start index or winding.
///
/// When `frame_w`/`frame_h` are known, also tries a 180° image-space
/// remap of the CPU corners (`(W−x, H−y)`): the DMT `recording.mp4`
/// frames as extracted by ffmpeg are 180° from the coordinate system the
/// GPU scanner logged in `Observations.csv` (measured: payload-matched
/// codes disagree by ~800 px raw, but agree to sub-module after the
/// 180° remap). Identity is still tried first so a same-orientation
/// source stays exact.
fn mean_corner_err(
    gpu: &[[f64; 2]; 4],
    cpu: &[[f64; 2]; 4],
    frame_w: Option<f64>,
    frame_h: Option<f64>,
) -> f64 {
    let mut cands: Vec<[[f64; 2]; 4]> = vec![*cpu];
    if let (Some(w), Some(h)) = (frame_w, frame_h) {
        let mut rot180 = [[0.0; 2]; 4];
        for i in 0..4 {
            rot180[i] = [w - cpu[i][0], h - cpu[i][1]];
        }
        cands.push(rot180);
    }
    let mut best = f64::INFINITY;
    for cand in &cands {
        for rev in [false, true] {
            for rot in 0..4 {
                let mut sum = 0.0;
                for i in 0..4 {
                    let j = if rev {
                        (rot + 4 - i) % 4
                    } else {
                        (rot + i) % 4
                    };
                    let (dx, dy) = (gpu[i][0] - cand[j][0], gpu[i][1] - cand[j][1]);
                    sum += (dx * dx + dy * dy).sqrt();
                }
                best = best.min(sum / 4.0);
            }
        }
    }
    best
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn frame_path(frames_root: &Path, scan: &str, frame_idx: usize) -> PathBuf {
    // ffmpeg -f%05d is 1-indexed: frame 0 → f00001.png
    frames_root
        .join(scan)
        .join("all_frames")
        .join(format!("f{:05}.png", frame_idx + 1))
}

fn evaluate_scan(
    gold: &ScanGold,
    frames_root: &Path,
    cfg_name: &str,
    opts: &ScanOptions,
    session_period: Option<u32>,
    obs_only: bool,
    quiet: bool,
) -> ScanReport {
    let cfg = config_by_name(cfg_name);
    let mut session = session_period.map(|p| {
        ScanSession::new(
            cfg,
            SessionConfig {
                rotation_period: p,
                ..SessionConfig::default()
            },
        )
    });

    let frames_to_scan: Vec<usize> = if obs_only {
        gold.by_frame.keys().copied().collect()
    } else {
        // Scan every extracted frame that exists, up to gold.n_frames.
        let dir = frames_root.join(&gold.name).join("all_frames");
        let mut idxs = Vec::new();
        for i in 0..gold.n_frames {
            if frame_path(frames_root, &gold.name, i).exists()
                || dir.join(format!("f{:05}.png", i + 1)).exists()
            {
                idxs.push(i);
            }
        }
        if idxs.is_empty() {
            // fall back to obs frames only
            gold.by_frame.keys().copied().collect()
        } else {
            idxs
        }
    };

    let mut times = Vec::new();
    let mut obs_matched = 0usize;
    let mut obs_missed = 0usize;
    let mut frame_matched = 0usize;
    let mut frame_missed = 0usize;
    let mut extra_decode_frames = 0usize;
    let mut extra_decode_count = 0usize;
    let mut matched_ids: BTreeSet<String> = BTreeSet::new();
    let mut missed_ids: BTreeSet<String> = BTreeSet::new();
    let mut sample_misses: Vec<String> = Vec::new();
    let mut corner_errs: Vec<f64> = Vec::new();
    let mut refined_errs: Vec<f64> = Vec::new();
    let gpu_ids: BTreeSet<String> = gold
        .by_frame
        .values()
        .flat_map(|v| v.iter().map(|o| o.short_id.clone()))
        .collect();

    // When session is on and obs_only, we still need sequential frames for
    // temporal pooling — walk 0..max continuously but only score obs frames.
    let walk: Vec<usize> = if session.is_some() && obs_only {
        let max = gold.by_frame.keys().copied().max().unwrap_or(0);
        (0..=max).collect()
    } else {
        frames_to_scan.clone()
    };

    let score_set: BTreeSet<usize> = frames_to_scan.iter().copied().collect();

    for &frame_idx in &walk {
        let path = frame_path(frames_root, &gold.name, frame_idx);
        let Some((luma, w, h)) = load_png_luma(&path) else {
            if score_set.contains(&frame_idx) && !quiet {
                eprintln!("  missing frame {}", path.display());
            }
            // Advance session with empty state? better skip without advancing
            // rotation unfairly — still advance so indices stay aligned.
            if let Some(s) = session.as_mut() {
                // Feed a tiny blank so frame_index advances deterministically.
                let blank = vec![128u8; 64 * 48];
                let v = LumaView::new(&blank, 64, 48, 64).unwrap();
                let _ = s.scan_frame(&v, opts);
            }
            continue;
        };
        let view = LumaView::new(&luma, w, h, w).unwrap();
        let t0 = Instant::now();
        let det = if let Some(s) = session.as_mut() {
            s.scan_frame(&view, opts)
        } else {
            scan_robust(&view, opts, &cfg)
        };
        let ms = t0.elapsed().as_secs_f64() * 1e3;

        if !score_set.contains(&frame_idx) {
            continue;
        }
        times.push(ms);

        let payloads: Vec<String> = det.codes.iter().map(|c| c.code.payload.clone()).collect();
        let gpu = gold.by_frame.get(&frame_idx).cloned().unwrap_or_default();
        let gpu_ids_here: Vec<String> = gpu.iter().map(|o| o.short_id.clone()).collect();

        let mut matched = Vec::new();
        let mut missed = Vec::new();
        for o in &gpu {
            if let Some(code) = det
                .codes
                .iter()
                .find(|c| payload_matches_short_id(&c.code.payload, &o.short_id))
            {
                matched.push(o.short_id.clone());
                matched_ids.insert(o.short_id.clone());
                obs_matched += 1;
                // Prefer source-mapped coarse corners; refined when present.
                let fw = Some(w as f64);
                let fh = Some(h as f64);
                corner_errs.push(mean_corner_err(
                    &o.corners,
                    &code.corners_source,
                    fw,
                    fh,
                ));
                if let Some(ref rc) = code.refined_corners_source {
                    refined_errs.push(mean_corner_err(&o.corners, rc, fw, fh));
                }
            } else {
                missed.push(o.short_id.clone());
                missed_ids.insert(o.short_id.clone());
                obs_missed += 1;
                if sample_misses.len() < 24 {
                    sample_misses.push(format!(
                        "{} f{} id={} cpu={:?} finders={} trips={}",
                        gold.name,
                        frame_idx,
                        o.short_id,
                        payloads,
                        det.finders.len(),
                        det.triplets.len()
                    ));
                }
            }
        }
        if !gpu.is_empty() {
            if matched.is_empty() {
                frame_missed += 1;
            } else {
                frame_matched += 1;
            }
        }

        // Extras: CPU payloads that don't match any GPU shortId on this frame.
        let extras: Vec<String> = payloads
            .iter()
            .filter(|p| {
                !gpu_ids_here
                    .iter()
                    .any(|id| payload_matches_short_id(p, id))
            })
            .cloned()
            .collect();
        if !extras.is_empty() {
            extra_decode_frames += 1;
            extra_decode_count += extras.len();
        }

        if !quiet && (!matched.is_empty() || !missed.is_empty() || !extras.is_empty()) {
            let _hit = FrameHit {
                frame: frame_idx,
                gpu_ids: gpu_ids_here,
                cpu_payloads: payloads,
                matched,
                missed,
                extras,
                total_ms: ms,
                unified_finders: det.finders.len(),
                unified_triplets: det.triplets.len(),
            };
            // keep silent in quiet mode; detailed JSON is enough
        }
    }

    times.sort_by(|a, b| a.total_cmp(b));
    corner_errs.sort_by(|a, b| a.total_cmp(b));
    refined_errs.sort_by(|a, b| a.total_cmp(b));
    let n_gpu_obs: usize = gold.by_frame.values().map(|v| v.len()).sum();
    let n_gpu_obs_frames = gold.by_frame.len();
    // When obs_only, frame_matched denominator is n_gpu_obs_frames.
    // When full scan, same (we only score GPU frames for frame_recall).
    let frame_denom = frame_matched + frame_missed;
    let obs_denom = obs_matched + obs_missed;

    ScanReport {
        scan: gold.name.clone(),
        config: if let Some(p) = session_period {
            format!("{cfg_name}+session({p})")
        } else {
            cfg_name.to_string()
        },
        n_frames_scanned: times.len(),
        n_gpu_obs,
        n_gpu_obs_frames,
        obs_matched,
        obs_missed,
        obs_recall: if obs_denom > 0 {
            obs_matched as f64 / obs_denom as f64
        } else {
            0.0
        },
        frame_matched,
        frame_missed,
        frame_recall: if frame_denom > 0 {
            frame_matched as f64 / frame_denom as f64
        } else {
            0.0
        },
        extra_decode_frames,
        extra_decode_count,
        mean_ms: if times.is_empty() {
            0.0
        } else {
            times.iter().sum::<f64>() / times.len() as f64
        },
        p50_ms: percentile(&times, 0.50),
        p95_ms: percentile(&times, 0.95),
        max_ms: times.last().copied().unwrap_or(0.0),
        gpu_unique_ids: gpu_ids.len(),
        cpu_unique_ids_matched: matched_ids.len(),
        missed_ids: missed_ids.into_iter().collect(),
        sample_misses,
        corner_n: corner_errs.len(),
        corner_mean_err_px: if corner_errs.is_empty() {
            0.0
        } else {
            corner_errs.iter().sum::<f64>() / corner_errs.len() as f64
        },
        corner_p95_err_px: percentile(&corner_errs, 0.95),
        refined_corner_n: refined_errs.len(),
        refined_corner_mean_err_px: if refined_errs.is_empty() {
            0.0
        } else {
            refined_errs.iter().sum::<f64>() / refined_errs.len() as f64
        },
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut domain = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/real/full-domain-data");
    let mut frames_root = PathBuf::from("/tmp/domain_gold");
    let mut configs: Vec<String> = Vec::new();
    let mut max_dim: u32 = 1280;
    let mut session_period: Option<u32> = None;
    let mut obs_only = true;
    let mut quiet = false;
    let mut out_path: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--domain" => {
                i += 1;
                domain = PathBuf::from(&args[i]);
            }
            "--frames-root" => {
                i += 1;
                frames_root = PathBuf::from(&args[i]);
            }
            "--config" => {
                i += 1;
                configs.push(args[i].clone());
            }
            "--max-dim" => {
                i += 1;
                max_dim = args[i].parse().unwrap();
            }
            s if s == "--session" || s.starts_with("--session=") => {
                session_period = Some(
                    s.strip_prefix("--session=")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(3),
                );
            }
            "--obs-only" => obs_only = true,
            "--all-frames" => obs_only = false,
            "--quiet" => quiet = true,
            "--out" => {
                i += 1;
                out_path = Some(PathBuf::from(&args[i]));
            }
            other => panic!("unknown arg {other}"),
        }
        i += 1;
    }
    if configs.is_empty() {
        configs = vec!["baseline".into(), "robust-fast".into()];
    }
    // Refine on for domain eval: corner precision vs GPU is a first-class
    // acceptance metric (pose estimation is why this scanner exists).
    let opts = ScanOptions {
        max_working_dim: max_dim,
        refine: true,
    };

    let mut golds = Vec::new();
    let mut scans: Vec<PathBuf> = fs::read_dir(&domain)
        .unwrap_or_else(|e| panic!("domain dir {}: {e}", domain.display()))
        .filter_map(|e| {
            let p = e.ok()?.path();
            p.is_dir().then_some(p)
        })
        .collect();
    scans.sort();
    for scan_dir in &scans {
        if let Some(g) = parse_gold(scan_dir) {
            if !quiet {
                eprintln!(
                    "loaded {} — {} frames, {} GPU obs on {} frames, {} unique ids",
                    g.name,
                    g.n_frames,
                    g.by_frame.values().map(|v| v.len()).sum::<usize>(),
                    g.by_frame.len(),
                    g.by_frame
                        .values()
                        .flat_map(|v| v.iter().map(|o| o.short_id.as_str()))
                        .collect::<BTreeSet<_>>()
                        .len()
                );
            }
            golds.push(g);
        }
    }
    assert!(!golds.is_empty(), "no scans found under {}", domain.display());

    let mut reports = Vec::new();
    for cfg_name in &configs {
        for gold in &golds {
            if !quiet {
                eprintln!(
                    "\n=== {} / {} max_dim={} session={:?} obs_only={} ===",
                    gold.name, cfg_name, max_dim, session_period, obs_only
                );
            }
            let r = evaluate_scan(
                gold,
                &frames_root,
                cfg_name,
                &opts,
                session_period,
                obs_only,
                quiet,
            );
            if !quiet {
                eprintln!(
                    "  obs recall {:.1}% ({}/{})  frame recall {:.1}% ({}/{})  extras {} on {} frames",
                    100.0 * r.obs_recall,
                    r.obs_matched,
                    r.obs_matched + r.obs_missed,
                    100.0 * r.frame_recall,
                    r.frame_matched,
                    r.frame_matched + r.frame_missed,
                    r.extra_decode_count,
                    r.extra_decode_frames
                );
                eprintln!(
                    "  timing mean={:.1} p50={:.1} p95={:.1} max={:.1} ms  matched_ids={}/{}",
                    r.mean_ms,
                    r.p50_ms,
                    r.p95_ms,
                    r.max_ms,
                    r.cpu_unique_ids_matched,
                    r.gpu_unique_ids
                );
                if r.corner_n > 0 {
                    eprintln!(
                        "  corners n={} mean={:.2}px p95={:.2}px  refined n={} mean={:.2}px",
                        r.corner_n,
                        r.corner_mean_err_px,
                        r.corner_p95_err_px,
                        r.refined_corner_n,
                        r.refined_corner_mean_err_px
                    );
                }
                if !r.missed_ids.is_empty() {
                    eprintln!("  never-seen ids: {:?}", r.missed_ids);
                }
            }
            reports.push(r);
        }
    }

    // Aggregate per config across scans.
    let mut totals = Vec::new();
    for cfg_name in &configs {
        let label = if let Some(p) = session_period {
            format!("{cfg_name}+session({p})")
        } else {
            cfg_name.clone()
        };
        let rs: Vec<&ScanReport> = reports.iter().filter(|r| r.config == label).collect();
        let obs_matched: usize = rs.iter().map(|r| r.obs_matched).sum();
        let obs_missed: usize = rs.iter().map(|r| r.obs_missed).sum();
        let frame_matched: usize = rs.iter().map(|r| r.frame_matched).sum();
        let frame_missed: usize = rs.iter().map(|r| r.frame_missed).sum();
        let n_frames: usize = rs.iter().map(|r| r.n_frames_scanned).sum();
        let total_ms: f64 = rs
            .iter()
            .map(|r| r.mean_ms * r.n_frames_scanned as f64)
            .sum();
        let mut missed_ids: BTreeSet<String> = BTreeSet::new();
        for r in &rs {
            missed_ids.extend(r.missed_ids.iter().cloned());
        }
        // Sum of per-scan unique counts (ids can repeat across scans).
        let gpu_unique: usize = rs.iter().map(|r| r.gpu_unique_ids).sum();
        let cpu_matched_unique: usize = rs.iter().map(|r| r.cpu_unique_ids_matched).sum();
        let corner_n: usize = rs.iter().map(|r| r.corner_n).sum();
        let corner_mean = if corner_n > 0 {
            rs.iter()
                .map(|r| r.corner_mean_err_px * r.corner_n as f64)
                .sum::<f64>()
                / corner_n as f64
        } else {
            0.0
        };
        let refined_n: usize = rs.iter().map(|r| r.refined_corner_n).sum();
        let refined_mean = if refined_n > 0 {
            rs.iter()
                .map(|r| r.refined_corner_mean_err_px * r.refined_corner_n as f64)
                .sum::<f64>()
                / refined_n as f64
        } else {
            0.0
        };
        totals.push(ScanReport {
            scan: "ALL".into(),
            config: label,
            n_frames_scanned: n_frames,
            n_gpu_obs: obs_matched + obs_missed,
            n_gpu_obs_frames: frame_matched + frame_missed,
            obs_matched,
            obs_missed,
            obs_recall: if obs_matched + obs_missed > 0 {
                obs_matched as f64 / (obs_matched + obs_missed) as f64
            } else {
                0.0
            },
            frame_matched,
            frame_missed,
            frame_recall: if frame_matched + frame_missed > 0 {
                frame_matched as f64 / (frame_matched + frame_missed) as f64
            } else {
                0.0
            },
            extra_decode_frames: rs.iter().map(|r| r.extra_decode_frames).sum(),
            extra_decode_count: rs.iter().map(|r| r.extra_decode_count).sum(),
            mean_ms: if n_frames > 0 {
                total_ms / n_frames as f64
            } else {
                0.0
            },
            p50_ms: 0.0,
            p95_ms: rs.iter().map(|r| r.p95_ms).fold(0.0, f64::max),
            max_ms: rs.iter().map(|r| r.max_ms).fold(0.0, f64::max),
            gpu_unique_ids: gpu_unique,
            cpu_unique_ids_matched: cpu_matched_unique,
            missed_ids: missed_ids.into_iter().collect(),
            sample_misses: rs
                .iter()
                .flat_map(|r| r.sample_misses.iter().cloned())
                .take(40)
                .collect(),
            corner_n,
            corner_mean_err_px: corner_mean,
            corner_p95_err_px: rs.iter().map(|r| r.corner_p95_err_px).fold(0.0, f64::max),
            refined_corner_n: refined_n,
            refined_corner_mean_err_px: refined_mean,
        });
    }

    let report = Report {
        configs: reports,
        totals,
    };
    let json = serde_json::to_string_pretty(&report).unwrap();
    if let Some(p) = out_path {
        fs::write(&p, &json).unwrap();
        eprintln!("wrote {}", p.display());
    }
    println!("{json}");
}
