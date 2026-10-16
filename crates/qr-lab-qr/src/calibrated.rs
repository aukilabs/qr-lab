//! Calibrated counterpart of `scan`: rectified detection, original-pixel sampling.

use crate::sample::SourceView;
use crate::scanner::detect_with_source;
use crate::{downscaled_dims, BinarizeSpec, LumaView, ScanOptions, StageClock, StageTimings};
use qr_lab_geometry::{CameraCalibration, Quad};
use qr_lab_image::{Gray8Image, Size};
use qr_lab_imgproc::{undistort::UndistortMap, ImgProcError, ImgProcResult};

/// Decoded QR and its corners in the original, distorted input image.
///
/// Corners are the module-region boundary, ordered TL/TR/BR/BL in QR space.
/// A lens-distorted edge between two corners can be curved.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CalibratedCode {
    /// Decoded content as UTF-8 text (same convention as `DecodedCode`).
    pub payload: String,
    /// Decoded content as raw bytes.
    pub payload_bytes: Vec<u8>,
    /// QR version, 1 through 40.
    pub version: u32,
    /// Error-correction level: L, M, Q, H, or ?.
    pub ecc: char,
    /// Whether decoding required mirrored interpretation.
    pub mirrored: bool,
    /// Number of modules per side.
    pub dimension: u32,
    /// Whether the QR has light modules on a dark background.
    pub inverted: bool,
    /// Coarse corners in original input pixel coordinates, TL/TR/BR/BL.
    pub corners_source: Quad,
    /// Refined corners in original input pixels; absent if disabled or unsuccessful.
    pub refined_corners_source: Option<Quad>,
    /// Which refined corners came from fitted edges; others retain coarse positions.
    pub corner_refined: [bool; 4],
}

/// Result of a calibrated single-frame scan. All exposed corners are source pixels.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CalibratedDetections {
    /// Successfully decoded QR codes.
    pub codes: Vec<CalibratedCode>,
    /// Detection, decoding, and refinement timings (excluding rectification).
    pub timings: StageTimings,
    /// Time spent remapping this frame to the rectified working image.
    pub rectification_ns: u64,
    /// Complete call time. Includes map construction for `scan_calibrated`,
    /// excludes it for `CalibratedScanner::scan` (the map is already cached).
    pub total_ns: u64,
}

/// Reusable calibrated single-frame scanner, sharing the [`crate::scan`] pipeline.
///
/// Finder detection, timing/version reads, and alignment searches use a corrected
/// working image. Module samples and subpixel edge profiles use the original
/// image through `perspective → rectified pixels → lens projection`. Refinement
/// fits straight lines in rectified space and maps their intersections back.
///
/// This is the calibrated counterpart of `scan`, not the robust escalation or
/// temporal API. Calibration and options are fixed at construction; construct a
/// new scanner when either changes. The pixel map and working buffer are reused.
pub struct CalibratedScanner {
    calibration: CameraCalibration,
    options: ScanOptions,
    map: UndistortMap,
    working: Gray8Image,
}

impl CalibratedScanner {
    /// Build the cached rectification map. Invalid output dimensions return an error.
    pub fn new(calibration: CameraCalibration, options: ScanOptions) -> ImgProcResult<Self> {
        let size = calibration.image_size();
        let (w, h) = downscaled_dims(size.width, size.height, options.max_working_dim)
            .unwrap_or((size.width, size.height));
        let working_size = Size::new(w, h);
        let map = UndistortMap::new(&calibration, working_size)?;
        let working = Gray8Image::new(working_size)?;
        Ok(Self {
            calibration,
            options,
            map,
            working,
        })
    }

    /// Calibration used by this scanner.
    pub const fn calibration(&self) -> &CameraCalibration {
        &self.calibration
    }

    /// Rectified working-image dimensions (after applying the working cap).
    pub fn working_size(&self) -> Size {
        self.working.size()
    }

