# Plan 2: Detection Front End Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `qr-lab-core` detects QR finder patterns and groups them into ordered triplet candidates on every fixture in the golden suite — both polarities, no white-plate assumption — with per-stage tracing and a compiling WASM target.

**Architecture:** One dense pass builds a 16×16 tile min/max grid (local thresholds + skip mask). A row-skipped scanline pass run-length-matches `1:1:3:1:1` in both polarities, cross-checks vertically and diagonally, and merges hits into verified finder candidates. Triplet grouping filters candidate triples by module-size ratio, leg balance, and corner angle, orders them TL/TR/BL, and estimates the symbol dimension. A square↔quad perspective transform (needed by tests now, by grid sampling in Plan 3) is its own tested unit. All stages record artifacts/timings into a `Trace` that compiles to nothing without the `debug-trace` feature.

**Tech Stack:** Rust (no new required deps; optional `serde` feature on qr-lab-core), `wasm-bindgen` + `serde-wasm-bindgen` in the new `qr-lab-wasm` crate.

## Global Constraints

- Rust MSRV 1.87; `qr-lab-core` keeps `#![forbid(unsafe_code)]` and zero *required* runtime deps (serde is optional, off by default; mobile builds stay dep-free).
- Pinned algorithm constants (single source: `crates/qr-lab-core/src/consts.rs`):
  - `TILE: usize = 16`; tile threshold = `(min+max)/2` after 3×3 tile dilation of extrema; `CONTRAST_FLOOR: u8 = 12` (tiles with `max-min < 12` are skip).
  - `ROW_STEP: usize = 2` (scan every 2nd row).
  - Run-pattern check (zxing variance rules): `unit = total/7.0`; runs 0,1,3,4 must satisfy `|run − unit| < unit/2`; run 2 must satisfy `|run − 3·unit| < 3·unit/2`; all runs ≥ 1 px.
  - Candidate merge: centers within `module` px on both axes AND module ratio ≤ 1.3 → weighted merge.
  - Verification: horizontal + vertical run-checks mandatory; ≥1 of the 2 diagonals must also pass.
  - Triplet filters: pairwise module ratio ≤ 1.5; same polarity; corner angle `|cos| ≤ 0.4`; legs `|1 − a/b| ≤ 0.5`; per-leg dimension estimate `round(leg/module) + 7`; legs must agree within 4; mean snapped to ≡1 (mod 4); snapped dimension ∈ [21, 177]; snap distance recorded as `snap_error`.
- Both polarities are first-class end to end (`inverted` flag on candidates and triplets); no stage assumes a white quiet zone (the trans_/invtrans_ fixtures gate this).
- Fixture gates use ground truth via the homography: expected finder centers at normalized `(3.5/n, 3.5/n)`, `((n−3.5)/n, 3.5/n)`, `(3.5/n, (n−3.5)/n)` mapped through square→quad(corners TL,TR,BR,BL), `n = 4·version + 17`. Position tolerance: `max(2.0, module_size_px)`.
- **Gate-failure protocol:** if a fixture gate fails against a faithful implementation, report exact numbers (fixture, code, distances, candidate list) as DONE_WITH_CONCERNS — never loosen a tolerance or constant silently; constants change only by controller decision recorded in this plan.
- **No overfitting (user directive):** the fixtures are highly idealized; they VERIFY behavior, they never drive tuning. Every constant must have a principled derivation (QR geometry or established practice — zxing/AprilTag) stated where it is defined; "makes fixture X pass" is not a rationale. Prefer fixing the algorithm over nudging a constant.
- Commits end with the repo's Claude co-author trailer.

## File Structure

```
crates/qr-lab-core/src/
  lib.rs           (add: mod consts, homography, tiles, finder, triplet, trace, scanner; re-exports)
  consts.rs        pinned constants above
  homography.rs    PerspectiveTransform (square→quad, inverse, quad→quad)
  luma.rs          (add luma_from_rgba helper)
  tiles.rs         TileGrid
  finder.rs        run matcher + scanline finder detection
  triplet.rs       triplet grouping
  trace.rs         Trace (debug-trace feature)
  scanner.rs       detect() orchestration + StageTimings
crates/qr-lab-core/examples/scan_fixture.rs   CLI: scan one fixture, print stages/timings
crates/qr-lab-wasm/   (new crate: cdylib, wasm-bindgen scan_rgba)
scripts/check-wasm.sh
```

---

### Task 1: Perspective transform (`homography.rs`)

**Files:**
- Create: `crates/qr-lab-core/src/homography.rs`, `crates/qr-lab-core/src/consts.rs`
- Modify: `crates/qr-lab-core/src/lib.rs`

**Interfaces:**
- Produces: `pub struct PerspectiveTransform` with
  `pub fn square_to_quad(q: [[f64; 2]; 4]) -> Self` (unit square (0,0),(1,0),(1,1),(0,1) → quad TL,TR,BR,BL),
  `pub fn map(&self, u: f64, v: f64) -> [f64; 2]`,
  `pub fn inverse(&self) -> Self` (adjugate — no determinant division needed for a homography).
  Consumed by Plan 2's fixture gates (expected finder centers) and Plan 3's grid sampling.

