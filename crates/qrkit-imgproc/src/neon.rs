//! aarch64 NEON hot paths for tile min/max and per-row binarization.
//!
//! All public helpers are **bit-identical** to the obvious scalar loops
//! (u8 min/max and `<` compare have no intermediate rounding). Unsafe is
//! confined here; callers stay safe. On non-aarch64 the crate never
//! compiles this module (`cfg(target_arch = "aarch64")` in `lib.rs`).

use std::arch::aarch64::*;

/// Min and max of exactly 16 bytes. Bit-identical to a scalar fold.
#[inline]
pub fn min_max_u8x16(s: &[u8; 16]) -> (u8, u8) {
    // SAFETY: `s` is a 16-byte reference; `vld1q_u8` loads exactly 16 bytes.
    // `vminvq_u8` / `vmaxvq_u8` reduce a u8x16 lane-wise — same as iterated
    // min/max over the same 16 values.
    unsafe {
        let v = vld1q_u8(s.as_ptr());
        (vminvq_u8(v), vmaxvq_u8(v))
    }
}

/// Compare 16 pixels against a scalar threshold: `out[i] = pixels[i] < thr`.
/// Bit-identical to scalar `<`.
#[inline]
pub fn dark_mask_u8x16(pixels: &[u8; 16], thr: u8) -> [bool; 16] {
    // SAFETY: load 16 bytes; `vcltq_u8` is unsigned lane-wise less-than;
    // store mask as 0xFF/0x00 per lane then convert to bool.
    unsafe {
        let v = vld1q_u8(pixels.as_ptr());
        let t = vdupq_n_u8(thr);
        let m = vcltq_u8(v, t); // 0xFF where pixel < thr, else 0x00
        let mut bytes = [0u8; 16];
        vst1q_u8(bytes.as_mut_ptr(), m);
        let mut out = [false; 16];
        for i in 0..16 {
            out[i] = bytes[i] != 0;
        }
        out
    }
}

/// Sum of 16 u8 values as u32 (for Sauvola tile sums). Exact.
#[inline]
pub fn sum_u8x16(s: &[u8; 16]) -> u32 {
    // SAFETY: pairwise widening add reduction — exact integer sum.
    unsafe {
        let v = vld1q_u8(s.as_ptr());
        // vaddlvq_u8: sum of all lanes as u16, then widen — actually
        // vaddlvq_u8 returns u16 sum of 16 u8s (max 16*255=4080 fits u16).
        vaddlvq_u8(v) as u32
    }
}

/// Sum of squares of 16 u8 values as u64 (for Sauvola). Exact.
#[inline]
pub fn sumsq_u8x16(s: &[u8; 16]) -> u64 {
    // SAFETY: widen to u16, multiply, accumulate — exact.
    unsafe {
        let v = vld1q_u8(s.as_ptr());
        let lo = vget_low_u8(v);
        let hi = vget_high_u8(v);
        let lo16 = vmovl_u8(lo);
        let hi16 = vmovl_u8(hi);
        // mul of u16 lanes → u32 via vmull
        let lo32 = vmull_u16(vget_low_u16(lo16), vget_low_u16(lo16));
        let lo32b = vmull_u16(vget_high_u16(lo16), vget_high_u16(lo16));
        let hi32 = vmull_u16(vget_low_u16(hi16), vget_low_u16(hi16));
        let hi32b = vmull_u16(vget_high_u16(hi16), vget_high_u16(hi16));
        let s1 = vaddvq_u32(lo32) as u64;
        let s2 = vaddvq_u32(lo32b) as u64;
        let s3 = vaddvq_u32(hi32) as u64;
        let s4 = vaddvq_u32(hi32b) as u64;
        s1 + s2 + s3 + s4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_max_matches_scalar() {
        let mut buf = [0u8; 16];
        for seed in 0u32..200 {
            for (i, value) in buf.iter_mut().enumerate() {
                *value = ((seed.wrapping_mul(1103515245).wrapping_add(i as u32 * 17)) % 256) as u8;
            }
            let (lo, hi) = min_max_u8x16(&buf);
            let mut slo = 255u8;
            let mut shi = 0u8;
            for &p in &buf {
                slo = slo.min(p);
                shi = shi.max(p);
            }
            assert_eq!((lo, hi), (slo, shi), "buf={buf:?}");
        }
    }

    #[test]
    fn dark_mask_matches_scalar() {
        let pix = [
            0u8, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 255,
        ];
        for thr in [0u8, 1, 50, 100, 128, 200, 255] {
            let m = dark_mask_u8x16(&pix, thr);
            for i in 0..16 {
                assert_eq!(m[i], pix[i] < thr, "thr={thr} i={i}");
            }
        }
    }

    #[test]
    fn sum_and_sumsq_match_scalar() {
        let pix = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 255];
        let s: u32 = pix.iter().map(|&p| p as u32).sum();
        let sq: u64 = pix.iter().map(|&p| (p as u64) * (p as u64)).sum();
        assert_eq!(sum_u8x16(&pix), s);
        assert_eq!(sumsq_u8x16(&pix), sq);
    }
}
