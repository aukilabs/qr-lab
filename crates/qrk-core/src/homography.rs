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

#[cfg(test)]
mod tests {
    use super::*;

    const Q: [[f64; 2]; 4] =
        [[100.0, 50.0], [420.0, 80.0], [400.0, 380.0], [90.0, 350.0]];

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
        let h = PerspectiveTransform::square_to_quad(
            [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]).unwrap();
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
}