- [ ] **Step 1: Write the failing tests** (bottom of `homography.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const Q: [[f64; 2]; 4] =
        [[100.0, 50.0], [420.0, 80.0], [400.0, 380.0], [90.0, 350.0]];

    #[test]
    fn corners_map_exactly() {
        let h = PerspectiveTransform::square_to_quad(Q);
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for (i, [u, v]) in uv.iter().enumerate() {
            let p = h.map(*u, *v);
            assert!((p[0] - Q[i][0]).abs() < 1e-9, "corner {i}: {p:?}");
            assert!((p[1] - Q[i][1]).abs() < 1e-9, "corner {i}: {p:?}");
        }
    }

    #[test]
    fn affine_case_scales() {
        let h = PerspectiveTransform::square_to_quad(
            [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]);
        let p = h.map(0.25, 0.75);
        assert!((p[0] - 0.5).abs() < 1e-12 && (p[1] - 1.5).abs() < 1e-12);
    }

    #[test]
    fn inverse_round_trips() {
        let h = PerspectiveTransform::square_to_quad(Q);
        let inv = h.inverse();
        for i in 0..=10 {
            for j in 0..=10 {
                let (u, v) = (i as f64 / 10.0, j as f64 / 10.0);
                let p = h.map(u, v);
                let b = inv.map(p[0], p[1]);
                assert!((b[0] - u).abs() < 1e-9 && (b[1] - v).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn straight_lines_stay_straight() {
        // Projective invariant: collinear points stay collinear.
        let h = PerspectiveTransform::square_to_quad(Q);
        let a = h.map(0.0, 0.5);
        let b = h.map(0.5, 0.5);
        let c = h.map(1.0, 0.5);
        let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        assert!(cross.abs() < 1e-6, "cross={cross}");
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p qr-lab-core homography` → compile error.

- [ ] **Step 3: Implement** (zxing's PerspectiveTransform, unit-square variant)

```rust
/// 3x3 homography stored row-major as coefficients a11..a33.
/// Maps (u,v) -> ((a11*u + a21*v + a31)/w, (a12*u + a22*v + a32)/w),
/// w = a13*u + a23*v + a33.
#[derive(Clone, Copy, Debug)]
pub struct PerspectiveTransform {
    a11: f64, a21: f64, a31: f64,
    a12: f64, a22: f64, a32: f64,
    a13: f64, a23: f64, a33: f64,
}

impl PerspectiveTransform {
    /// Unit square (0,0),(1,0),(1,1),(0,1) -> quad [TL, TR, BR, BL].
    pub fn square_to_quad(q: [[f64; 2]; 4]) -> Self {
        let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = q;
        let dx3 = x0 - x1 + x2 - x3;
        let dy3 = y0 - y1 + y2 - y3;
        if dx3 == 0.0 && dy3 == 0.0 {
            Self {
                a11: x1 - x0, a21: x2 - x1, a31: x0,
                a12: y1 - y0, a22: y2 - y1, a32: y0,
                a13: 0.0, a23: 0.0, a33: 1.0,
            }
        } else {
            let dx1 = x1 - x2;
            let dx2 = x3 - x2;
            let dy1 = y1 - y2;
            let dy2 = y3 - y2;
            let den = dx1 * dy2 - dx2 * dy1;
            let a13 = (dx3 * dy2 - dx2 * dy3) / den;
            let a23 = (dx1 * dy3 - dx3 * dy1) / den;
            Self {
                a11: x1 - x0 + a13 * x1, a21: x3 - x0 + a23 * x3, a31: x0,
                a12: y1 - y0 + a13 * y1, a22: y3 - y0 + a23 * y3, a32: y0,
                a13, a23, a33: 1.0,
            }
        }
    }

    pub fn map(&self, u: f64, v: f64) -> [f64; 2] {
        let w = self.a13 * u + self.a23 * v + self.a33;
        [
            (self.a11 * u + self.a21 * v + self.a31) / w,
            (self.a12 * u + self.a22 * v + self.a32) / w,
        ]
    }

    /// Adjugate: inverse up to scale, which a homography ignores.
    pub fn inverse(&self) -> Self {
        Self {
            a11: self.a22 * self.a33 - self.a23 * self.a32,
            a21: self.a23 * self.a31 - self.a21 * self.a33,
            a31: self.a21 * self.a32 - self.a22 * self.a31,
            a12: self.a13 * self.a32 - self.a12 * self.a33,
            a22: self.a11 * self.a33 - self.a13 * self.a31,
            a32: self.a12 * self.a31 - self.a11 * self.a32,
            a13: self.a12 * self.a23 - self.a13 * self.a22,
            a23: self.a13 * self.a21 - self.a11 * self.a23,
            a33: self.a11 * self.a22 - self.a12 * self.a21,
        }
    }
}
```

`consts.rs` (used from Task 3 on; create now so the module exists):

```rust
pub const TILE: usize = 16;
pub const CONTRAST_FLOOR: u8 = 12;
pub const ROW_STEP: usize = 2;
```

`lib.rs`: add `mod consts; mod homography;` and `pub use homography::PerspectiveTransform;`.

- [ ] **Step 4: Run tests** — 4 new pass, all existing pass.
- [ ] **Step 5: Commit** — `feat: perspective transform + pinned detection constants`

---

### Task 2: RGBA→luma helper

**Files:** Modify `crates/qr-lab-core/src/luma.rs`, `lib.rs`.

**Interfaces:**
- Produces: `pub fn luma_from_rgba(rgba: &[u8], width: usize, height: usize) -> Vec<u8>` — BT.601 integer luma `y = (77·r + 150·g + 29·b + 128) >> 8`. Consumed by qr-lab-wasm (canvas readPixels) and future consumer adapters.

- [ ] **Step 1: Failing tests** (in `luma.rs` tests module)

```rust
    #[test]
    fn rgba_conversion_bt601() {
        let rgba = [255, 255, 255, 255,  0, 0, 0, 255,  255, 0, 0, 255];
        let y = luma_from_rgba(&rgba, 3, 1);
        assert_eq!(y, vec![255, 0, 76]); // (77*255+128)>>8 = 76
    }

    #[test]
    #[should_panic]
    fn rgba_wrong_len_panics() {
        luma_from_rgba(&[0; 10], 3, 1);
    }
```

- [ ] **Step 2: Verify failure, implement, verify pass**

```rust
pub fn luma_from_rgba(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    assert_eq!(rgba.len(), width * height * 4, "rgba buffer size mismatch");
    rgba.chunks_exact(4)
        .map(|p| {
            ((77 * p[0] as u32 + 150 * p[1] as u32 + 29 * p[2] as u32 + 128)
                >> 8) as u8
        })
        .collect()
}
```

- [ ] **Step 3: Commit** — `feat: rgba-to-luma input adapter`

---

### Task 3: Tile grid (`tiles.rs`)

**Files:** Create `crates/qr-lab-core/src/tiles.rs`; modify `lib.rs`.

**Interfaces:**
- Produces: `pub struct TileGrid { pub tiles_x: usize, pub tiles_y: usize, /* private: min, max, threshold: Vec<u8>, skip: Vec<bool> */ }`
  - `pub fn build(view: &LumaView) -> TileGrid` — per-16×16-tile min/max (edge tiles clipped), extrema dilated over the 3×3 tile neighborhood, `threshold = ((min as u16 + max as u16) / 2) as u8` from the **dilated** extrema, `skip = (dilated_max - dilated_min) < CONTRAST_FLOOR`.
  - `pub fn threshold_at(&self, x: usize, y: usize) -> u8` (pixel coords → owning tile)
  - `pub fn is_skip(&self, x: usize, y: usize) -> bool`
  - `pub fn row_all_skip(&self, y: usize) -> bool` (true iff every tile the row crosses is skip)

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::LumaView;

    fn img(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        (0..w * h).map(|i| f(i % w, i / w)).collect()
    }

    #[test]
    fn flat_image_is_all_skip() {
        let d = img(64, 48, |_, _| 128);
        let g = TileGrid::build(&LumaView::new(&d, 64, 48, 64).unwrap());
        assert_eq!((g.tiles_x, g.tiles_y), (4, 3));
        for y in (0..48).step_by(7) {
            assert!(g.row_all_skip(y));
            for x in (0..64).step_by(7) {
                assert!(g.is_skip(x, y));
                assert_eq!(g.threshold_at(x, y), 128);
            }
        }
    }

    #[test]
    fn contrast_tile_thresholds_midpoint_and_dilates() {
        // 64x48: left half black(20), right half white(220).
        let d = img(64, 48, |x, _| if x < 32 { 20 } else { 220 });
        let g = TileGrid::build(&LumaView::new(&d, 64, 48, 64).unwrap());
        // The boundary column of tiles sees both -> not skip, threshold 120.
        assert!(!g.is_skip(32, 24));
        assert_eq!(g.threshold_at(32, 24), 120);
        // One tile AWAY from the boundary: dilation pulls the extrema
        // across, so threshold is still 120 and not skip.
        assert!(!g.is_skip(16, 24));
        assert_eq!(g.threshold_at(16, 24), 120);
        assert!(!g.is_skip(48, 24));
        // Two tiles away (x=0..15 is 2 tiles from the boundary col at 32?
        // tiles are 16 wide: tile0 [0,16), tile1 [16,32), tile2 [32,48).
        // tile0 is adjacent to tile1 which touches the boundary via
        // dilation from tile2... tile0's 3x3 neighborhood = tiles 0,1 ->
        // all black -> skip.
        assert!(g.is_skip(0, 24));
    }

    #[test]
    fn ragged_edges_are_handled() {
        // 70x30 -> 5x2 tiles, last column 6px wide, last row 14px tall.
        let d = img(70, 30, |x, y| if x >= 64 && y >= 16 { 200 } else { 50 });
        let g = TileGrid::build(&LumaView::new(&d, 70, 30, 70).unwrap());
        assert_eq!((g.tiles_x, g.tiles_y), (5, 2));
        assert!(!g.is_skip(69, 29));
        assert_eq!(g.threshold_at(69, 29), 125);
    }
}
```

- [ ] **Step 2: Verify failure, implement**

```rust
use crate::consts::{CONTRAST_FLOOR, TILE};
use crate::LumaView;

pub struct TileGrid {
    pub tiles_x: usize,
    pub tiles_y: usize,
    threshold: Vec<u8>,
    skip: Vec<bool>,
}

impl TileGrid {
    pub fn build(view: &LumaView) -> TileGrid {
        let (w, h) = (view.width(), view.height());
        let tiles_x = w.div_ceil(TILE);
        let tiles_y = h.div_ceil(TILE);
        let mut mins = vec![255u8; tiles_x * tiles_y];
        let mut maxs = vec![0u8; tiles_x * tiles_y];
        for ty in 0..tiles_y {
            for y in ty * TILE..((ty + 1) * TILE).min(h) {
                let row = view.row(y);
                for tx in 0..tiles_x {
                    let s = &row[tx * TILE..((tx + 1) * TILE).min(w)];
                    let idx = ty * tiles_x + tx;
                    for &p in s {
                        mins[idx] = mins[idx].min(p);
                        maxs[idx] = maxs[idx].max(p);
                    }
                }
            }
        }
        let mut threshold = vec![0u8; tiles_x * tiles_y];
        let mut skip = vec![false; tiles_x * tiles_y];
        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let (mut lo, mut hi) = (255u8, 0u8);
                for ny in ty.saturating_sub(1)..=(ty + 1).min(tiles_y - 1) {
                    for nx in tx.saturating_sub(1)..=(tx + 1).min(tiles_x - 1) {
                        lo = lo.min(mins[ny * tiles_x + nx]);
                        hi = hi.max(maxs[ny * tiles_x + nx]);
                    }
                }
                let idx = ty * tiles_x + tx;
                threshold[idx] = ((lo as u16 + hi as u16) / 2) as u8;
                skip[idx] = hi - lo < CONTRAST_FLOOR;
            }
        }
        TileGrid { tiles_x, tiles_y, threshold, skip }
    }

    #[inline]
    pub fn threshold_at(&self, x: usize, y: usize) -> u8 {
        self.threshold[(y / TILE) * self.tiles_x + x / TILE]
    }

    #[inline]
    pub fn is_skip(&self, x: usize, y: usize) -> bool {
        self.skip[(y / TILE) * self.tiles_x + x / TILE]
    }

    pub fn row_all_skip(&self, y: usize) -> bool {
        let base = (y / TILE) * self.tiles_x;
        self.skip[base..base + self.tiles_x].iter().all(|&s| s)
    }
}
```

- [ ] **Step 3: Verify pass, commit** — `feat: tile min/max grid with local thresholds and skip mask`

---

### Task 4: Run-pattern matcher (`finder.rs`, part 1)

**Files:** Create `crates/qr-lab-core/src/finder.rs`; modify `lib.rs`.

**Interfaces:**
- Produces (consumed by Task 5 in the same file):
  - `pub(crate) struct RunHit { pub center: f64, pub module: f64, pub inverted: bool, pub start: usize, pub end: usize }` — `center` = midpoint of the middle run; `module = total/7`; `inverted` = middle run is ABOVE threshold (light); `start..end` = pixel span of the whole 5-run window.
  - `pub(crate) fn pattern_fits(runs: &[f64; 5]) -> bool` — the pinned variance rules.
  - `pub(crate) fn match_row(bits: impl Iterator<Item = bool>, out: &mut Vec<RunHit>)` where `bits` yields `pixel < threshold` (true = dark) per x; finds every 5-run window satisfying `pattern_fits`, both polarities (a window's polarity is the middle run's value).

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn bits(spec: &[(bool, usize)]) -> Vec<bool> {
        spec.iter().flat_map(|&(v, n)| std::iter::repeat(v).take(n)).collect()
    }

    #[test]
    fn pattern_fits_accepts_ideal_and_tolerant() {
        assert!(pattern_fits(&[4.0, 4.0, 12.0, 4.0, 4.0]));
        assert!(pattern_fits(&[3.0, 4.0, 13.0, 4.0, 5.0]));
        assert!(!pattern_fits(&[4.0, 4.0, 4.0, 4.0, 4.0]));   // middle not 3x
        assert!(!pattern_fits(&[1.0, 8.0, 12.0, 4.0, 4.0]));  // outer off
    }

    #[test]
    fn match_row_finds_dark_finder() {
        // light(10) dark(4) light(4) DARK(12) light(4) dark(4) light(10)
        let row = bits(&[(false, 10), (true, 4), (false, 4), (true, 12),
                         (false, 4), (true, 4), (false, 10)]);
        let mut hits = Vec::new();
        match_row(row.iter().copied(), &mut hits);
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert!(!h.inverted);
        assert!((h.module - 4.0).abs() < 1e-9);
        // middle run spans x in [18, 30) -> center 24.0 (pixel centers).
        assert!((h.center - 23.5).abs() <= 0.5, "center={}", h.center);
    }

    #[test]
    fn match_row_finds_inverted_finder() {
        // Same geometry, polarity flipped: middle run is LIGHT.
        let row = bits(&[(true, 10), (false, 4), (true, 4), (false, 12),
                         (true, 4), (false, 4), (true, 10)]);
        let mut hits = Vec::new();
        match_row(row.iter().copied(), &mut hits);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].inverted);
    }

    #[test]
    fn match_row_rejects_noise() {
        let row = bits(&[(false, 3), (true, 2), (false, 9), (true, 1),
                         (false, 5), (true, 7), (false, 2)]);
        let mut hits = Vec::new();
        match_row(row.iter().copied(), &mut hits);
        assert!(hits.is_empty());
    }

    #[test]
    fn match_row_finds_two_adjacent_finders() {
        let one = [(true, 4), (false, 4), (true, 12), (false, 4), (true, 4)];
        let mut spec = vec![(false, 8)];
        spec.extend_from_slice(&one);
        spec.push((false, 20));
        spec.extend_from_slice(&one);
        spec.push((false, 8));
        let mut hits = Vec::new();
        match_row(bits(&spec).iter().copied(), &mut hits);
        assert_eq!(hits.len(), 2);
    }
}
```

