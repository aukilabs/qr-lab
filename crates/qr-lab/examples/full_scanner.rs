use qr_lab::image::Gray8View;
use qr_lab::{Scanner, ScannerConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pixels = vec![255u8; 640 * 480];
    let frame = Gray8View::new(&pixels, 640, 480, 640)?;
    let mut scanner = Scanner::new(ScannerConfig::robust_fast());
    let result = scanner.scan(&frame);
    println!("decoded {} QR codes", result.codes.len());
    Ok(())
}
