//! Golden-fixture loader shared by qrk-core integration tests.
//! Schema contract: tools/fixtures/generate.py (spec §6).
use std::fs;
use std::path::PathBuf;

use qrk_core::{LumaView, PerspectiveTransform};
use serde::Deserialize;

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct CodeTruth {
    pub payload: String,
    pub version: u32,
    pub ecc: String,
    pub mirrored: bool,
    pub module_size_px: f64,
    pub corners_px: [[f64; 2]; 4],
    pub inverted: bool,
    pub opaque_plate: bool,
    /// Plan 6 degraded-fixture expectations (absent on the legacy golden
    /// families ⇒ default `true`/`0`): whether the BASELINE scanner is
    /// physically expected to detect/decode this code (rules documented in
    /// tools/fixtures/scenarios.py `expectations()`). Legacy gates stay
    /// 100%-exact on the golden set; codes with `expect_decode == false`
    /// exist to measure the robustness ladder's headroom and are reported,
    /// not gated.
    #[serde(default = "default_true")]
    pub expect_detect: bool,
    #[serde(default = "default_true")]
    pub expect_decode: bool,
    #[serde(default)]
    pub difficulty: u8,
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
    /// Present (non-null) only on Plan 6 degraded fixtures.
    #[serde(default)]
    degradations: Option<serde_json::Value>,
}

// `name`/`codes` unused by `decode_trace_gate.rs` (Plan 4 Task 6), which
// only needs `view()` + `width`/`height` — same "each integration test
// binary compiles `common` independently" situation as `CodeTruth` above.
#[allow(dead_code)]
pub struct Fixture {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub luma: Vec<u8>,
    pub codes: Vec<CodeTruth>,
    /// `true` for Plan 6 degraded fixtures (a `degradations` key in the
    /// JSON): these are excluded from the 100%-exact golden gates and
    /// measured by the robustness gate / benchmark instead.
    pub degraded: bool,
}

impl Fixture {
    pub fn view(&self) -> LumaView<'_> {
        LumaView::new(&self.luma, self.width, self.height, self.width).unwrap()
    }
}

fn suite_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn load(name: &str) -> Fixture {
    let dir = suite_dir();
    let json_text = fs::read_to_string(dir.join(format!("{name}.json")))
        .unwrap_or_else(|e| panic!("{name}.json: {e}"));
    let meta: Meta =
        serde_json::from_str(&json_text).unwrap_or_else(|e| panic!("{name}.json: {e}"));
    let luma =
        fs::read(dir.join(format!("{name}.luma"))).unwrap_or_else(|e| panic!("{name}.luma: {e}"));
    assert_eq!(luma.len(), meta.width * meta.height, "{name}: luma size");
    Fixture {
        name: meta.name,
        width: meta.width,
        height: meta.height,
        luma,
        codes: meta.codes,
        degraded: meta.degradations.is_some(),
    }
}

/// The legacy golden suite only — every fixture WITHOUT degradations. The
/// 100%-exact gates (decode gate 1, refine gate, smoke ink probes) run on
/// this subset; degraded fixtures are measured, not gated at 100%.
#[allow(dead_code)]
pub fn load_golden() -> Vec<Fixture> {
    load_all().into_iter().filter(|f| !f.degraded).collect()
}

/// Ground-truth pixel centers of a code's three finder patterns (TL, TR,
/// BL), derived by mapping the finder-center points in the unit square
/// (module coordinates, 3.5 modules in from each relevant edge — the
/// center of a 7x7-module finder) through the code's known
/// square-to-quad homography. Shared by `finder_gate.rs` (per-finder
/// match) and `triplet_gate.rs` (per-triplet match) — each integration
/// test binary compiles `common` independently, so a helper unused by a
/// given binary (e.g. `fixtures_smoke.rs`, which uses neither gate) would
/// otherwise warn dead_code there; same treatment as `CodeTruth` above.
#[allow(dead_code)]
pub fn expected_finder_centers(c: &CodeTruth) -> [[f64; 2]; 3] {
    let n = (4 * c.version + 17) as f64;
    let h = PerspectiveTransform::square_to_quad(c.corners_px)
        .expect("ground-truth quad is never degenerate");
    let f = 3.5 / n;
    let g = (n - 3.5) / n;
    [h.map(f, f), h.map(g, f), h.map(f, g)]
}

// Unused by `decode_trace_gate.rs` (Plan 4 Task 6), which loads a single
// named fixture via `load` — same per-binary situation as above.
#[allow(dead_code)]
pub fn load_all() -> Vec<Fixture> {
    let mut names: Vec<String> = fs::read_dir(suite_dir())
        .expect("fixtures/ missing — run tools/fixtures/generate.py")
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension()? == "json").then(|| p.file_stem().unwrap().to_str().unwrap().to_string())
        })
        .collect();
    names.sort();
    names.iter().map(|n| load(n)).collect()
}