    /// Scan a grayscale frame (including padded camera Y planes).
    ///
    /// Returns `DimensionMismatch` if the frame does not match the calibration.
    /// A decoded corner with an undefined lens projection returns
    /// `InvalidConfiguration`, never a NaN corner. Samples outside the camera
    /// buffer are counted as missing when decoding/refining. The working image
    /// uses a white border for its missing samples.
    pub fn scan(&mut self, source: &LumaView<'_>) -> ImgProcResult<CalibratedDetections> {
        let clock = StageClock::start();
        let rectification_clock = StageClock::start();
        self.map.remap_into(*source, self.working.view_mut(), 255)?;
        let rectification_ns = rectification_clock.elapsed_ns();
        let working = self.working.view();
        let sx = working.width() as f64 / source.width() as f64;
        let sy = working.height() as f64 / source.height() as f64;
        let detected = detect_with_source(
            &working,
            Some(SourceView {
                view: source,
                sx,
                sy,
                camera: Some(&self.calibration),
            }),
            None,
            self.options.refine,
            BinarizeSpec::default(),
        );
        let project_quad = |quad: Quad| -> ImgProcResult<Quad> {
            let mut out = [[0.0; 2]; 4];
            for (dst, p) in out.iter_mut().zip(quad) {
                *dst = self
                    .calibration
                    .rectified_to_source(p)
                    .ok_or(ImgProcError::InvalidConfiguration)?;
            }
            Ok(out)
        };
        let codes = detected
            .codes
            .into_iter()
            .map(|code| {
                Ok(CalibratedCode {
                    corners_source: project_quad(code.corners.map(|[x, y]| [x / sx, y / sy]))?,
                    refined_corners_source: code.refined_corners.map(project_quad).transpose()?,
                    payload: code.payload,
                    payload_bytes: code.payload_bytes,
                    version: code.version,
                    ecc: code.ecc,
                    mirrored: code.mirrored,
                    dimension: code.dimension,
                    inverted: code.inverted,
                    corner_refined: code.corner_refined,
                })
            })
            .collect::<ImgProcResult<Vec<_>>>()?;
        Ok(CalibratedDetections {
            codes,
            timings: detected.timings,
            rectification_ns,
            total_ns: clock.elapsed_ns(),
        })
    }
}

/// Scan one image with known intrinsics and lens distortion.
///
/// Allocates a rectification map for this call. Prefer [`CalibratedScanner`]
/// for repeated frames. The working cap and refinement options match [`crate::scan`];
/// the returned corners always refer to the original input image.
///
/// ```
/// use qr_lab_qr::{scan_calibrated, CameraCalibration, CameraIntrinsics,
///     LensDistortion, LumaView, ScanOptions};
/// use qr_lab_image::Size;
/// let calibration = CameraCalibration::new(Size::new(640, 480),
///     CameraIntrinsics { fx: 350.0, fy: 350.0, cx: 319.5, cy: 239.5 },
///     LensDistortion::Fisheye([0.01, -0.002, 0.0, 0.0]))?;
/// let pixels = vec![255; 640 * 480];
/// let frame = LumaView::new(&pixels, 640, 480, 640)?;
/// let result = scan_calibrated(&frame, &ScanOptions {
///     max_working_dim: 640, refine: true,
/// }, &calibration)?;
/// for code in result.codes {
///     println!("{} {:?}", code.payload, code.corners_source);
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn scan_calibrated(
    source: &LumaView<'_>,
    options: &ScanOptions,
    calibration: &CameraCalibration,
) -> ImgProcResult<CalibratedDetections> {
    if source.size() != calibration.image_size() {
        return Err(ImgProcError::DimensionMismatch);
    }
    let clock = StageClock::start();
    let mut result = CalibratedScanner::new(*calibration, *options)?.scan(source)?;
    result.total_ns = clock.elapsed_ns();
    Ok(result)
}
