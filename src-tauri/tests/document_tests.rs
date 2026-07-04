use image::{ImageBuffer, Rgb};
use print_bridge_lib::{
    document::{detect_format_from_bytes, fit_contain, image_to_pdf, DocumentFormat, FitRect},
    protocol::EffectivePaper,
};
use std::fs;

#[test]
fn detect_format_from_bytes_recognizes_pdf_png_and_jpeg_signatures() {
    assert_eq!(
        detect_format_from_bytes(b"%PDF-1.7\n..."),
        Some(DocumentFormat::Pdf)
    );
    assert_eq!(
        detect_format_from_bytes(b"\x89PNG\r\n\x1a\n..."),
        Some(DocumentFormat::Png)
    );
    assert_eq!(
        detect_format_from_bytes(b"\xff\xd8\xff\xe0..."),
        Some(DocumentFormat::Jpeg)
    );
}

#[test]
fn detect_format_from_bytes_returns_none_for_unknown_or_short_input() {
    assert_eq!(detect_format_from_bytes(b"not a document"), None);
    assert_eq!(detect_format_from_bytes(b"\xff\xd8"), None);
}

#[test]
fn fit_contain_preserves_aspect_ratio_and_centers_wide_image() {
    let rect = fit_contain(200.0, 100.0, 100.0, 100.0).unwrap();

    assert_eq!(
        rect,
        FitRect {
            x: 0.0,
            y: 25.0,
            width: 100.0,
            height: 50.0,
        }
    );
}

#[test]
fn fit_contain_preserves_aspect_ratio_and_centers_tall_image() {
    let rect = fit_contain(100.0, 200.0, 100.0, 100.0).unwrap();

    assert_eq!(
        rect,
        FitRect {
            x: 25.0,
            y: 0.0,
            width: 50.0,
            height: 100.0,
        }
    );
}

#[test]
fn fit_contain_rejects_zero_negative_nan_and_infinite_dimensions() {
    assert!(fit_contain(0.0, 100.0, 100.0, 100.0).is_err());
    assert!(fit_contain(100.0, -1.0, 100.0, 100.0).is_err());
    assert!(fit_contain(100.0, 100.0, f64::NAN, 100.0).is_err());
    assert!(fit_contain(100.0, 100.0, 100.0, f64::INFINITY).is_err());
}

#[test]
fn image_to_pdf_embeds_image_xobject_on_requested_page() {
    let image_path = temp_path("source.png");
    let pdf_path = temp_path("output.pdf");
    let _ = fs::remove_file(&image_path);
    let _ = fs::remove_file(&pdf_path);

    let image = ImageBuffer::from_fn(2, 1, |x, _| {
        if x == 0 {
            Rgb([255_u8, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    });
    image.save(&image_path).unwrap();

    image_to_pdf(
        &image_path,
        &EffectivePaper {
            width_mm: 50.8,
            height_mm: 25.4,
        },
        &pdf_path,
    )
    .unwrap();

    let pdf = fs::read(&pdf_path).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(contains_bytes(&pdf, b"/XObject"));
    assert!(contains_bytes(&pdf, b" Do"));
    let (media_width, media_height) = media_box_size(&pdf).unwrap();
    assert_close(media_width, 144.0);
    assert_close(media_height, 72.0);

    let _ = fs::remove_file(&image_path);
    let _ = fs::remove_file(&pdf_path);
}

fn temp_path(file_name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "print-bridge-document-test-{}-{file_name}",
        std::process::id()
    ))
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn media_box_size(pdf: &[u8]) -> Option<(f64, f64)> {
    let text = String::from_utf8_lossy(pdf);
    let media_box_start = text.find("/MediaBox")?;
    let box_values_start = text[media_box_start..].find('[')? + media_box_start + 1;
    let box_values_end = text[box_values_start..].find(']')? + box_values_start;
    let values: Vec<f64> = text[box_values_start..box_values_end]
        .split_whitespace()
        .filter_map(|value| value.parse().ok())
        .collect();

    if values.len() == 4 {
        Some((values[2] - values[0], values[3] - values[1]))
    } else {
        None
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.01,
        "expected {actual} to be within 0.01 of {expected}"
    );
}
