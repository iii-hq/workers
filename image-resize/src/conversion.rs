use anyhow::{anyhow, Result};
use image::{DynamicImage, ImageFormat, RgbaImage};
use pdf2image::{Pages, RenderOptionsBuilder, DPI, PDF};
use std::io::Cursor;

pub fn needs_conversion(format: &str) -> bool {
    matches!(format, "pdf" | "psd")
}

pub fn convert_to_image(bytes: &[u8], format: &str) -> Result<DynamicImage> {
    match format {
        "pdf" => convert_pdf(bytes),
        "psd" => convert_psd(bytes),
        _ => Err(anyhow!("Unsupported conversion format: {}", format)),
    }
}

pub fn to_png_bytes(img: &DynamicImage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);
    img.to_rgba8()
        .write_to(&mut cursor, ImageFormat::Png)?;
    Ok(buf)
}

// ── PDF ─────────────────────────────────────────────────

fn convert_pdf(bytes: &[u8]) -> Result<DynamicImage> {
    let pdf = PDF::from_bytes(bytes.to_vec())
        .map_err(|e| anyhow!("Failed to parse PDF: {}", e))?;

    let options = RenderOptionsBuilder::default()
        .resolution(DPI::Uniform(150))
        .build()
        .map_err(|e| anyhow!("Failed to build render options: {}", e))?;

    // Render only page 1 (1-indexed) to keep memory low for large PDFs
    let mut pages = pdf
        .render(Pages::Range(1..=1), options)
        .map_err(|e| anyhow!("Failed to render PDF page: {}", e))?;

    pages
        .pop()
        .ok_or_else(|| anyhow!("PDF rendered zero pages"))
}

// ── PSD ─────────────────────────────────────────────────

fn convert_psd(bytes: &[u8]) -> Result<DynamicImage> {
    let psd = psd::Psd::from_bytes(bytes).map_err(|e| anyhow!("Failed to parse PSD: {:?}", e))?;

    let width = psd.width();
    let height = psd.height();
    let pixels = psd.rgba();

    let rgba = RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| anyhow!("PSD pixel buffer size mismatch"))?;

    Ok(DynamicImage::ImageRgba8(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_conversion() {
        assert!(needs_conversion("pdf"));
        assert!(needs_conversion("psd"));
        assert!(!needs_conversion("doc"));
        assert!(!needs_conversion("docx"));
        assert!(!needs_conversion("jpeg"));
        assert!(!needs_conversion("png"));
        assert!(!needs_conversion("webp"));
    }

    #[test]
    fn test_to_png_bytes() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            10,
            10,
            image::Rgba([255, 0, 0, 255]),
        ));
        let bytes = to_png_bytes(&img).unwrap();
        assert!(!bytes.is_empty());
        // PNG magic bytes
        assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4e, 0x47]);
    }

    #[test]
    fn test_convert_psd() {
        let psd_bytes = std::fs::read("example/images/sample_640×426.psd")
            .expect("sample PSD file should exist");
        let img = convert_psd(&psd_bytes).unwrap();
        assert_eq!(img.width(), 640);
        assert_eq!(img.height(), 426);
    }

    #[test]
    fn test_convert_pdf() {
        let pdf_bytes =
            std::fs::read("example/images/handbook.pdf").expect("sample PDF file should exist");
        let img = convert_pdf(&pdf_bytes).unwrap();
        assert!(img.width() > 0);
        assert!(img.height() > 0);
    }
}
