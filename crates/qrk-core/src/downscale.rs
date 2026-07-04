//! Nearest-neighbor luma downscale — the Rust-owned port (Plan 5 Task 1) of
//! `debug-ui/src/scanner/downscale.ts`'s `downscaleRgba`: scale so the
//! longest side is at most `max_dim`, using `round(dim * max_dim /
//! max(w, h))` for both output dimensions (the same rounding the TS
//! production capture path used before this task), so `scan`'s working
//! resolution matches what the debug UI predicted pre-Plan-5 pixel-for-pixel.
//! The two implementations are pinned to each other by this module's unit
//! tests, which transcribe `downscale.test.ts`'s own vectors verbatim.
//!
//! `downscaleRgba` operated on interleaved RGBA and returned the *same*
//! buffer reference on a no-op passthrough; `downscale_luma` operates on a
//! single-channel [`LumaView`] and returns `None` on the equivalent no-op
//! case (`max_dim == 0` — cap disabled, TS's `maxDim <= 0` — or the source
//! already fits) so the caller can borrow `src` directly instead of
//! allocating an identical copy.

use crate::LumaView;

/// The `(dst_w, dst_h)` [`downscale_luma`] would produce for a `width` x
/// `height` source capped at `max_dim`, or `None` on the no-op case (see
/// [`downscale_luma`]'s doc comment). Pure dimension arithmetic — no pixel
/// data is touched — so callers that only need the WORKING-resolution
/// dimensions (e.g. `qrk-wasm`'s `scan_rgba` reporting `scan_width`/
/// `scan_height` in its response) don't pay for a redundant O(w·h) NN pass
/// just to learn them. `downscale_luma` itself calls this first.
pub fn downscaled_dims(width: usize, height: usize, max_dim: u32) -> Option<(usize, usize)> {
    let longest = width.max(height);
    if max_dim == 0 || longest <= max_dim as usize {
        return None;
    }
    // f64 throughout, matching the TS formula
    // (`Math.round((w * maxDim) / longest)`) exactly — both languages use
    // IEEE-754 double-precision arithmetic here, so the same inputs produce
    // bit-identical rounding decisions.
    let max_dim = max_dim as f64;
    let longest = longest as f64;
    let dst_w = ((width as f64 * max_dim) / longest).round().max(1.0) as usize;
    let dst_h = ((height as f64 * max_dim) / longest).round().max(1.0) as usize;
    Some((dst_w, dst_h))
}

