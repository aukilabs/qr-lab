//! Alignment-pattern location: ISO 18004 Annex E's per-version coordinate
//! table, parallelogram position prediction from already-located
//! neighbors, and a concentric re-centering probe that turns a rough
//! prediction into a precise pixel-space anchor.
//!
//! An alignment pattern is a 5×5-module target (1-module dark border
//! ring, 1-module light ring, single dark center module — ISO 18004
//! §6.3.6) placed at every `(row, col)` pair drawn from a version's shared
//! coordinate set, except the three pairs that coincide with a finder
//! pattern's own corner. This file locates all of them for one candidate
//! code: [`alignment_coords`] gives the coordinate set, [`AlignmentGrid`]
//! holds the per-node results, and [`locate_alignment_patterns`] walks the
//! grid in raster order, predicting each node's position (parallelogram
//! rule from already-found neighbors, or the caller's provisional
//! transform when neighbors aren't available) and re-centering with a
//! ±[`crate::consts::ALIGNMENT_PROBE_HALF_MODULES`]-module concentric
//! probe.
//!
//! `sample.rs` consumes [`AlignmentGrid`] as its anchor lookup for building
//! per-region sampling transforms; `decode.rs` (Task 5) wires this file into
//! the per-candidate pipeline as a real caller, so the module-level
//! `dead_code` exemption that used to live here is removed.

use crate::consts::ALIGNMENT_PROBE_HALF_MODULES;
use crate::homography::PerspectiveTransform;
use crate::tiles::TileGrid;
use crate::trace::AlignmentTraceEntry;
use crate::version::sample_module_ink;
use crate::LumaView;

/// ISO 18004 Annex E alignment-pattern coordinate table, transcribed
/// verbatim from zxing's `Version.java` `ALIGNMENT_PATTERN_POSITIONS`
/// (index 0 = v1, empty — v1 has no alignment patterns at all — through
/// index 39 = v40). Cross-checked against this crate's own `qrcode`
/// dev-dependency's copy of the identical table (`qrcode-0.14.1`'s
/// `canvas.rs`, `static ALIGNMENT_PATTERN_POSITIONS`, which that crate uses
/// to *draw* the patterns this file's synthetic-render tests rasterize) —
/// so the table used here to *locate* patterns is guaranteed consistent
/// with the table used to draw them in every test in this file, and both
/// independently agree with the ISO/zxing reference values.
///
/// Each entry is the version's single shared row/column coordinate set —
/// real alignment-pattern centers are every `(row, col)` pair drawn from
/// crossing the set with itself (see [`locate_alignment_patterns`]).
/// Coordinates are 0-indexed module *centers*: a listed value `c` denotes
/// the pattern's center module, whose continuous module-space center is
/// `c + 0.5` — the same `index + 0.5` convention `version.rs` already uses
/// for its own module-center sampling.
const ALIGNMENT_COORDS: [&[u8]; 40] = [
    &[],                                  // v1
    &[6, 18],                             // v2
    &[6, 22],                             // v3
    &[6, 26],                             // v4
    &[6, 30],                             // v5
    &[6, 34],                             // v6
    &[6, 22, 38],                         // v7
    &[6, 24, 42],                         // v8
    &[6, 26, 46],                         // v9
    &[6, 28, 50],                         // v10
    &[6, 30, 54],                         // v11
    &[6, 32, 58],                         // v12
    &[6, 34, 62],                         // v13
    &[6, 26, 46, 66],                     // v14
    &[6, 26, 48, 70],                     // v15
    &[6, 26, 50, 74],                     // v16
    &[6, 30, 54, 78],                     // v17
    &[6, 30, 56, 82],                     // v18
    &[6, 30, 58, 86],                     // v19
    &[6, 34, 62, 90],                     // v20
    &[6, 28, 50, 72, 94],                 // v21
    &[6, 26, 50, 74, 98],                 // v22
    &[6, 30, 54, 78, 102],                // v23
    &[6, 28, 54, 80, 106],                // v24
    &[6, 32, 58, 84, 110],                // v25
    &[6, 30, 58, 86, 114],                // v26
    &[6, 34, 62, 90, 118],                // v27
    &[6, 26, 50, 74, 98, 122],            // v28
    &[6, 30, 54, 78, 102, 126],           // v29
    &[6, 26, 52, 78, 104, 130],           // v30
    &[6, 30, 56, 82, 108, 134],           // v31
    &[6, 34, 60, 86, 112, 138],           // v32
    &[6, 30, 58, 86, 114, 142],           // v33
    &[6, 34, 62, 90, 118, 146],           // v34
    &[6, 30, 54, 78, 102, 126, 150],      // v35
    &[6, 24, 50, 76, 102, 128, 154],      // v36
    &[6, 28, 54, 80, 106, 132, 158],      // v37
    &[6, 32, 58, 84, 110, 136, 162],      // v38
    &[6, 26, 54, 82, 110, 138, 166],      // v39
    &[6, 30, 58, 86, 114, 142, 170],      // v40
];

