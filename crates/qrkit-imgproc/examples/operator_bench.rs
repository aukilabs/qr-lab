use qrkit_image::{Gray8Image, Gray8View};
use qrkit_imgproc::deblur::{van_cittert_line_into, DeblurWorkspace, VanCittertConfig};
use std::time::Instant;

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280usize, 720usize);
    let pixels: Vec<u8> = (0..width * height)
        .map(|index| ((index * 37 + index / width * 11) & 255) as u8)
        .collect();
    let source = Gray8View::new(&pixels, width, height, width)?;
    let mut output = Gray8Image::new(source.size())?;
    let mut workspace = DeblurWorkspace::default();
    let config = VanCittertConfig {
        blur_length: 7,
        ..VanCittertConfig::default()
    };

    for _ in 0..10 {
        van_cittert_line_into(source, output.view_mut(), &config, &mut workspace)?;
    }
    let mut samples_ms = Vec::with_capacity(50);
    for _ in 0..50 {
        let start = Instant::now();
        van_cittert_line_into(source, output.view_mut(), &config, &mut workspace)?;
        samples_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    samples_ms.sort_by(f64::total_cmp);
    let mean = samples_ms.iter().sum::<f64>() / samples_ms.len() as f64;
    println!(
        "operator=van_cittert_line size={}x{} pixels={} iterations={} warmup=10 samples=50 mean_ms={:.3} p50_ms={:.3} p95_ms={:.3}",
        width,
        height,
        width * height,
        config.iterations,
        mean,
        percentile(&samples_ms, 0.50),
        percentile(&samples_ms, 0.95),
    );
    Ok(())
}
