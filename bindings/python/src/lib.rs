//! Native Python/NumPy bindings for QRKit.

use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyArray2, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pythonize::pythonize;
use qrkit::image::{Gray8Image, Gray8View, Size};
use qrkit::imgproc::blur::{
    estimate_line_direction, estimate_line_length_with, LineLengthConfig, RasterDirection4,
};
use qrkit::imgproc::deblur::{
    van_cittert_line_into, DeblurWorkspace, LineBorderMode, VanCittertConfig,
};
use qrkit::imgproc::illumination::{
    background_divide_into, BackgroundDivideConfig, IlluminationWorkspace,
};
use qrkit::{ScanConfig, ScanOptions, Scanner as CoreScanner, ScannerConfig, SessionConfig};
use serde::Serialize;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize)]
struct BlurEstimate {
    theta_radians: f64,
    confidence: f64,
    raster_direction: &'static str,
    blur_length: Option<f64>,
}

fn copy_gray8(image: PyReadonlyArray2<'_, u8>) -> PyResult<(Vec<u8>, usize, usize)> {
    let shape = image.shape();
    let (height, width) = (shape[0], shape[1]);
    if width == 0 || height == 0 {
        return Err(PyValueError::new_err(
            "image must have non-zero width and height",
        ));
    }
    let pixels = image.as_array().iter().copied().collect();
    Ok((pixels, width, height))
}

fn image_to_numpy<'py>(
    py: Python<'py>,
    pixels: Vec<u8>,
    width: usize,
    height: usize,
) -> PyResult<Bound<'py, PyArray2<u8>>> {
    let array = Array2::from_shape_vec((height, width), pixels)
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    Ok(array.into_pyarray(py))
}

fn runtime_error(error: impl ToString) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

fn preset_config(preset: &str) -> PyResult<ScanConfig> {
    match preset {
        "baseline" => Ok(ScanConfig::BASELINE),
        "robust_fast" => Ok(ScanConfig::ROBUST_FAST),
        "robust_full" | "robust_full_benchmark" => Ok(ScanConfig::ROBUST_FULL_BENCHMARK),
        _ => Err(PyValueError::new_err(format!(
            "unknown preset {preset:?}; expected 'baseline', 'robust_fast', or 'robust_full'"
        ))),
    }
}

fn scanner_config(
    preset: &str,
    max_dimension: u32,
    refine: bool,
    temporal: bool,
    rotation_period: u32,
    pool_ttl_frames: u64,
) -> PyResult<ScannerConfig> {
    if temporal && rotation_period == 0 {
        return Err(PyValueError::new_err(
            "rotation_period must be at least 1 in temporal mode",
        ));
    }
    let session = temporal.then_some(SessionConfig {
        rotation_period,
        pool_ttl_frames,
    });
    Ok(ScannerConfig {
        scan: preset_config(preset)?,
        options: ScanOptions {
            max_working_dim: max_dimension,
            refine,
        },
        session,
    })
}

fn parse_border(border: &str, constant: u8) -> PyResult<LineBorderMode> {
    match border {
        "replicate" => Ok(LineBorderMode::Replicate),
        "reflect" => Ok(LineBorderMode::Reflect),
        "constant" => Ok(LineBorderMode::Constant(constant)),
        _ => Err(PyValueError::new_err(format!(
            "unknown border {border:?}; expected 'replicate', 'reflect', or 'constant'"
        ))),
    }
}

fn raster_direction_name(direction: RasterDirection4) -> &'static str {
    match direction {
        RasterDirection4::Horizontal => "horizontal",
        RasterDirection4::DownRight => "down_right",
        RasterDirection4::Vertical => "vertical",
        RasterDirection4::DownLeft => "down_left",
    }
}

fn run_background_divide(
    pixels: &[u8],
    width: usize,
    height: usize,
    config: &BackgroundDivideConfig,
    workspace: &mut IlluminationWorkspace,
) -> Result<Vec<u8>, String> {
    let source = Gray8View::new(pixels, width, height, width).map_err(|e| e.to_string())?;
    let mut output = Gray8Image::new(Size::new(width, height)).map_err(|e| e.to_string())?;
    background_divide_into(source, output.view_mut(), config, workspace)
        .map_err(|e| e.to_string())?;
    Ok(output.into_vec())
}