/// The alignment-pattern coordinate set for `version`.
///
/// # Panics
/// Panics (via out-of-bounds slice indexing) if `version` is outside
/// `1..=40` — every caller in this crate works from a version already
/// validated against that range (triplet dimension estimate or a
/// successfully BCH-decoded version-info reading).
pub(crate) fn alignment_coords(version: u32) -> &'static [u8] {
    ALIGNMENT_COORDS[(version - 1) as usize]
}

/// One alignment-pattern lattice slot's status.
///
/// # Deviation from the brief
/// The brief's literal interface is `found: Vec<Option<[f64; 2]>>`. Plain
/// `Option` conflates two different reasons a slot has no position: "this
/// lattice coordinate is one of the three finder corners, so no alignment
/// pattern is ever searched for here" vs. "the probe searched and found
/// nothing". Task 4's grid sampler needs exactly that distinction — a
/// finder-corner slot is not a detection failure and should never make the
/// sampler second-guess a perfectly good finder-anchored region, whereas a
/// real search miss is a signal worth falling back to prediction for.
/// Making the distinction a type-level fact means Task 4 matches on the
/// enum instead of re-deriving "is `(i, j)` a finder corner" from
/// `coords.len()` every time it reads `found`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AnchorSlot {
    /// This lattice node is one of the three finder-pattern corners —
    /// `(coords[0], coords[0])`, `(coords[0], coords[n-1])`, or
    /// `(coords[n-1], coords[0])` in `(row, col)` terms — where ISO
    /// 18004 places a finder pattern instead of an alignment pattern, so
    /// it is never searched. Its implicit anchor position, used only as a
    /// parallelogram-prediction input for interior slots, is the
    /// provisional transform's direct projection of the lattice
    /// coordinate (see `anchor_of`) — not a real detected position.
    FinderCorner,
    /// The concentric probe found and re-centered a real alignment
    /// pattern here, in image pixel space.
    Found([f64; 2]),
    /// A position was predicted and the concentric probe run around it,
    /// but no matching dark-light-dark cross-section was found within the
    /// probe window.
    Missing,
}

/// A version's alignment-pattern lattice and this candidate's search
/// results over it.
pub(crate) struct AlignmentGrid {
    /// This grid's `alignment_coords(version)` (owned copy — callers may
    /// want to keep an `AlignmentGrid` past the coordinate table's
    /// `'static` borrow's convenience, e.g. across a serialization
    /// boundary).
    pub coords: Vec<u8>,
    /// Row-major (raster-order) lattice, length `coords.len()^2`:
    /// `found[i * coords.len() + j]` is the slot for grid node
    /// `(row = coords[i], col = coords[j])`. Task 4 indexes this
    /// directly by `(i, j)` — this row-major convention, with `i` the row
    /// index and `j` the column index, is the indexing contract it must
    /// use.
    pub found: Vec<AnchorSlot>,
    /// Same row-major indexing as `found`: the position
    /// [`predict_position`] computed for this slot before the concentric
    /// probe ran (or, for a `FinderCorner` slot, `predict_position`'s
    /// naive provisional-transform projection of that slot's own lattice
    /// coordinate — never a real search result, but harmless: Task 6's
    /// trace consumer, [`AlignmentGrid::to_trace_entries`], skips
    /// finder-corner slots entirely). Plan 4 Task 6's debug-UI overlay
    /// input — recorded unconditionally (cheap: at most `7x7` entries)
    /// rather than gated behind a trace flag, matching
    /// `decode::DecodeAttemptTrace`'s own "always build it, let the
    /// caller decide whether to keep it" precedent.
    pub predicted: Vec<[f64; 2]>,
}