- [ ] **Step 2: Verify failure, implement**

Implementation contract: run-length encode the bit stream into `(value, start, len)` runs; slide a 5-run window; for each window where `pattern_fits` on the 5 lengths (as f64), emit a `RunHit` with `inverted = middle run's value == false`… careful — define: `bits` yields `is_dark`; a NORMAL finder's middle run is dark (`true`); `inverted = !middle_value`. `center = middle.start + middle.len/2 − 0.5` in pixel-center coordinates (so a run covering x∈[18,30) has center 23.5). Windows may overlap (two finders share a light gap run — the example above); advance run-by-run, not window-by-window.

```rust
pub(crate) fn pattern_fits(runs: &[f64; 5]) -> bool {
    let total: f64 = runs.iter().sum();
    if total < 7.0 {
        return false;
    }
    let unit = total / 7.0;
    let max_var = unit / 2.0;
    (runs[0] - unit).abs() < max_var
        && (runs[1] - unit).abs() < max_var
        && (runs[2] - 3.0 * unit).abs() < 3.0 * max_var
        && (runs[3] - unit).abs() < max_var
        && (runs[4] - unit).abs() < max_var
        && runs.iter().all(|&r| r >= 1.0)
}
```

- [ ] **Step 3: Verify pass, commit** — `feat: 1:1:3:1:1 run matcher, both polarities`

