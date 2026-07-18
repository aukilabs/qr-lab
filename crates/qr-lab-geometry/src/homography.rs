/// 3x3 homography stored row-major as coefficients a11..a33.
/// Maps (u,v) -> ((a11*u + a21*v + a31)/w, (a12*u + a22*v + a32)/w),
/// w = a13*u + a23*v + a33.
#[derive(Clone, Copy, Debug)]
pub struct PerspectiveTransform {
    a11: f64,
    a21: f64,
    a31: f64,
    a12: f64,
    a22: f64,
    a32: f64,
    a13: f64,
    a23: f64,
    a33: f64,
}

impl PerspectiveTransform {
    /// Unit square (0,0),(1,0),(1,1),(0,1) -> quad [TL, TR, BR, BL].
    ///
    /// Returns `None` for a degenerate `q` that admits no valid
    /// homography: the affine branch (`q` a parallelogram) is degenerate
    /// iff its two edge vectors `p1-p0` and `p3-p0` are parallel (zero
    /// cross product — a zero-area parallelogram); the general
    /// projective branch is degenerate iff its `den` denominator is
    /// exactly zero. Both are honest "no solution" cases, not just
    /// numerically unstable ones — callers with real (non-collinear)
    /// image geometry will not hit them.
    pub fn square_to_quad(q: [[f64; 2]; 4]) -> Option<Self> {
        let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = q;
        let dx3 = x0 - x1 + x2 - x3;
        let dy3 = y0 - y1 + y2 - y3;
        if dx3 == 0.0 && dy3 == 0.0 {
            let cross = (x1 - x0) * (y3 - y0) - (y1 - y0) * (x3 - x0);
            if cross == 0.0 {
                return None;
            }
            Some(Self {
                a11: x1 - x0,
                a21: x2 - x1,
                a31: x0,
                a12: y1 - y0,
                a22: y2 - y1,
                a32: y0,
                a13: 0.0,
                a23: 0.0,
                a33: 1.0,
            })
        } else {
            let dx1 = x1 - x2;
            let dx2 = x3 - x2;
            let dy1 = y1 - y2;
            let dy2 = y3 - y2;
            let den = dx1 * dy2 - dx2 * dy1;
            if den == 0.0 {
                return None;
            }
            let a13 = (dx3 * dy2 - dx2 * dy3) / den;
            let a23 = (dx1 * dy3 - dx3 * dy1) / den;
            Some(Self {
                a11: x1 - x0 + a13 * x1,
                a21: x3 - x0 + a23 * x3,
                a31: x0,
                a12: y1 - y0 + a13 * y1,
                a22: y3 - y0 + a23 * y3,
                a32: y0,
                a13,
                a23,
                a33: 1.0,
            })
        }
    }

    /// Maps a unit-square point `(u, v)` to its image under this
    /// transform.
    ///
    /// # Behavior near the horizon
    /// The projective denominator `w = a13*u + a23*v + a33` can be zero,
    /// or merely close to zero, for points on or near this transform's
    /// "horizon line" — this is a property of a projective map, not a
    /// bug, and is deliberately left unguarded here: `square_to_quad`
    /// rejects `q`s that admit no valid transform at all (see its docs),
    /// but a valid transform can still place its horizon inside the unit
    /// square for a sufficiently extreme `q`. At `w == 0.0` this returns
    /// `±inf`/`NaN`; for `w` merely near zero it returns a finite but
    /// numerically unstable result. Every caller in this crate only ever
    /// evaluates `map` at points derived from real, in-frame image
    /// geometry, so this is not expected to be hit in practice — but it
    /// is not checked, so a future caller with unusual `(u, v)` inputs
    /// should be aware of it.
    #[inline]
    pub fn map(&self, u: f64, v: f64) -> [f64; 2] {
        let w = self.a13 * u + self.a23 * v + self.a33;
        [
            (self.a11 * u + self.a21 * v + self.a31) / w,
            (self.a12 * u + self.a22 * v + self.a32) / w,
        ]
    }

