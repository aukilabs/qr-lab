use qrkit_image::Gray8View;
use qrkit_imgproc::deblur::{van_cittert_line, VanCittertConfig};

fn box_blur_horizontal(source: &[u8], width: usize, height: usize, length: usize) -> Vec<u8> {
    let radius = (length / 2) as isize;
    let mut output = vec![0; source.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0u32;
            for offset in -radius..=radius {
                let sx = (x as isize + offset).clamp(0, width as isize - 1) as usize;
                sum += source[y * width + sx] as u32;
            }
            output[y * width + x] = (sum / length as u32) as u8;
        }
    }
    output
}

fn mean_absolute_error(actual: &[u8], expected: &[u8]) -> f64 {
    actual
        .iter()
        .zip(expected)
        .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs() as f64)
        .sum::<f64>()
        / actual.len() as f64
}

fn amplitude(image: &[u8], width: usize, row: usize, margin: usize) -> u8 {
    let samples = &image[row * width + margin..(row + 1) * width - margin];
    samples.iter().max().unwrap() - samples.iter().min().unwrap()
}

fn restore(blurred: &[u8], width: usize, height: usize, length: usize) -> Vec<u8> {
    let view = Gray8View::new(blurred, width, height, width).unwrap();
    van_cittert_line(
        view,
        &VanCittertConfig {
            blur_length: length,
            ..VanCittertConfig::default()
        },
    )
    .unwrap()
    .into_vec()
}

#[test]
fn known_line_psf_improves_a_synthetic_barcode() {
    // Eight-pixel barcode modules under a 13-pixel smear. This is the
    // intended recoverable regime: substantial contrast collapse before the
    // line PSF's first unrecoverable frequency zero.
    let (width, height) = (192, 64);
    let sharp: Vec<u8> = (0..width * height)
        .map(|index| {
            if ((index % width) / 8) % 2 == 0 {
                35
            } else {
                220
            }
        })
        .collect();
    let blurred = box_blur_horizontal(&sharp, width, height, 13);
    let restored = restore(&blurred, width, height, 13);
    let blurred_amplitude = amplitude(&blurred, width, height / 2, 24);
    let restored_amplitude = amplitude(&restored, width, height / 2, 24);
    assert!(
        restored_amplitude >= blurred_amplitude.saturating_mul(2),
        "blurred amplitude {blurred_amplitude}, restored amplitude {restored_amplitude}; MAE {:.3} -> {:.3}",
        mean_absolute_error(&blurred, &sharp),
        mean_absolute_error(&restored, &sharp),
    );
}

#[test]
fn known_line_psf_improves_a_checker_marker() {
    let (width, height) = (128, 128);
    let mut sharp = vec![230u8; width * height];
    for y in 16..height - 16 {
        for x in 16..width - 16 {
            let cell_x = (x - 16) / 12;
            let cell_y = (y - 16) / 12;
            sharp[y * width + x] = if (cell_x + cell_y) % 2 == 0 { 25 } else { 230 };
        }
    }
    let blurred = box_blur_horizontal(&sharp, width, height, 9);
    let restored = restore(&blurred, width, height, 9);
    let blurred_amplitude = amplitude(&blurred, width, 34, 24);
    let restored_amplitude = amplitude(&restored, width, 34, 24);
    assert!(
        restored_amplitude > blurred_amplitude,
        "blurred amplitude {blurred_amplitude}, restored amplitude {restored_amplitude}; MAE {:.3} -> {:.3}",
        mean_absolute_error(&blurred, &sharp),
        mean_absolute_error(&restored, &sharp)
    );
}