---

### Task 5: Scanline finder detection (`finder.rs`, part 2)

**Files:** Modify `crates/qr-lab-core/src/finder.rs`, `lib.rs`. Test: `crates/qr-lab-core/tests/finder_gate.rs`.

**Interfaces:**
- Produces: `pub struct FinderCandidate { pub x: f64, pub y: f64, pub module: f64, pub inverted: bool, pub hits: u32 }`
  `pub fn find_finders(view: &LumaView, grid: &TileGrid) -> Vec<FinderCandidate>`

**Algorithm contract (implementer fills bodies; constants from consts.rs):**
1. For `y in (0..h).step_by(ROW_STEP)`, skip rows where `grid.row_all_skip(y)`; binarize on the fly (`view.get(x,y) < grid.threshold_at(x,y)`), run `match_row`.
2. For each `RunHit`: **vertical cross-check** — from `(center, y)`, walk up and down counting runs in the same polarity (dark-light-dark… pattern centered on the middle module): collect the 5 vertical run lengths centered at y (walk while runs stay < `4*module` px); apply `pattern_fits`; on success compute `cy` = center of the vertical middle run. Then **diagonal cross-checks** from `(center, cy)` along (+1,+1)/(−1,−1) and (+1,−1)/(−1,+1); require `pattern_fits` on ≥1 of the two diagonals. (Walks clamp at image borders; a clamped run ends there.)
3. Verified hits become candidates at `(cx = hit.center, cy)` with `module = (h_module + v_module)/2`. **Merge** into the accumulator per the pinned rule (centers within `module` on both axes, module ratio ≤ 1.3, same polarity): weighted average by `hits` count.
4. Return candidates with `hits ≥ 2` (seen on ≥2 scan rows — a real finder ≥14 px tall at ROW_STEP=2 gets ≥3; noise rarely repeats) OR `module < 2·ROW_STEP` (tiny finders may hit once… far_ modules are 3.6 px → finder 25 px tall → ~12 rows; keep the ≥2 rule unconditional).
   Final rule, pinned: **`hits ≥ 2`, unconditional.**