fn run_van_cittert(
    pixels: &[u8],
    width: usize,
    height: usize,
    config: &VanCittertConfig,
    workspace: &mut DeblurWorkspace,
) -> Result<Vec<u8>, String> {
    let source = Gray8View::new(pixels, width, height, width).map_err(|e| e.to_string())?;
    let mut output = Gray8Image::new(Size::new(width, height)).map_err(|e| e.to_string())?;
    van_cittert_line_into(source, output.view_mut(), config, workspace)
        .map_err(|e| e.to_string())?;
    Ok(output.into_vec())
}

fn run_blur_estimate(
    pixels: &[u8],
    width: usize,
    height: usize,
    minimum_transitions: usize,
    minimum_amplitude: u8,
) -> Result<BlurEstimate, String> {
    let source = Gray8View::new(pixels, width, height, width).map_err(|e| e.to_string())?;
    let direction = estimate_line_direction(source);
    let blur_length = estimate_line_length_with(
        source,
        direction.theta_radians,
        &LineLengthConfig {
            minimum_transitions,
            minimum_amplitude,
        },
    );
    Ok(BlurEstimate {
        theta_radians: direction.theta_radians,
        confidence: direction.confidence,
        raster_direction: raster_direction_name(direction.raster_direction),
        blur_length,
    })
}

#[pyfunction]
#[pyo3(signature = (image, *, preset = "robust_fast", max_dimension = 1280, refine = false))]
fn scan<'py>(
    py: Python<'py>,
    image: PyReadonlyArray2<'py, u8>,
    preset: &str,
    max_dimension: u32,
    refine: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let (pixels, width, height) = copy_gray8(image)?;
    let config = scanner_config(preset, max_dimension, refine, false, 3, 4)?;
    let detections = py.detach(move || {
        let source = Gray8View::new(&pixels, width, height, width)
            .expect("copy_gray8 produced a valid packed image");
        CoreScanner::new(config).scan(&source)
    });
    pythonize(py, &detections).map_err(runtime_error)
}

#[pyfunction]
#[pyo3(signature = (image, *, structuring_element = 31, target_luma = 200, denominator_floor = 8))]
fn background_divide<'py>(
    py: Python<'py>,
    image: PyReadonlyArray2<'py, u8>,
    structuring_element: usize,
    target_luma: u8,
    denominator_floor: u8,
) -> PyResult<Bound<'py, PyArray2<u8>>> {
    let (pixels, width, height) = copy_gray8(image)?;
    let config = BackgroundDivideConfig {
        structuring_element,
        target_luma,
        denominator_floor,
    };
    let output = py
        .detach(move || {
            run_background_divide(
                &pixels,
                width,
                height,
                &config,
                &mut IlluminationWorkspace::default(),
            )
        })
        .map_err(runtime_error)?;
    image_to_numpy(py, output, width, height)
}

#[pyfunction]
#[pyo3(signature = (image, *, minimum_transitions = 8, minimum_amplitude = 24))]
fn estimate_line_blur<'py>(
    py: Python<'py>,
    image: PyReadonlyArray2<'py, u8>,
    minimum_transitions: usize,
    minimum_amplitude: u8,
) -> PyResult<Bound<'py, PyAny>> {
    if minimum_transitions == 0 || minimum_amplitude == 0 {
        return Err(PyValueError::new_err(
            "minimum_transitions and minimum_amplitude must be non-zero",
        ));
    }
    let (pixels, width, height) = copy_gray8(image)?;
    let estimate = py
        .detach(move || {
            run_blur_estimate(
                &pixels,
                width,
                height,
                minimum_transitions,
                minimum_amplitude,
            )
        })
        .map_err(runtime_error)?;
    pythonize(py, &estimate).map_err(runtime_error)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (image, theta_radians, blur_length, *, iterations = 3, relaxation = 1.0, border = "replicate", border_value = 0))]
fn van_cittert<'py>(
    py: Python<'py>,
    image: PyReadonlyArray2<'py, u8>,
    theta_radians: f64,
    blur_length: usize,
    iterations: usize,
    relaxation: f64,
    border: &str,
    border_value: u8,
) -> PyResult<Bound<'py, PyArray2<u8>>> {
    let (pixels, width, height) = copy_gray8(image)?;
    let config = VanCittertConfig {
        theta_radians,
        blur_length,
        iterations,
        relaxation,
        border: parse_border(border, border_value)?,
    };
    let output = py
        .detach(move || {
            run_van_cittert(
                &pixels,
                width,
                height,
                &config,
                &mut DeblurWorkspace::default(),
            )
        })
        .map_err(runtime_error)?;
    image_to_numpy(py, output, width, height)
}

/// Stateful scanner which optionally retains temporal video state.
#[pyclass(name = "Scanner")]
struct PyScanner {
    scanner: CoreScanner,
}

