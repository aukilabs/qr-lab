use std::error::Error;
use std::fmt;

/// A point in a continuous two-dimensional coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

impl Point2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub const fn to_array(self) -> [f64; 2] {
        [self.x, self.y]
    }
}

impl From<[f64; 2]> for Point2 {
    fn from([x, y]: [f64; 2]) -> Self {
        Self::new(x, y)
    }
}

/// A total-least-squares line represented by its centroid and unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line2 {
    pub centroid: [f64; 2],
    pub dir: [f64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitLineError {
    TooFewPoints,
    LengthMismatch,
    NonFiniteInput,
    NonPositiveWeightSum,
    Degenerate,
}

impl fmt::Display for FitLineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TooFewPoints => "at least two points are required",
            Self::LengthMismatch => "points and weights must have equal lengths",
            Self::NonFiniteInput => "points and weights must be finite",
            Self::NonPositiveWeightSum => "weights must have a positive sum",
            Self::Degenerate => "points do not define a stable line",
        };
        f.write_str(message)
    }
}

impl Error for FitLineError {}

pub fn fit_line_tls(points: &[[f64; 2]]) -> Result<Line2, FitLineError> {
    let weights = vec![1.0; points.len()];
    fit_line_tls_weighted(points, &weights)
}

pub fn fit_line_tls_weighted(points: &[[f64; 2]], weights: &[f64]) -> Result<Line2, FitLineError> {
    if points.len() < 2 {
        return Err(FitLineError::TooFewPoints);
    }
    if points.len() != weights.len() {
        return Err(FitLineError::LengthMismatch);
    }
    if points
        .iter()
        .flatten()
        .chain(weights.iter())
        .any(|value| !value.is_finite())
    {
        return Err(FitLineError::NonFiniteInput);
    }
    let weight_sum: f64 = weights.iter().sum();
    if weight_sum <= 0.0 {
        return Err(FitLineError::NonPositiveWeightSum);
    }
    let cx = points
        .iter()
        .zip(weights)
        .map(|(point, weight)| point[0] * weight)
        .sum::<f64>()
        / weight_sum;
    let cy = points
        .iter()
        .zip(weights)
        .map(|(point, weight)| point[1] * weight)
        .sum::<f64>()
        / weight_sum;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for (point, weight) in points.iter().zip(weights) {
        if *weight < 0.0 {
            return Err(FitLineError::NonPositiveWeightSum);
        }
        let dx = point[0] - cx;
        let dy = point[1] - cy;
        sxx += weight * dx * dx;
        sxy += weight * dx * dy;
        syy += weight * dy * dy;
    }
    if sxx + syy <= f64::EPSILON {
        return Err(FitLineError::Degenerate);
    }
    let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    Ok(Line2 {
        centroid: [cx, cy],
        dir: [theta.cos(), theta.sin()],
    })
}

pub fn intersect_lines(a: &Line2, b: &Line2) -> Option<[f64; 2]> {
    let cross = a.dir[0] * b.dir[1] - a.dir[1] * b.dir[0];
    if !cross.is_finite() || cross.abs() < 1e-9 {
        return None;
    }
    let dx = b.centroid[0] - a.centroid[0];
    let dy = b.centroid[1] - a.centroid[1];
    let distance = (dx * b.dir[1] - dy * b.dir[0]) / cross;
    Some([
        a.centroid[0] + distance * a.dir[0],
        a.centroid[1] + distance * a.dir[1],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_fit_favors_high_confidence_points() {
        let points = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [1.0, 5.0]];
        let fit = fit_line_tls_weighted(&points, &[10.0, 10.0, 10.0, 0.01]).unwrap();
        assert!(fit.dir[1].abs() < 0.02, "{fit:?}");
    }

    #[test]
    fn intersection_rejects_parallel_lines() {
        let a = Line2 {
            centroid: [0.0, 0.0],
            dir: [1.0, 0.0],
        };
        let b = Line2 {
            centroid: [0.0, 1.0],
            dir: [1.0, 0.0],
        };
        assert_eq!(intersect_lines(&a, &b), None);
    }
}