- [ ] **Step 1: Synthetic unit test first** (in `finder.rs` tests)

```rust
    /// Paint an axis-aligned finder pattern (7x7 modules, scale px/module)
    /// at (ox, oy) into a light background.
    fn paint_finder(img: &mut [u8], w: usize, ox: usize, oy: usize,
                    scale: usize, ink: u8, bg_ring: u8) {
        for my in 0..7 {
            for mx in 0..7 {
                let dark = my == 0 || my == 6 || mx == 0 || mx == 6
                    || (1..=4).contains(&(mx as i32 - 1))
                        && (1..=4).contains(&(my as i32 - 1))
                        && (2..=4).contains(&mx) && (2..=4).contains(&my);
                let v = if dark { ink } else { bg_ring };
                for py in 0..scale {
                    for px in 0..scale {
                        img[(oy + my * scale + py) * w + ox + mx * scale + px] = v;
                    }
                }
            }
        }
    }

    #[test]
    fn detects_synthetic_finder_both_polarities() {
        let (w, h) = (160, 160);
        for inverted in [false, true] {
            let (ink, bg) = if inverted { (230, 30) } else { (25, 225) };
            let mut img = vec![bg; w * h];
            paint_finder(&mut img, w, 40, 56, 6, ink, bg);
            let view = crate::LumaView::new(&img, w, h, w).unwrap();
            let grid = crate::tiles::TileGrid::build(&view);
            let f = find_finders(&view, &grid);
            assert_eq!(f.len(), 1, "inverted={inverted}: {f:?}");
            assert_eq!(f[0].inverted, inverted);
            // center = (40 + 3.5*6, 56 + 3.5*6) = (61, 77) at pixel centers
            assert!((f[0].x - 60.5).abs() < 1.5, "{:?}", f[0]);
            assert!((f[0].y - 76.5).abs() < 1.5, "{:?}", f[0]);
            assert!((f[0].module - 6.0).abs() < 1.0);
        }
    }
```

(Note the dark-cell predicate above is just "outer ring + 3×3 center"; write it plainly: `let dark = my == 0 || my == 6 || mx == 0 || mx == 6 || ((2..=4).contains(&mx) && (2..=4).contains(&my));`.)

- [ ] **Step 2: Fixture gate test** — `crates/qr-lab-core/tests/finder_gate.rs`

