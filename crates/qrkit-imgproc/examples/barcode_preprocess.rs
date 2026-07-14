use qrkit_image::{Gray8Image, Gray8View};
use qrkit_imgproc::deblur::{van_cittert_line_into, DeblurWorkspace, VanCittertConfig};
use qrkit_imgproc::illumination::{
    background_divide_into, BackgroundDivideConfig, IlluminationWorkspace,
};

fn preprocess_for_barcode(frame: Gray8View<'_>) -> Result<Gray8Image, Box<dyn std::error::Error>> {
    let mut normalized = Gray8Image::new(frame.size())?;
    let mut illumination = IlluminationWorkspace::default();
    background_divide_into(
        frame,
        normalized.view_mut(),
        &BackgroundDivideConfig::default(),
        &mut illumination,
    )?;

    let mut restored = Gray8Image::new(frame.size())?;
    let mut deblur = DeblurWorkspace::default();
    van_cittert_line_into(
        normalized.view(),
        restored.view_mut(),
        &VanCittertConfig::default(),
        &mut deblur,
    )?;
    Ok(restored)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pixels = vec![128u8; 320 * 200];
    let frame = Gray8View::new(&pixels, 320, 200, 320)?;
    let prepared = preprocess_for_barcode(frame)?;
    println!(
        "pass {} prepared pixels to a barcode decoder",
        prepared.as_slice().len()
    );
    Ok(())
}
