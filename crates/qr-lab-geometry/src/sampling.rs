use qr_lab_image::Gray8View;

/// Behavior when a sample falls outside the integer-center pixel domain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderMode {
    /// Reject samples outside `[0, width - 1] x [0, height - 1]`.
    #[default]
    Reject,
    /// Clamp the continuous coordinate to the nearest image edge.
    Clamp,
    /// Return a fixed value for an out-of-domain coordinate.
    Constant(u8),
}

/// Bilinearly sample an 8-bit grayscale image at an integer-center coordinate.
#[inline]
pub fn sample_bilinear(view: Gray8View<'_>, x: f64, y: f64, border: BorderMode) -> Option<f64> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let max_x = (view.width() - 1) as f64;
    let max_y = (view.height() - 1) as f64;
    let outside = x < 0.0 || y < 0.0 || x > max_x || y > max_y;
    let (x, y) = if outside {
        match border {
            BorderMode::Reject => return None,
            BorderMode::Clamp => (x.clamp(0.0, max_x), y.clamp(0.0, max_y)),
            BorderMode::Constant(value) => return Some(value as f64),
        }
    } else {
        (x, y)
    };

    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(view.width() - 1);
    let y1 = (y0 + 1).min(view.height() - 1);
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let v00 = view.get(x0, y0) as f64;
    let v10 = view.get(x1, y0) as f64;
    let v01 = view.get(x0, y1) as f64;
    let v11 = view.get(x1, y1) as f64;
    let top = v00 + (v10 - v00) * fx;
    let bottom = v01 + (v11 - v01) * fx;
    Some(top + (bottom - top) * fy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(data: &[u8]) -> Gray8View<'_> {
        Gray8View::new(data, 2, 2, 2).unwrap()
    }

    #[test]
    fn integer_and_fractional_samples_follow_pixel_center_convention() {
        let data = [0, 10, 20, 30];
        assert_eq!(
            sample_bilinear(view(&data), 1.0, 1.0, BorderMode::Reject),
            Some(30.0)
        );
        assert_eq!(
            sample_bilinear(view(&data), 0.5, 0.5, BorderMode::Reject),
            Some(15.0)
        );
    }

    #[test]
    fn border_modes_are_explicit() {
        let data = [0, 10, 20, 30];
        assert_eq!(
            sample_bilinear(view(&data), -0.1, 0.0, BorderMode::Reject),
            None
        );
        assert_eq!(
            sample_bilinear(view(&data), -0.1, 0.0, BorderMode::Clamp),
            Some(0.0)
        );
        assert_eq!(
            sample_bilinear(view(&data), -0.1, 0.0, BorderMode::Constant(7)),
            Some(7.0)
        );
    }
}
