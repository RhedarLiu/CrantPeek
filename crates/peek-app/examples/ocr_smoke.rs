//! OCR a deliberately blank white image without capturing the user's screen.
#[path = "../src/ocr.rs"]
mod ocr;
fn main() {
    let image = image::RgbaImage::from_pixel(640, 320, image::Rgba([255, 255, 255, 255]));
    match ocr::recognize(&image) {
        Ok(text) => {
            assert!(
                text.trim().is_empty(),
                "Blank image should not contain recognized text"
            );
            println!("OS OCR smoke test passed: blank image, no text");
        }
        Err(e) => {
            eprintln!("OS OCR smoke test failed: {e}");
            std::process::exit(1);
        }
    }
}