impl AlignmentGrid {
    /// This grid's search results as Task 6's flat, finder-corner-excluded
    /// trace entries — see [`AlignmentTraceEntry`]'s doc for why corners
    /// are skipped.
    pub(crate) fn to_trace_entries(&self) -> Vec<AlignmentTraceEntry> {
        let n = self.coords.len();
        let mut out = Vec::with_capacity((n * n).saturating_sub(3));
        for i in 0..n {
            for j in 0..n {
                if is_finder_corner(i, j, n) {
                    continue;
                }
                let found = match self.found[i * n + j] {
                    AnchorSlot::Found(p) => Some(p),
                    AnchorSlot::Missing => None,
                    AnchorSlot::FinderCorner => unreachable!(
                        "(i, j) = ({i}, {j}) is a FinderCorner slot but wasn't matched by \
                         is_finder_corner above — the two have gone out of sync"
                    ),
                };
                out.push(AlignmentTraceEntry { predicted: self.predicted[i * n + j], found });
            }
        }
        out
    }
}

/// `true` iff lattice node `(i, j)` (0-indexed into a `coords` of length
/// `n`) is one of the three finder-pattern corners.
///
/// `pub(crate)` (not private): Task 4's `sample.rs` needs this same
/// three-corner test to decide whether a region's tiling anchor is a real
/// alignment-pattern node or a finder corner, and re-deriving it there
/// would risk the two definitions drifting apart.
pub(crate) fn is_finder_corner(i: usize, j: usize, n: usize) -> bool {
    match (i, j) {
        (0, 0) => true,
        (0, col) if col == n - 1 => true,
        (row, 0) if row == n - 1 => true,
        _ => false,
    }
}

/// The best available anchor position for lattice node `(i, j)`, in image
/// pixel space: the probe's refined position if already `Found`, the
/// provisional transform's direct projection of the lattice coordinate if
/// it's a `FinderCorner`, or `None` if the search already ran there and
/// came up `Missing` (a `Missing` slot cannot anchor anything else — it
/// carries no position at all).
fn anchor_of(
    found: &[AnchorSlot],
    coords: &[u8],
    i: usize,
    j: usize,
    n: usize,
    dim: u32,
    provisional: &PerspectiveTransform,
) -> Option<[f64; 2]> {
    match found[i * n + j] {
        AnchorSlot::Found(p) => Some(p),
        AnchorSlot::FinderCorner => {
            let dimf = dim as f64;
            Some(provisional.map(
                (coords[j] as f64 + 0.5) / dimf,
                (coords[i] as f64 + 0.5) / dimf,
            ))
        }
        AnchorSlot::Missing => None,
    }
}

/// Predict lattice node `(i, j)`'s pixel position: the parallelogram rule
/// `AP(i-1,j) + AP(i,j-1) - AP(i-1,j-1)` when all three neighbors have an
/// anchor position (see [`anchor_of`]), else the provisional transform's
/// direct projection of this node's own lattice coordinate. Raster-order
/// iteration in [`locate_alignment_patterns`] guarantees `(i-1, j)`,
/// `(i, j-1)`, and `(i-1, j-1)` are already resolved (found, missing, or a
/// preset finder corner) by the time this runs for `(i, j)`.
fn predict_position(
    found: &[AnchorSlot],
    coords: &[u8],
    i: usize,
    j: usize,
    n: usize,
    dim: u32,
    provisional: &PerspectiveTransform,
) -> [f64; 2] {
    if i > 0 && j > 0 {
        let a = anchor_of(found, coords, i - 1, j, n, dim, provisional);
        let b = anchor_of(found, coords, i, j - 1, n, dim, provisional);
        let c = anchor_of(found, coords, i - 1, j - 1, n, dim, provisional);
        if let (Some(a), Some(b), Some(c)) = (a, b, c) {
            return [a[0] + b[0] - c[0], a[1] + b[1] - c[1]];
        }
    }
    let dimf = dim as f64;
    provisional.map(
        (coords[j] as f64 + 0.5) / dimf,
        (coords[i] as f64 + 0.5) / dimf,
    )
}

