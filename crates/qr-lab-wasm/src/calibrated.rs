//! Reusable calibrated scanner for browser camera simulations and clients.
use qr_lab::image::Size;
use qr_lab::{
    luma_from_rgba, CalibratedDetections, CalibratedScanner, CameraCalibration, CameraIntrinsics,
    LensDistortion, LumaView, ScanOptions,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Intrinsics {
    fx: f64,
    fy: f64,
    cx: f64,
    cy: f64,
}
impl From<Intrinsics> for CameraIntrinsics {
    fn from(k: Intrinsics) -> Self {
        Self {
            fx: k.fx,
            fy: k.fy,
            cx: k.cx,
            cy: k.cy,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "model", content = "coefficients", rename_all = "snake_case")]
enum Distortion {
    None,
    BrownConrady([f64; 5]),
    Fisheye([f64; 4]),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Calibration {
    intrinsics: Intrinsics,
    distortion: Distortion,
    rectified_intrinsics: Option<Intrinsics>,
}
impl Calibration {
    fn build(self, size: Size) -> Result<CameraCalibration, String> {
        let distortion = match self.distortion {
            Distortion::None => LensDistortion::None,
            Distortion::BrownConrady(k) => LensDistortion::BrownConrady(k),
            Distortion::Fisheye(k) => LensDistortion::Fisheye(k),
        };
        let camera = CameraCalibration::new(size, self.intrinsics.into(), distortion)
            .map_err(|e| format!("{e:?}"))?;
        match self.rectified_intrinsics {
            Some(k) => camera
                .with_rectified_intrinsics(k.into())
                .map_err(|e| format!("{e:?}")),
            None => Ok(camera),
        }
    }
}

/// All geometry is in original sensor pixels. No rectified diagnostic trace.
#[derive(Serialize)]
struct CalibratedResult {
    detections: CalibratedDetections,
    scan_width: u32,
    scan_height: u32,
}

/// Calibration/options are fixed at construction; the rectification map is reused.
/// JS must call `free()` when replacing a scanner. Calibration uses OpenCV
/// coefficient order and pixel centers at integers; omitted rectified_intrinsics
/// defaults to intrinsics. The scan result's corners remain in sensor pixels.
#[wasm_bindgen]
pub struct WasmCalibratedScanner {
    scanner: CalibratedScanner,
    rgba_len: usize,
}

fn error(message: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&format!("WasmCalibratedScanner: {message}"))
}

#[wasm_bindgen]
impl WasmCalibratedScanner {
    #[wasm_bindgen(constructor)]
    pub fn new(
        width: u32,
        height: u32,
        calibration: JsValue,
        max_dim: u32,
        refine: bool,
    ) -> Result<Self, JsValue> {
        let size = Size::new(width as usize, height as usize);
        let rgba_len = size
            .width
            .checked_mul(size.height)
            .and_then(|n| n.checked_mul(4))
            .filter(|&n| n > 0)
            .ok_or_else(|| error("invalid image dimensions"))?;
        let calibration: Calibration =
            serde_wasm_bindgen::from_value(calibration).map_err(error)?;
        let camera = calibration.build(size).map_err(error)?;
        let scanner = CalibratedScanner::new(
            camera,
            ScanOptions {
                max_working_dim: max_dim,
                refine,
            },
        )
        .map_err(|e| error(format!("{e:?}")))?;
        Ok(Self { scanner, rgba_len })
    }

    pub fn scan_rgba(&mut self, rgba: &[u8]) -> Result<JsValue, JsValue> {
        if rgba.len() != self.rgba_len {
            return Err(error(format!(
                "expected {} RGBA bytes, got {}",
                self.rgba_len,
                rgba.len()
            )));
        }
        let size = self.scanner.calibration().image_size();
        let luma = luma_from_rgba(rgba, size.width, size.height);
        let view = LumaView::new(&luma, size.width, size.height, size.width)
            .map_err(|e| error(format!("{e:?}")))?;
        let detections = self
            .scanner
            .scan(&view)
            .map_err(|e| error(format!("{e:?}")))?;
        serde_wasm_bindgen::to_value(&CalibratedResult {
            detections,
            scan_width: size.width as u32,
            scan_height: size.height as u32,
        })
        .map_err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_contract() {
        let json = r#"{"intrinsics":{"fx":500,"fy":510,"cx":319.5,"cy":239.5},"distortion":{"model":"fisheye","coefficients":[-0.25,0,0,0]}}"#;
        let camera = serde_json::from_str::<Calibration>(json)
            .unwrap()
            .build(Size::new(640, 480))
            .unwrap();
        assert_eq!(
            camera.distortion(),
            LensDistortion::Fisheye([-0.25, 0.0, 0.0, 0.0])
        );
        assert_eq!(camera.intrinsics(), camera.rectified_intrinsics());
        assert!(serde_json::from_str::<Calibration>(&json.replace("-0.25,0,0,0", "0,0")).is_err());
        assert!(
            serde_json::from_str::<Calibration>(&json.replace("500", "0"))
                .unwrap()
                .build(Size::new(640, 480))
                .is_err()
        );
    }
}
