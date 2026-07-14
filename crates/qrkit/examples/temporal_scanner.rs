use qrkit::image::Gray8View;
use qrkit::{Scanner, ScannerConfig, SessionConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pixels = vec![255u8; 640 * 480];
    let frame = Gray8View::new(&pixels, 640, 480, 640)?;
    let config = ScannerConfig::robust_fast().temporal(SessionConfig::default());
    let mut scanner = Scanner::new(config);

    for _ in 0..3 {
        let result = scanner.scan(&frame);
        println!("decoded {} QR codes", result.codes.len());
    }
    scanner.reset();
    Ok(())
}