#[pymethods]
impl PyScanner {
    #[new]
    #[pyo3(signature = (preset = "robust_fast", *, max_dimension = 1280, refine = false, temporal = false, rotation_period = 3, pool_ttl_frames = 4))]
    fn new(
        preset: &str,
        max_dimension: u32,
        refine: bool,
        temporal: bool,
        rotation_period: u32,
        pool_ttl_frames: u64,
    ) -> PyResult<Self> {
        Ok(Self {
            scanner: CoreScanner::new(scanner_config(
                preset,
                max_dimension,
                refine,
                temporal,
                rotation_period,
                pool_ttl_frames,
            )?),
        })
    }

    fn scan<'py>(
        &mut self,
        py: Python<'py>,
        image: PyReadonlyArray2<'py, u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (pixels, width, height) = copy_gray8(image)?;
        let detections = py.detach(|| {
            let source = Gray8View::new(&pixels, width, height, width)
                .expect("copy_gray8 produced a valid packed image");
            self.scanner.scan(&source)
        });
        pythonize(py, &detections).map_err(runtime_error)
    }

    fn reset(&mut self) {
        self.scanner.reset();
    }
}

/// Reuses image-processing scratch buffers across frames.
#[pyclass(name = "ImageProcessor")]
#[derive(Default)]
struct PyImageProcessor {
    illumination: IlluminationWorkspace,
    deblur: DeblurWorkspace,
}

#[pymethods]
impl PyImageProcessor {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[pyo3(signature = (image, *, structuring_element = 31, target_luma = 200, denominator_floor = 8))]
    fn background_divide<'py>(
        &mut self,
        py: Python<'py>,
        image: PyReadonlyArray2<'py, u8>,
        structuring_element: usize,
        target_luma: u8,
        denominator_floor: u8,
    ) -> PyResult<Bound<'py, PyArray2<u8>>> {
        let (pixels, width, height) = copy_gray8(image)?;
        let config = BackgroundDivideConfig {
            structuring_element,
            target_luma,
            denominator_floor,
        };
        let output = py
            .detach(|| {
                run_background_divide(&pixels, width, height, &config, &mut self.illumination)
            })
            .map_err(runtime_error)?;
        image_to_numpy(py, output, width, height)
    }

    #[pyo3(signature = (image, *, minimum_transitions = 8, minimum_amplitude = 24))]
    fn estimate_line_blur<'py>(
        &self,
        py: Python<'py>,
        image: PyReadonlyArray2<'py, u8>,
        minimum_transitions: usize,
        minimum_amplitude: u8,
    ) -> PyResult<Bound<'py, PyAny>> {
        if minimum_transitions == 0 || minimum_amplitude == 0 {
            return Err(PyValueError::new_err(
                "minimum_transitions and minimum_amplitude must be non-zero",
            ));
        }
        let (pixels, width, height) = copy_gray8(image)?;
        let estimate = py
            .detach(move || {
                run_blur_estimate(
                    &pixels,
                    width,
                    height,
                    minimum_transitions,
                    minimum_amplitude,
                )
            })
            .map_err(runtime_error)?;
        pythonize(py, &estimate).map_err(runtime_error)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (image, theta_radians, blur_length, *, iterations = 3, relaxation = 1.0, border = "replicate", border_value = 0))]
    fn van_cittert<'py>(
        &mut self,
        py: Python<'py>,
        image: PyReadonlyArray2<'py, u8>,
        theta_radians: f64,
        blur_length: usize,
        iterations: usize,
        relaxation: f64,
        border: &str,
        border_value: u8,
    ) -> PyResult<Bound<'py, PyArray2<u8>>> {
        let (pixels, width, height) = copy_gray8(image)?;
        let config = VanCittertConfig {
            theta_radians,
            blur_length,
            iterations,
            relaxation,
            border: parse_border(border, border_value)?,
        };
        let output = py
            .detach(|| run_van_cittert(&pixels, width, height, &config, &mut self.deblur))
            .map_err(runtime_error)?;
        image_to_numpy(py, output, width, height)
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", VERSION)?;
    module.add_function(wrap_pyfunction!(scan, module)?)?;
    module.add_function(wrap_pyfunction!(background_divide, module)?)?;
    module.add_function(wrap_pyfunction!(estimate_line_blur, module)?)?;
    module.add_function(wrap_pyfunction!(van_cittert, module)?)?;
    module.add_class::<PyScanner>()?;
    module.add_class::<PyImageProcessor>()?;
    Ok(())
}