    /// Diagonal scaling homography: `(u, v) -> (sx*u, sy*v)`. Plan 5 Task
    /// 2's source-resolution sampling composes this with a module→working
    /// transform (via [`Self::then`]) to lift it to a module→SOURCE
    /// transform: `working.then(&PerspectiveTransform::scaled(1.0 / sx,
    /// 1.0 / sy))` — with `(sx, sy)` the PER-AXIS working/source ratios,
    /// which `scan`'s downscale rounds independently — maps a module
    /// coordinate straight to source-image pixels, since `source_px =
    /// working_px / s` per axis (see `sample::SourceView`'s doc for the
    /// convention, and `Detections::source_scale` for the width-pinned
    /// public scalar it deliberately differs from on the height axis). Not
    /// itself projective (`a13 == a23 == 0.0`), just the diagonal special
    /// case expressed in the same 3x3 form so [`Self::then`]'s matrix
    /// composition applies unchanged.
    pub fn scaled(sx: f64, sy: f64) -> Self {
        Self {
            a11: sx,
            a21: 0.0,
            a31: 0.0,
            a12: 0.0,
            a22: sy,
            a32: 0.0,
            a13: 0.0,
            a23: 0.0,
            a33: 1.0,
        }
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

    /// This transform's 3x3 matrix, row-major, acting on the homogeneous
    /// column vector `[u, v, 1]^T` to produce `[x*w, y*w, w]^T` (see the
    /// struct doc's `map` derivation). Used only by [`Self::then`]'s matrix
    /// composition.
    fn matrix(&self) -> [[f64; 3]; 3] {
        [
            [self.a11, self.a21, self.a31],
            [self.a12, self.a22, self.a32],
            [self.a13, self.a23, self.a33],
        ]
    }

    fn from_matrix(m: [[f64; 3]; 3]) -> Self {
        Self {
            a11: m[0][0],
            a21: m[0][1],
            a31: m[0][2],
            a12: m[1][0],
            a22: m[1][1],
            a32: m[1][2],
            a13: m[2][0],
            a23: m[2][1],
            a33: m[2][2],
        }
    }

    /// Compose two homographies: `self.then(outer)` applies `self` first,
    /// then `outer` — i.e. `self.then(outer).map(p) == outer.map(self.map(p))`
    /// (up to the shared projective scale ambiguity, which `map`'s `/w`
    /// normalization removes either way). Implemented as the 3x3 matrix
    /// product `outer.matrix() * self.matrix()`, since composing two
    /// projective maps is exactly multiplying their homogeneous matrices.
    ///
    /// `pub(crate)` (not private) so `sample.rs`'s source-resolution module
    /// sampling (Plan 5 Task 2) can lift a module→working transform to
    /// module→source by composing with [`Self::scaled`] — the same
    /// composition `quad_to_quad` already builds internally, just exposed
    /// for a second caller instead of duplicated.
    pub fn then(&self, outer: &Self) -> Self {
        let a = outer.matrix();
        let b = self.matrix();
        let mut r = [[0.0f64; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        Self::from_matrix(r)
    }

    /// General quadrilateral-to-quadrilateral homography: maps `src`'s own
    /// quad (in whatever coordinate space its 4 corners are given) onto
    /// `dst`'s quad, extending correctly (as a single projective map) to
    /// every other point — not just the 4 corners themselves.
    ///
    /// `= square_to_quad(dst) ∘ square_to_quad(src)⁻¹`: `square_to_quad(src)`
    /// already IS the unique homography taking the unit square's corners to
    /// `src`'s corners, so its inverse recovers, for any point expressed in
    /// `src`'s coordinate space, the `(u, v)` unit-square fraction that
    /// produces it; composing with `square_to_quad(dst)` re-expands that
    /// same `(u, v)` fraction against `dst`'s corners instead. A homography
    /// is uniquely determined by 4 (non-collinear-triple) point
    /// correspondences, so this composition — built from exactly the 4
    /// `src[i] -> dst[i]` correspondences — is THE homography satisfying
    /// them, not merely an approximation.
    ///
    /// Returns `None` when either `square_to_quad` call does (a degenerate
    /// `src` or `dst` admits no valid homography at all — see
    /// [`Self::square_to_quad`]'s docs).
    pub fn quad_to_quad(src: [[f64; 2]; 4], dst: [[f64; 2]; 4]) -> Option<Self> {
        let s = Self::square_to_quad(src)?;
        let d = Self::square_to_quad(dst)?;
        Some(s.inverse().then(&d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: [[f64; 2]; 4] = [[100.0, 50.0], [420.0, 80.0], [400.0, 380.0], [90.0, 350.0]];

    #[test]
    fn corners_map_exactly() {
        let h = PerspectiveTransform::square_to_quad(Q).unwrap();
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for (i, [u, v]) in uv.iter().enumerate() {
            let p = h.map(*u, *v);
            assert!((p[0] - Q[i][0]).abs() < 1e-9, "corner {i}: {p:?}");
            assert!((p[1] - Q[i][1]).abs() < 1e-9, "corner {i}: {p:?}");
        }
    }

    #[test]
    fn affine_case_scales() {
        let h =
            PerspectiveTransform::square_to_quad([[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]])
                .unwrap();
        let p = h.map(0.25, 0.75);
        assert!((p[0] - 0.5).abs() < 1e-12 && (p[1] - 1.5).abs() < 1e-12);
    }

    #[test]
    fn inverse_round_trips() {
        let h = PerspectiveTransform::square_to_quad(Q).unwrap();
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
        let h = PerspectiveTransform::square_to_quad(Q).unwrap();
        let a = h.map(0.0, 0.5);
        let b = h.map(0.5, 0.5);
        let c = h.map(1.0, 0.5);
        let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        assert!(cross.abs() < 1e-6, "cross={cross}");
    }

    #[test]
    fn collinear_quad_returns_none() {
        // Four collinear points: no quadrilateral, so no valid
        // homography (falls into the general projective branch with
        // den == 0.0).
        let q = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
        assert!(PerspectiveTransform::square_to_quad(q).is_none());
    }

    #[test]
    fn degenerate_parallelogram_returns_none() {
        // dx3 == 0 && dy3 == 0 (the affine/parallelogram branch), but
        // p1 == p3 == p0's antipode makes the "parallelogram" zero-area.
        let q = [[0.0, 0.0], [2.0, 0.0], [2.0, 0.0], [0.0, 0.0]];
        assert!(PerspectiveTransform::square_to_quad(q).is_none());
    }

    // --- quad_to_quad ---

    /// A genuinely projective (non-parallelogram) source quad and a
    /// genuinely projective destination quad — exercises the general
    /// branch of `square_to_quad` on both sides of the composition, not
    /// just the affine shortcut.
    const SRC_Q: [[f64; 2]; 4] = [[10.0, 10.0], [50.0, 12.0], [46.0, 54.0], [8.0, 50.0]];
    const DST_Q: [[f64; 2]; 4] = [[100.0, 20.0], [300.0, 15.0], [310.0, 220.0], [90.0, 210.0]];

    #[test]
    fn quad_to_quad_corners_map_exactly() {
        let h = PerspectiveTransform::quad_to_quad(SRC_Q, DST_Q).unwrap();
        for (i, &[x, y]) in SRC_Q.iter().enumerate() {
            let p = h.map(x, y);
            assert!((p[0] - DST_Q[i][0]).abs() < 1e-6, "corner {i}: {p:?}");
            assert!((p[1] - DST_Q[i][1]).abs() < 1e-6, "corner {i}: {p:?}");
        }
    }

    #[test]
    fn quad_to_quad_interior_round_trips() {
        let fwd = PerspectiveTransform::quad_to_quad(SRC_Q, DST_Q).unwrap();
        let back = PerspectiveTransform::quad_to_quad(DST_Q, SRC_Q).unwrap();
        // Interior points expressed in SRC_Q's own coordinate space (not
        // just its 4 corners): forward then backward must recover them.
        for &[x, y] in &[[25.0, 25.0], [30.0, 40.0], [15.0, 45.0], [40.0, 20.0]] {
            let p = fwd.map(x, y);
            let b = back.map(p[0], p[1]);
            assert!(
                (b[0] - x).abs() < 1e-6 && (b[1] - y).abs() < 1e-6,
                "({x},{y}) -> {p:?} -> {b:?}"
            );
        }
    }

    #[test]
    fn quad_to_quad_degenerate_src_or_dst_returns_none() {
        let collinear = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
        assert!(PerspectiveTransform::quad_to_quad(collinear, DST_Q).is_none());
        assert!(PerspectiveTransform::quad_to_quad(SRC_Q, collinear).is_none());
    }

    // --- scaled + then composition (Plan 5 Task 2) ---

    #[test]
    fn scaled_maps_a_known_point_exactly() {
        let s = PerspectiveTransform::scaled(2.0, 0.5);
        let p = s.map(3.0, 10.0);
        assert!(
            (p[0] - 6.0).abs() < 1e-12 && (p[1] - 5.0).abs() < 1e-12,
            "{p:?}"
        );
    }

    #[test]
    fn scaled_round_trips_with_inverse() {
        let s = PerspectiveTransform::scaled(4.0, 0.25);
        let inv = s.inverse();
        for &(x, y) in &[(1.0, 1.0), (3.5, -2.0), (0.0, 7.0)] {
            let p = s.map(x, y);
            let b = inv.map(p[0], p[1]);
            assert!(
                (b[0] - x).abs() < 1e-9 && (b[1] - y).abs() < 1e-9,
                "({x},{y}) -> {p:?} -> {b:?}"
            );
        }
    }

    /// The exact composition Task 2's source-resolution sampling relies on:
    /// a module→working transform, lifted to module→source by composing
    /// with `scaled(1/source_scale, 1/source_scale)`, must land on exactly
    /// `working_point / source_scale` — the `source_px = working_px /
    /// source_scale` convention `Detections::source_scale` documents.
    #[test]
    fn then_composes_a_transform_with_a_scale_to_lift_working_to_source() {
        let working = PerspectiveTransform::square_to_quad(Q).unwrap();
        let source_scale = 0.4; // working = source * 0.4 (a downscale)
        let to_source = PerspectiveTransform::scaled(1.0 / source_scale, 1.0 / source_scale);
        let lifted = working.then(&to_source);
        for &(u, v) in &[(0.0, 0.0), (1.0, 0.0), (0.3, 0.7), (1.0, 1.0)] {
            let working_pt = working.map(u, v);
            let got = lifted.map(u, v);
            let want = [working_pt[0] / source_scale, working_pt[1] / source_scale];
            assert!(
                (got[0] - want[0]).abs() < 1e-9 && (got[1] - want[1]).abs() < 1e-9,
                "({u},{v}): got {got:?}, want {want:?}"
            );
        }
    }
}
