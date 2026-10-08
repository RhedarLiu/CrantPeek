//! OCR a deliberately blank white image without capturing the user's screen.
#[path = "../src/ocr.rs"]
mod ocr;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let image = if let Some(path) = args.first() {
        image::open(path).expect("Open test image").to_rgba8()
    } else {
        image::RgbaImage::from_pixel(640, 320, image::Rgba([255, 255, 255, 255]))
    };
    match ocr::recognize(&image) {
        Ok(text) => {
            if args.is_empty() {
                assert!(
                    text.trim().is_empty(),
                    "Blank image should not contain recognized text"
                );
            }
            if let Some(expected) = args.get(1) {
                assert!(
                    text.contains(expected),
                    "Expected {expected:?}, got {text:?}"
                );
            }
            println!("OS OCR smoke test passed: {text}");
        }
        Err(e) => {
            eprintln!("OS OCR smoke test failed: {e}");
            std::process::exit(1);
        }
    }
}