/// Concentric-probe search step, in modules. Fine enough that the search
/// reliably places at least one candidate offset inside the true center
/// module's exact continuous-space extent (a full module wide, since
/// nearest-neighbor rendering — see `testpaint::render_module_grid_transformed`
/// — gives every module a hard-edged, exactly-one-module-wide footprint
/// with no antialiasing to blur it), while keeping the ±2.25-module ×
/// ±2.25-module search cheap (`(2*2.25/0.25 + 1)^2 = 361` candidates).
const PROBE_STEP_MODULES: f64 = 0.25;

/// The five expected polarity samples (`true` = dark) crossing an
/// alignment pattern's center along either axis, at module offsets
/// `[-2, -1, 0, 1, 2]` from the center: dark border ring, light ring, dark
/// center, light ring, dark border ring (ISO 18004 §6.3.6's 5×5 pattern).
const EXPECTED_CROSS_SECTION: [bool; 5] = [true, false, true, false, true];
const CROSS_SECTION_OFFSETS: [f64; 5] = [-2.0, -1.0, 0.0, 1.0, 2.0];

/// Bundles the read-only inputs the concentric probe's per-candidate
/// checks need, so [`cross_section_matches`] takes one reference instead
/// of five positional arguments (also keeps `clippy::too_many_arguments`
/// happy).
struct ProbeContext<'a> {
    view: &'a LumaView<'a>,
    grid: &'a TileGrid,
    transform: &'a PerspectiveTransform,
    dim: u32,
    inverted: bool,
}

/// `true` iff the polarity samples along one axis through candidate center
/// `(cu, cv)` (module-space units, i.e. already multiplied by `dim` — see
/// [`recenter_alignment_pattern`]) exactly match [`EXPECTED_CROSS_SECTION`].
/// `horizontal`: vary the first (column) coordinate across the offsets and
/// hold the row fixed at `cv`; else vary the row and hold the column fixed
/// at `cu`. `None` from any single sample (mapped pixel outside the image)
/// counts as a non-match, not a panic or a silent skip — an incomplete
/// cross-section cannot confirm a pattern.
fn cross_section_matches(ctx: &ProbeContext, cu: f64, cv: f64, horizontal: bool) -> bool {
    for (k, &offset) in CROSS_SECTION_OFFSETS.iter().enumerate() {
        let (col, row) = if horizontal { (cu + offset, cv) } else { (cu, cv + offset) };
        match sample_module_ink(ctx.view, ctx.grid, ctx.transform, ctx.dim, col, row, ctx.inverted) {
            Some(ink) if ink == EXPECTED_CROSS_SECTION[k] => {}
            _ => return false,
        }
    }
    true
}