```rust
mod common;

use qr_lab_core::{find_finders, LumaView, PerspectiveTransform, TileGrid};

fn expected_centers(c: &common::CodeTruth) -> [[f64; 2]; 3] {
    let n = (4 * c.version + 17) as f64;
    let h = PerspectiveTransform::square_to_quad(c.corners_px);
    let f = 3.5 / n;
    let g = (n - 3.5) / n;
    [h.map(f, f), h.map(g, f), h.map(f, g)]
}

#[test]
fn every_ground_truth_finder_is_detected() {
    let mut missed: Vec<String> = Vec::new();
    let mut worst_fp = 0usize;
    for fx in common::load_all() {
        let view = fx.view();
        let grid = TileGrid::build(&view);
        let found = find_finders(&view, &grid);
        worst_fp = worst_fp.max(found.len());
        for c in &fx.codes {
            let tol = c.module_size_px.max(2.0);
            for (k, e) in expected_centers(c).iter().enumerate() {
                let best = found
                    .iter()
                    .map(|f| ((f.x - e[0]).powi(2) + (f.y - e[1]).powi(2)).sqrt())
                    .fold(f64::INFINITY, f64::min);
                if best > tol {
                    missed.push(format!(
                        "{} code v{} finder {k}: nearest {best:.2}px (tol {tol:.2})",
                        fx.name, c.version));
                }
            }
        }
    }
    assert!(missed.is_empty(), "missed finders:\n{}", missed.join("\n"));
    // Candidate-explosion guard, not a precision gate.
    assert!(worst_fp < 600, "candidate explosion: {worst_fp}");
}
```

- [ ] **Step 3: Verify both fail** (no `find_finders`), implement per the contract, iterate until green. If the fixture gate cannot be met faithfully, follow the Gate-failure protocol (exact misses in the report).
- [ ] **Step 4: Full suite green, commit** — `feat: scanline finder detection with cross-checks and merging`

---

### Task 6: Triplet grouping (`triplet.rs`)

**Files:** Create `crates/qr-lab-core/src/triplet.rs`; modify `lib.rs`. Test: `crates/qr-lab-core/tests/triplet_gate.rs`.

**Interfaces:**
- Produces: `pub struct TripletCandidate { pub tl: [f64; 2], pub tr: [f64; 2], pub bl: [f64; 2], pub module: f64, pub dimension: u32, pub snap_error: f64, pub inverted: bool }`
  `pub fn group_triplets(finders: &[FinderCandidate]) -> Vec<TripletCandidate>`

**Algorithm contract (amended — recorded decision after the first gate run):** the original contract derived dimension from the scan-measured `FinderCandidate.module`, which is systematically biased by `1/cos(rotation)` (axis-aligned scan lines cross a rotated grid along a chord; measured 1.10–1.31× on rotated fixtures) and cannot represent per-axis perspective foreshortening at 45° tilt. Fix per established practice (zxing `Detector.calculateModuleSize`): measure module **along each leg**.

`group_triplets(view: &LumaView, grid: &TileGrid, finders: &[FinderCandidate]) -> Vec<TripletCandidate>`:
1. Geometry filters (image-free, as before): equal `inverted`, pairwise scan-module ratio ≤ 1.5 (coarse pre-filter only), corner finder = smallest `|cos|` of between-leg angle, `|cos| ≤ 0.4`, leg balance `|1 − a/b| ≤ 0.5`, canonical order via cross product (mirrored → swap, never reject).
2. Per-leg module (zxing semantics): `bwb(A→B)` = walk the binarized line (DDA, polarity-aware ink test vs tile thresholds) from finder center A toward B, measuring the ink(1.5)+space(1)+ink(1) crossing = 3.5 modules of distance; `bwb_both(A,B)` = that walk plus the same walk from A directly away from B (another 3.5 modules) — 7 modules total; `module_leg = (bwb_both(A,B) + bwb_both(B,A)) / 14`. A walk truncated by the image border contributes its partner's doubled value (zxing's border correction).
3. `dimension_leg = round(dist/module_leg) + 7`; legs agree within `max(4.0, 2.0·(dim_mean − 7.0)/(7.0·module_mean))` — second recorded amendment: the fixed ≤4 bound is provably exceeded by pure DDA quantization noise at large n / small modules (±1 px at each end of a 7-module BWB crossing → per-leg module error δm ≈ 2/14 px → per-leg dimension noise ≈ (n−7)·δm/module; two legs → 2×). False triples that survive the leg-balance ≤0.5 filter disagree by an order of magnitude more, so the gate keeps its purpose. Mean → snap ≡1 (mod 4); `snap_error`; snapped ∈ [21,177]. `TripletCandidate.module` = mean of the two leg modules. Sort by `snap_error` ascending, cap 64.

Unit tests paint synthetic finder patterns into small images (shared test helper) for the accept paths; pure-geometry reject tests need no image.

