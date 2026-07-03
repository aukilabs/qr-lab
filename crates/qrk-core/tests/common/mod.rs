//! Golden-fixture loader shared by qrk-core integration tests.
//! Schema contract: tools/fixtures/generate.py (spec §6).
use std::fs;
use std::path::PathBuf;

use qrk_core::LumaView;
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
}

#[derive(Deserialize)]
struct Meta {
    name: String,
    width: usize,
    height: usize,
    codes: Vec<CodeTruth>,
}

pub struct Fixture {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub luma: Vec<u8>,
    pub codes: Vec<CodeTruth>,
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
    let luma = fs::read(dir.join(format!("{name}.luma")))
        .unwrap_or_else(|e| panic!("{name}.luma: {e}"));
    assert_eq!(luma.len(), meta.width * meta.height, "{name}: luma size");
    Fixture {
        name: meta.name,
        width: meta.width,
        height: meta.height,
        luma,
        codes: meta.codes,
    }
}

pub fn load_all() -> Vec<Fixture> {
    let mut names: Vec<String> = fs::read_dir(suite_dir())
        .expect("fixtures/ missing — run tools/fixtures/generate.py")
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension()? == "json")
                .then(|| p.file_stem().unwrap().to_str().unwrap().to_string())
        })
        .collect();
    names.sort();
    names.iter().map(|n| load(n)).collect()
}
