use qrkit_image::{Gray8View, Rect};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let width = 640;
    let height = 480;
    let stride = 672;
    let camera_y_plane = vec![128u8; stride * height];

    let frame = Gray8View::new(&camera_y_plane, width, height, stride)?;
    let roi = frame.subview(Rect::new(100, 80, 320, 240))?;
    println!(
        "zero-copy ROI: {}x{}, stride {}",
        roi.width(),
        roi.height(),
        roi.stride()
    );
    Ok(())
}