/// Re-center a predicted alignment-pattern position (`predicted`, image
/// pixel space) by searching a ±[`ALIGNMENT_PROBE_HALF_MODULES`]-module
/// window for the 5×5 dark-light-dark cross-section, checked along both
/// axes, at [`PROBE_STEP_MODULES`] resolution. Returns the centroid of
/// every candidate offset that matches both cross-sections, mapped back to
/// pixel space through `provisional` — or `None` if nothing in the window
/// matched.
///
/// The search itself works in `provisional`'s module-space coordinates
/// (`predicted` is converted back via `provisional.inverse()`, stepped,
/// then re-projected via `provisional.map` for each candidate sample) —
/// not in raw pixel deltas — so the step directions and scale follow
/// `provisional`'s local orientation/scale even when `predicted` itself
/// came from the parallelogram rule (a pixel-space sum that need not
/// correspond to any single clean projection under `provisional`). This is
/// an approximation (assumes `provisional`'s local Jacobian near
/// `predicted` is representative), not an exact per-point re-derivation of
/// the true local geometry — acceptable because the caller only needs
/// ≤2.25-module-scale search coverage, and the re-centering itself (not
/// this step's exact direction) is what corrects any resulting error.
fn recenter_alignment_pattern(
    view: &LumaView,
    grid: &TileGrid,
    provisional: &PerspectiveTransform,
    dim: u32,
    predicted: [f64; 2],
    inverted: bool,
) -> Option<[f64; 2]> {
    let dimf = dim as f64;
    let inverse = provisional.inverse();
    let [u0, v0] = inverse.map(predicted[0], predicted[1]);
    let (center_u, center_v) = (u0 * dimf, v0 * dimf);
    let ctx = ProbeContext { view, grid, transform: provisional, dim, inverted };

    let steps = (ALIGNMENT_PROBE_HALF_MODULES / PROBE_STEP_MODULES).round() as i32;
    let mut hit_sum = (0.0f64, 0.0f64);
    let mut hit_count = 0u32;
    for di in -steps..=steps {
        let dv = di as f64 * PROBE_STEP_MODULES;
        let cv = center_v + dv;
        for dj in -steps..=steps {
            let du = dj as f64 * PROBE_STEP_MODULES;
            let cu = center_u + du;
            if cross_section_matches(&ctx, cu, cv, true) && cross_section_matches(&ctx, cu, cv, false)
            {
                hit_sum.0 += du;
                hit_sum.1 += dv;
                hit_count += 1;
            }
        }
    }
    if hit_count == 0 {
        return None;
    }
    let n = hit_count as f64;
    let (final_u, final_v) = (center_u + hit_sum.0 / n, center_v + hit_sum.1 / n);
    Some(provisional.map(final_u / dimf, final_v / dimf))
}