- [ ] **Step 1: Unit tests** (in `triplet.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder::FinderCandidate;

    fn f(x: f64, y: f64) -> FinderCandidate {
        FinderCandidate { x, y, module: 4.0, inverted: false, hits: 3 }
    }

    #[test]
    fn groups_axis_aligned_v1() {
        // v1: n=21, leg = 14 modules = 56px.
        let t = group_triplets(&[f(100.0, 100.0), f(156.0, 100.0), f(100.0, 156.0)]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].dimension, 21);
        assert_eq!(t[0].tl, [100.0, 100.0]);
        assert_eq!(t[0].tr, [156.0, 100.0]);
        assert_eq!(t[0].bl, [100.0, 156.0]);
        assert!(t[0].snap_error < 0.6);
    }

    #[test]
    fn canonicalizes_mirrored_order() {
        // Same three points fed so the natural order is mirrored;
        // output must still satisfy the cross-product convention.
        let t = group_triplets(&[f(100.0, 100.0), f(100.0, 156.0), f(156.0, 100.0)]);
        assert_eq!(t.len(), 1);
        let (tl, tr, bl) = (t[0].tl, t[0].tr, t[0].bl);
        let cross = (tr[0] - tl[0]) * (bl[1] - tl[1])
            - (tr[1] - tl[1]) * (bl[0] - tl[0]);
        assert!(cross > 0.0);
    }

    #[test]
    fn rejects_mixed_polarity_and_bad_geometry() {
        let mut inv = f(156.0, 100.0);
        inv.inverted = true;
        assert!(group_triplets(&[f(100.0, 100.0), inv, f(100.0, 156.0)]).is_empty());
        // Collinear points: no ~90° corner.
        assert!(group_triplets(&[f(0.0, 0.0), f(50.0, 0.0), f(100.0, 0.0)]).is_empty());
        // Legs too unbalanced (14 vs 40 modules).
        assert!(group_triplets(&[f(0.0, 0.0), f(56.0, 0.0), f(0.0, 160.0)]).is_empty());
    }

    #[test]
    fn rotated_45_still_groups() {
        let t = group_triplets(&[
            f(200.0, 200.0),
            f(200.0 + 39.6, 200.0 + 39.6),
            f(200.0 - 39.6, 200.0 + 39.6),
        ]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].dimension, 21);
    }
}
```

- [ ] **Step 2: Fixture gate** — `crates/qr-lab-core/tests/triplet_gate.rs`: same shape as the finder gate; for every code, at least one triplet must have all three of {tl,tr,bl} within `max(2.0, module_size_px)` of the expected three centers **as a set** (mirrored fixtures land with tr/bl swapped — match set-wise), `inverted == truth.inverted`, and `dimension` within ±4 of `4·version+17` (dimension refinement is Plan 3's job; this bounds gross errors). Reuse `expected_centers` by moving it into `tests/common/mod.rs` as `pub fn expected_finder_centers(c: &CodeTruth) -> [[f64; 2]; 3]` (update `finder_gate.rs` to use it from common).

```rust
mod common;

use qr_lab_core::{find_finders, group_triplets, LumaView, TileGrid};

#[test]
fn every_code_yields_a_matching_triplet() {
    let mut missed = Vec::new();
    for fx in common::load_all() {
        let view = fx.view();
        let grid = TileGrid::build(&view);
        let trips = group_triplets(&find_finders(&view, &grid));
        for c in &fx.codes {
            let exp = common::expected_finder_centers(c);
            let tol = c.module_size_px.max(2.0);
            let n = 4 * c.version + 17;
            let ok = trips.iter().any(|t| {
                t.inverted == c.inverted
                    && (t.dimension as i64 - n as i64).abs() <= 4
                    && [t.tl, t.tr, t.bl].iter().all(|p| {
                        exp.iter().any(|e| {
                            ((p[0] - e[0]).powi(2) + (p[1] - e[1]).powi(2)).sqrt() <= tol
                        })
                    })
            });
            if !ok {
                missed.push(format!("{} v{}: no matching triplet ({} cands)",
                                    fx.name, c.version, trips.len()));
            }
        }
    }
    assert!(missed.is_empty(), "unmatched codes:\n{}", missed.join("\n"));
}
```

- [ ] **Step 3: Implement, iterate to green (gate-failure protocol on faithful failure), full suite green.**
- [ ] **Step 4: Commit** — `feat: finder triplet grouping with dimension estimate`

---

### Task 7: Trace + `detect()` orchestration + example

**Files:** Create `crates/qr-lab-core/src/trace.rs`, `crates/qr-lab-core/src/scanner.rs`, `crates/qr-lab-core/examples/scan_fixture.rs`; modify `lib.rs`, `crates/qr-lab-core/Cargo.toml`.

**Interfaces:**
- `Cargo.toml` gains `[features] debug-trace = [] serde = ["dep:serde"]` with serde optional (`serde = { version = "1", features = ["derive"], optional = true }` moved/added as optional dep; keep the dev-dependency too).
- `pub struct StageTimings { pub tiles_ns: u64, pub finders_ns: u64, pub triplets_ns: u64 }`
- `pub struct Detections { pub finders: Vec<FinderCandidate>, pub triplets: Vec<TripletCandidate>, pub timings: StageTimings }`
- `pub fn detect(view: &LumaView) -> Detections` and `pub fn detect_traced(view: &LumaView, trace: &mut Trace) -> Detections`
- `pub struct Trace` — ALWAYS exists. Without `debug-trace` it is a unit struct with `#[inline]` no-op methods (`Trace::new()`, `record_tiles(&TileGrid)`, `record_finders(&[FinderCandidate])`, `record_triplets(&[TripletCandidate])`) so `detect_traced` compiles either way and the optimizer erases it. With the feature, it stores: `pub tiles: Option<TileTrace>` (tiles_x/tiles_y/thresholds/skip as vecs), `pub finders: Vec<FinderCandidate>`, `pub triplets: Vec<TripletCandidate>`. All public detection structs get `#[cfg_attr(feature = "serde", derive(serde::Serialize))]`.
- Timings via `std::time::Instant` (in `detect`, not in stage code).

- [ ] **Step 1: Tests** (in `scanner.rs`; plus a feature-matrix check in CI steps)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::LumaView;

    #[test]
    fn detect_on_flat_image_is_empty_and_fast_path() {
        let d = vec![128u8; 320 * 240];
        let view = LumaView::new(&d, 320, 240, 320).unwrap();
        let det = detect(&view);
        assert!(det.finders.is_empty());
        assert!(det.triplets.is_empty());
    }

    #[cfg(feature = "debug-trace")]
    #[test]
    fn trace_records_stages() {
        let d = vec![128u8; 64 * 64];
        let view = LumaView::new(&d, 64, 64, 64).unwrap();
        let mut tr = Trace::new();
        let _ = detect_traced(&view, &mut tr);
        let tiles = tr.tiles.as_ref().expect("tiles recorded");
        assert_eq!(tiles.thresholds.len(), 4 * 4);
    }
}
```

- [ ] **Step 2: Implement; verify all four feature combinations compile & pass:**

```bash
cargo test -p qr-lab-core
cargo test -p qr-lab-core --features debug-trace
cargo check -p qr-lab-core --features serde
cargo check -p qr-lab-core --features "debug-trace serde"
```

- [ ] **Step 3: Example** — `examples/scan_fixture.rs`: `cargo run -p qr-lab-core --example scan_fixture -- near_00` loads `fixtures/<name>.{json,luma}` (reuse the same serde structs inline — examples can't use tests/common), runs `detect`, prints per-stage µs, candidate/triplet counts, and each triplet's centers/dimension. Verify it runs on `near_00` and `inv_00`.
- [ ] **Step 4: Commit** — `feat: detect() orchestration, debug-trace feature, scan_fixture example`

---

### Task 8: `qr-lab-wasm` crate

**Files:** Create `crates/qr-lab-wasm/Cargo.toml`, `crates/qr-lab-wasm/src/lib.rs`, `scripts/check-wasm.sh`; modify root `Cargo.toml` (workspace member).

**Interfaces:**
- `qr-lab-wasm` (crate-type `["cdylib", "rlib"]`), deps: `qr-lab-core` with `features = ["debug-trace", "serde"]`, `wasm-bindgen = "0.2"`, `serde-wasm-bindgen = "0.6"`, `serde`.
- Exports `#[wasm_bindgen] pub fn scan_rgba(rgba: &[u8], width: u32, height: u32, with_trace: bool) -> JsValue` — converts via `luma_from_rgba`, runs `detect`/`detect_traced`, returns `serde_wasm_bindgen::to_value(&WasmResult { detections, trace: Option<Trace> })`.
- `scripts/check-wasm.sh`: `rustup target add wasm32-unknown-unknown 2>/dev/null; cargo check -p qr-lab-wasm --target wasm32-unknown-unknown` (the debug UI's real packaging via wasm-pack is Plan 3).

- [ ] **Step 1: Native test first** (qr-lab-wasm builds for native too via rlib):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn scan_result_shape_is_serializable() {
        // Round-trip the result struct through serde_json natively to pin
        // the field names the debug UI will consume.
        let d = vec![128u8; 32 * 32 * 4];
        let luma = qr_lab_core::luma_from_rgba(&d, 32, 32);
        let view = qr_lab_core::LumaView::new(&luma, 32, 32, 32).unwrap();
        let det = qr_lab_core::detect(&view);
        let json = serde_json::to_value(&det).unwrap();
        assert!(json.get("finders").is_some());
        assert!(json.get("triplets").is_some());
        assert!(json.get("timings").is_some());
    }
}
```

(add `serde_json` as qr-lab-wasm dev-dependency)

- [ ] **Step 2: Implement crate; run** `cargo test -p qr-lab-wasm` **and** `bash scripts/check-wasm.sh` (must exit 0).
- [ ] **Step 3: Commit** — `feat: qr-lab-wasm crate with scan_rgba binding`

---

## Self-review notes

- **Spec coverage (milestone 2):** stages 1–3 ✓ (Tasks 3–6), polarity-agnostic + no-plate ✓ (gates run on all 81 fixtures incl. inv/trans), trace ✓ (Task 7), WASM ✓ (Task 8), homography pulled forward ✓ (Task 1, needed by gates + Plan 3). Threading/NEON deliberately absent (spec: added only when measured — device measurement is Plan 5).
- **Type consistency:** `FinderCandidate`/`TripletCandidate`/`Trace`/`Detections` names and fields match across Tasks 4–8; `expected_finder_centers` lives in tests/common from Task 6 on (Task 5 defines it locally first, Task 6 moves it — acceptable churn, flagged here so the implementer knows it's intentional).
- **Known risks:** the finder/triplet fixture gates are the first time the detector meets far_ (3.6 px modules) and combo_ (far+45°+rotation) — constants may genuinely need tuning; the gate-failure protocol channels that through the controller instead of silent loosening. The `hits ≥ 2` rule and the 600-candidate guard are first guesses, revisable by recorded decision.

## Post-merge follow-ups (recorded at final review, 2026-07-04)

- **Plan 5 prerequisite (before NEON):** segment-level skip-tile scanning in `find_finders` — spec stage 2 says skip row *segments* inside skip-tiles; currently only whole-skip rows are skipped, so flat regions still pay binarize+RLE. Restructure scanlines to tile-aligned segments (mind runs spanning segment edges) BEFORE vectorizing. Host release baseline to beat: tiles ~1–2ms, finders ~4.5–6ms @720p.
- **Plan 3 inputs:** (a) cap `group_triplets` candidate input (top-K by hits, K = max-codes-per-frame × 3 + headroom) — O(n³) is unbounded on QR-dense scenes; (b) consider exposing per-leg modules on `TripletCandidate` for dimension refinement; (c) ver_12_v40 dimension estimate lands 173 vs true 177 — version-info bits / timing counting MUST override the triplet estimate (this is a requirement, not folklore); (d) concentric-ring finder verification (spec stage 2, zxing-cpp style) was deferred — the vertical+diagonal cross-checks stand in; revisit when real-capture false-positive rates are known.
