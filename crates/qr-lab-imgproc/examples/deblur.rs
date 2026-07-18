use qr_lab_image::Gray8View;
use qr_lab_imgproc::blur::estimate_line_direction;
use qr_lab_imgproc::deblur::{van_cittert_line, VanCittertConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pixels = vec![128u8; 320 * 240];
    let frame = Gray8View::new(&pixels, 320, 240, 320)?;
    let direction = estimate_line_direction(frame);
    let restored = van_cittert_line(
        frame,
        &VanCittertConfig {
            theta_radians: direction.theta_radians,
            ..VanCittertConfig::default()
        },
    )?;
    println!("restored {} grayscale pixels", restored.as_slice().len());
    Ok(())
}