/// Locate every alignment pattern for a `version`-sized candidate.
///
/// Iterates the `coords x coords` lattice in raster order (row `i` major,
/// column `j` minor), skipping the three finder corners entirely (their
/// slot is preset to [`AnchorSlot::FinderCorner`] before the loop starts).
/// For every other node: predict its position ([`predict_position`]) from
/// already-resolved neighbors or `provisional`, then re-center
/// ([`recenter_alignment_pattern`]) with the concentric probe, recording
/// [`AnchorSlot::Found`] or [`AnchorSlot::Missing`].
pub(crate) fn locate_alignment_patterns(
    view: &LumaView,
    grid: &TileGrid,
    provisional: &PerspectiveTransform,
    version: u32,
    inverted: bool,
) -> AlignmentGrid {
    let coords = alignment_coords(version).to_vec();
    let n = coords.len();
    let mut found = vec![AnchorSlot::Missing; n * n];
    let mut predicted = vec![[0.0, 0.0]; n * n];
    if n == 0 {
        // v1: no alignment patterns at all.
        return AlignmentGrid { coords, found, predicted };
    }
    for &(ci, cj) in &[(0usize, 0usize), (0, n - 1), (n - 1, 0)] {
        found[ci * n + cj] = AnchorSlot::FinderCorner;
    }

    let dim = 17 + 4 * version; // ISO 18004 dimension formula.
    for i in 0..n {
        for j in 0..n {
            // Computed for every slot, including finder corners, so
            // `predicted` stays a dense `n x n` lattice matching `found`'s
            // own indexing (see the field doc) — cheap (a handful of
            // `provisional.map` calls at worst) and it's the same value
            // `predict_position` would derive for a corner anyway, since
            // raster order has already resolved every dependency it needs
            // regardless of whether this particular slot gets searched.
            let p = predict_position(&found, &coords, i, j, n, dim, provisional);
            predicted[i * n + j] = p;
            if is_finder_corner(i, j, n) {
                continue;
            }
            found[i * n + j] = recenter_alignment_pattern(view, grid, provisional, dim, p, inverted)
                .map(AnchorSlot::Found)
                .unwrap_or(AnchorSlot::Missing);
        }
    }
    AlignmentGrid { coords, found, predicted }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testpaint::render_module_grid_transformed;

    // --- Annex E table spot checks ---

    #[test]
    fn alignment_coords_table_spot_checks() {
        assert_eq!(alignment_coords(1), &[] as &[u8]);
        assert_eq!(alignment_coords(2), &[6, 18]);
        assert_eq!(alignment_coords(7), &[6, 22, 38]);
        // NOTE (deviation from the task brief's literal test text): the
        // brief's v32 spot-check literal is `[6, 34, 60, 86, 112]` (5
        // entries). The actual ISO/zxing v32 row has 6 entries, ending
        // `..., 112, 138` — confirmed two independent ways: (1) zxing
        // `Version.java`'s `ALIGNMENT_PATTERN_POSITIONS[25]` (index
        // `32-7`), and (2) this crate's own `qrcode` dev-dependency's
        // identical table in `qrcode-0.14.1/src/canvas.rs` (`positions =
        // ALIGNMENT_PATTERN_POSITIONS[(a-7)]`, entry 25). Both list
        // `[6, 34, 60, 86, 112, 138]`. Using a 5-entry table here would
        // silently drop v32's real 6th alignment pattern (dropping a
        // fixture's decode is exactly the class of self-inflicted bug
        // this whole table exists to avoid), so the brief's literal is
        // treated as a transcription typo and the verified 6-entry value
        // is asserted instead — recorded here and in the task report.
        assert_eq!(alignment_coords(32), &[6, 34, 60, 86, 112, 138]);
        assert_eq!(alignment_coords(40), &[6, 30, 58, 86, 114, 142, 170]);
    }

    #[test]
    fn min_gap_between_adjacent_real_nodes_is_16_modules() {
        // Backs the ±2.25-module probe half-width's "never reaches a
        // neighboring alignment pattern" justification in `consts.rs`:
        // confirm by direct computation (not by eyeballing the table)
        // that no version's coordinate list has two consecutive entries
        // closer than 16 modules apart, for every version that actually
        // has at least two real (non-finder-corner) alignment patterns
        // per axis to compare (v7..40 — v2..6 have only one real pattern
        // total, so their single `6 -> coord` gap never separates two
        // real patterns and is excluded here).
        let mut min_gap = u8::MAX;
        for version in 7..=40u32 {
            let coords = alignment_coords(version);
            for w in coords.windows(2) {
                min_gap = min_gap.min(w[1] - w[0]);
            }
        }
        assert_eq!(min_gap, 16);
    }

    /// Independent, table-wide structural check (in the spirit of
    /// `version.rs`'s min-Hamming-distance self-check): by ISO 18004
    /// construction, every version's coordinate set starts at the
    /// timing-pattern-tied value 6 and ends exactly at `dim - 7` (the
    /// far finder pattern's own coordinate), where `dim = 17 + 4*version`.
    /// Checking this across all 39 non-empty entries (v2..40) catches
    /// most first/last-value transcription slips that a handful of spot
    /// checks could miss.
    #[test]
    fn every_versions_coords_start_at_6_and_end_at_dim_minus_7() {
        for version in 2..=40u32 {
            let coords = alignment_coords(version);
            let dim = 17 + 4 * version;
            assert_eq!(coords[0], 6, "v{version} first coord");
            assert_eq!(
                *coords.last().unwrap() as u32,
                dim - 7,
                "v{version} last coord"
            );
        }
    }

    // --- Synthetic-render tests ---

    /// Same axis-aligned transform helper `version.rs`'s tests use: maps a
    /// `dim x dim` code's own module square to a pixel quad inset by
    /// `quiet` modules inside an `img_side`-square canvas, at `scale`
    /// px/module.
    fn axis_aligned_transform(dim: usize, scale: f64, quiet: f64) -> (PerspectiveTransform, usize) {
        let img_side = ((dim as f64 + 2.0 * quiet) * scale).round() as usize;
        let x0 = quiet * scale;
        let x1 = x0 + dim as f64 * scale;
        let quad = [[x0, x0], [x1, x0], [x1, x1], [x0, x1]];
        (PerspectiveTransform::square_to_quad(quad).unwrap(), img_side)
    }

    /// Render a v7 `qrcode`-crate matrix through `transform` into an
    /// `img_side`-square luma buffer.
    fn render_v7(payload: &[u8], transform: &PerspectiveTransform, img_side: usize) -> (Vec<u8>, usize) {
        let code = qrcode::QrCode::with_version(
            payload, qrcode::Version::Normal(7), qrcode::EcLevel::M,
        )
        .unwrap();
        let dim = code.width();
        let img = render_module_grid_transformed(
            dim,
            |x, y| code[(x, y)] == qrcode::Color::Dark,
            25,
            235,
            transform,
            img_side,
            img_side,
        );
        (img, dim)
    }

    /// Analytic (exact) pixel position of lattice node `(i, j)` under
    /// `transform`, per the `coord + 0.5` module-center convention.
    fn analytic_position(
        transform: &PerspectiveTransform,
        coords: &[u8],
        i: usize,
        j: usize,
        dim: usize,
    ) -> [f64; 2] {
        let dimf = dim as f64;
        transform.map(
            (coords[j] as f64 + 0.5) / dimf,
            (coords[i] as f64 + 0.5) / dimf,
        )
    }

    #[test]
    fn v7_all_alignment_patterns_found_within_half_module_with_exact_transform() {
        let dim = 17 + 4 * 7usize;
        let scale = 4.0;
        let (transform, img_side) = axis_aligned_transform(dim, scale, 4.0);
        let (img, dim_rendered) = render_v7(b"ALIGN7", &transform, img_side);
        assert_eq!(dim_rendered, dim);
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let tile_grid = TileGrid::build(&view);

        let ag = locate_alignment_patterns(&view, &tile_grid, &transform, 7, false);
        let n = ag.coords.len();
        assert_eq!(n, 3);
        assert_eq!(n * n - 3, 6, "v7 has 3^2 - 3 = 6 alignment patterns");

        let half_module_px = 0.5 * scale;
        let mut checked = 0;
        for i in 0..n {
            for j in 0..n {
                if is_finder_corner(i, j, n) {
                    continue;
                }
                let expected = analytic_position(&transform, &ag.coords, i, j, dim);
                match ag.found[i * n + j] {
                    AnchorSlot::Found(p) => {
                        let d = ((p[0] - expected[0]).powi(2) + (p[1] - expected[1]).powi(2)).sqrt();
                        assert!(d < half_module_px, "node ({i},{j}) off by {d}px (limit {half_module_px}px)");
                        checked += 1;
                    }
                    other => panic!("node ({i},{j}) not found: {other:?}"),
                }
            }
        }
        assert_eq!(checked, 6);
    }

    #[test]
    fn recenters_correctly_despite_one_module_biased_provisional_transform() {
        let dim = 17 + 4 * 7usize;
        let scale = 4.0;
        let (true_transform, img_side) = axis_aligned_transform(dim, scale, 4.0);
        let (img, _) = render_v7(b"BIAS7", &true_transform, img_side);
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let tile_grid = TileGrid::build(&view);

        // Deliberately biased provisional transform: the same quad,
        // translated by one full module (`scale` px) in both axes. The
        // rendered image content is unaffected — only the caller's
        // geometric estimate is wrong, exactly the "prediction error"
        // scenario the probe's search window exists to absorb.
        let x0 = 4.0 * scale;
        let x1 = x0 + dim as f64 * scale;
        let true_quad = [[x0, x0], [x1, x0], [x1, x1], [x0, x1]];
        let bias = scale;
        let biased_quad = true_quad.map(|[x, y]| [x + bias, y + bias]);
        let biased_transform = PerspectiveTransform::square_to_quad(biased_quad).unwrap();

        let ag = locate_alignment_patterns(&view, &tile_grid, &biased_transform, 7, false);
        let n = ag.coords.len();
        let half_module_px = 0.5 * scale;
        let mut checked = 0;
        for i in 0..n {
            for j in 0..n {
                if is_finder_corner(i, j, n) {
                    continue;
                }
                // Ground truth is always measured against the TRUE
                // transform, regardless of what the biased provisional
                // transform predicted.
                let expected = analytic_position(&true_transform, &ag.coords, i, j, dim);
                match ag.found[i * n + j] {
                    AnchorSlot::Found(p) => {
                        let d = ((p[0] - expected[0]).powi(2) + (p[1] - expected[1]).powi(2)).sqrt();
                        assert!(
                            d < half_module_px,
                            "node ({i},{j}) off by {d}px (limit {half_module_px}px) despite bias correction"
                        );
                        checked += 1;
                    }
                    other => panic!("node ({i},{j}) not found despite bias correction: {other:?}"),
                }
            }
        }
        assert_eq!(checked, 6);
    }
}