/// Nearest-neighbor-downscale `src` to at most `max_dim` on its longest
/// side. Returns `None` — rather than an identity copy — when no resize is
/// needed: `max_dim == 0` (cap disabled) or `max(width, height) <= max_dim`
/// (already within budget); callers should borrow `src` directly in that
/// case (see `scan.rs`'s use of this function for the pattern).
///
/// Destination dimensions: `round(dim * max_dim / max(w, h))`, clamped to a
/// minimum of 1px (see [`downscaled_dims`]). Source indexing: for each
/// destination pixel `(x, y)`, the source pixel is `(floor(x * w / dst_w),
/// floor(y * h / dst_h))`, each axis independently clamped to the source's
/// last valid index — this is the EXACT production formula (ported
/// verbatim from `downscaleRgba`, minus the interleaved-channel copy, which
/// doesn't apply to a single-channel luma plane).
pub fn downscale_luma(src: &LumaView, max_dim: u32) -> Option<(Vec<u8>, usize, usize)> {
    let (w, h) = (src.width(), src.height());
    let (dst_w, dst_h) = downscaled_dims(w, h, max_dim)?;

    let mut out = vec![0u8; dst_w * dst_h];
    for y in 0..dst_h {
        let src_y = (y * h / dst_h).min(h - 1);
        let row = src.row(src_y);
        let dst_row_start = y * dst_w;
        for x in 0..dst_w {
            let src_x = (x * w / dst_w).min(w - 1);
            out[dst_row_start + x] = row[src_x];
        }
    }
    Some((out, dst_w, dst_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a flat luma buffer where pixel `i` (row-major) has value `i as
    /// u8` (wrapping past 255 — only used at sizes small enough that the
    /// test cares about specific indices, never actual magnitude), so tests
    /// can identify exactly which source pixel survived the resample —
    /// mirrors `downscale.test.ts`'s `markedRgba` helper.
    fn marked_luma(width: usize, height: usize) -> Vec<u8> {
        (0..width * height).map(|i| i as u8).collect()
    }

    fn view(data: &[u8], w: usize, h: usize) -> LumaView<'_> {
        LumaView::new(data, w, h, w).unwrap()
    }

    // Transcribed from downscale.test.ts: "downscales 4x2 to 2x1 with exact
    // nearest-neighbor source pixels".
    #[test]
    fn downscales_4x2_to_2x1_with_exact_nearest_neighbor_source_pixels() {
        let data = marked_luma(4, 2);
        let (out, w, h) = downscale_luma(&view(&data, 4, 2), 2).expect("downscale needed");

        assert_eq!((w, h), (2, 1));
        assert_eq!(out.len(), 2);

        // dstW=2, dstH=1 -> srcX = floor(x*4/2), srcY = floor(y*2/1)
        // x=0 -> srcX=0, x=1 -> srcX=2; y=0 -> srcY=0
        assert_eq!(out[0], data[0]);
        assert_eq!(out[1], data[2]);
    }

    // Transcribed from downscale.test.ts: "is an identity passthrough (same
    // buffer) when maxDim <= 0" — `u32` can't be negative, so only the
    // `max_dim == 0` half of that test applies to this Rust port.
    #[test]
    fn returns_none_when_max_dim_is_zero() {
        let data = marked_luma(5, 3);
        assert_eq!(downscale_luma(&view(&data, 5, 3), 0), None);
    }

    // Transcribed from downscale.test.ts: "is an identity passthrough (same
    // buffer) when max(w,h) <= maxDim".
    #[test]
    fn returns_none_when_already_within_budget() {
        let data = marked_luma(4, 2);
        assert_eq!(downscale_luma(&view(&data, 4, 2), 4), None); // exact fit
        assert_eq!(downscale_luma(&view(&data, 4, 2), 100), None); // larger cap
    }

    // Transcribed from downscale.test.ts: "matches the production rounding
    // formula round(dim * maxDim / max(w,h))".
    #[test]
    fn matches_the_production_rounding_formula() {
        // w=7,h=5,maxDim=3 -> longest=7: newW=round(7*3/7)=3, newH=round(5*3/7)=round(2.142857)=2
        let data = marked_luma(7, 5);
        let (_, w, h) = downscale_luma(&view(&data, 7, 5), 3).expect("downscale needed");
        assert_eq!((w, h), (3, 2));

        // Half-integer boundary: w=2,h=1,maxDim=1 -> longest=2:
        // newW=round(2*1/2)=round(1)=1, newH=round(1*1/2)=round(0.5)=1 (round-half-up)
        let data2 = marked_luma(2, 1);
        let (_, w2, h2) = downscale_luma(&view(&data2, 2, 1), 1).expect("downscale needed");
        assert_eq!((w2, h2), (1, 1));
    }

    /// Cross-check test (Plan 5 Task 1 binding scope): a small asymmetric
    /// case with the expected output bytes embedded directly, computed by
    /// hand from the same formula as the tests above rather than derived
    /// from the implementation under test — an independent pin, not a
    /// tautology.
    ///
    /// Source: 5x3, values row-major `0..15` (`marked_luma`). max_dim=3 ->
    /// longest=5: dst_w=round(5*3/5)=3, dst_h=round(3*3/5)=round(1.8)=2.
    /// For each dst pixel, src_x=floor(x*5/3), src_y=floor(y*3/2):
    ///   y=0 -> src_y=floor(0*3/2)=0; y=1 -> src_y=floor(1*3/2)=floor(1.5)=1
    ///   x=0 -> src_x=floor(0*5/3)=0; x=1 -> src_x=floor(1*5/3)=1; x=2 -> src_x=floor(2*5/3)=3
    /// Row 0 (src_y=0): src pixels [0,1,3] -> values [0,1,3]
    /// Row 1 (src_y=1): src row starts at 1*5=5 -> src pixels [5,6,8] -> values [5,6,8]
    #[test]
    fn cross_check_small_asymmetric_case_matches_embedded_bytes() {
        let data = marked_luma(5, 3);
        let (out, w, h) = downscale_luma(&view(&data, 5, 3), 3).expect("downscale needed");
        assert_eq!((w, h), (3, 2));
        assert_eq!(out, vec![0, 1, 3, 5, 6, 8]);
    }

    #[test]
    fn downscaled_dims_agrees_with_downscale_luma_and_is_none_on_the_same_no_op_cases() {
        assert_eq!(downscaled_dims(7, 5, 3), Some((3, 2)));
        assert_eq!(downscaled_dims(4, 2, 0), None);
        assert_eq!(downscaled_dims(4, 2, 4), None);
        assert_eq!(downscaled_dims(4, 2, 100), None);
    }

    #[test]
    fn min_1px_clamp_on_an_extreme_aspect_ratio() {
        // A 100x1 source at max_dim=1: longest=100, dst_w=round(100*1/100)=1,
        // dst_h=round(1*1/100)=round(0.01)=0 before the min-1 clamp.
        let data = marked_luma(100, 1);
        let (out, w, h) = downscale_luma(&view(&data, 100, 1), 1).expect("downscale needed");
        assert_eq!((w, h), (1, 1));
        assert_eq!(out.len(), 1);
    }
}
