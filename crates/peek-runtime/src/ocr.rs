//! Local OS OCR, no image upload.
#[cfg(target_os = "macos")]
pub fn recognize(image: &image::RgbaImage) -> Result<String, String> {
    macos::recognize(image)
}

#[cfg(target_os = "macos")]
mod macos {
    use image::{RgbImage, RgbaImage};
    use objc2::{AnyThread, rc::autoreleasepool};
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
    };
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSError};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };
    use std::sync::Mutex;

    // Closing one snip does not stop its worker. Keep rapid consecutive snips
    // from competing for Vision's native recognition resources.
    static OCR_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug)]
    struct Failure {
        domain: String,
        code: isize,
        message: String,
    }

    impl Failure {
        fn from_native(error: &NSError) -> Self {
            Self {
                domain: error.domain().to_string(),
                code: error.code(),
                message: error.to_string(),
            }
        }

        fn image_reader(&self) -> bool {
            self.domain == "TextRecognition.CRImageReaderError" && self.code == 1
        }
    }

    pub(super) fn recognize(image: &RgbaImage) -> Result<String, String> {
        if image.width() == 0 || image.height() == 0 {
            return Err("OCR image is empty".to_owned());
        }
        let _guard = OCR_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        recover(image, perform)
    }

    fn recover(
        image: &RgbaImage,
        mut attempt: impl FnMut(&RgbImage, bool) -> Result<String, Failure>,
    ) -> Result<String, String> {
        let rgb = opaque_rgb(image);
        match attempt(&rgb, false) {
            Ok(text) => Ok(text),
            Err(first) if first.image_reader() => {
                // A new decoder/request and a small border provide a distinct
                // local recovery path; do not change language detection or send
                // the image to an AI service. Bound this to one retry.
                eprintln!(
                    "[ocr] native image reader failed; retrying locally ({}x{})",
                    image.width(),
                    image.height()
                );
                let padded =
                    image::ImageBuffer::from_fn(rgb.width() + 32, rgb.height() + 32, |x, y| {
                        if x >= 16 && y >= 16 && x < rgb.width() + 16 && y < rgb.height() + 16 {
                            *rgb.get_pixel(x - 16, y - 16)
                        } else {
                            image::Rgb([255, 255, 255])
                        }
                    });
                attempt(&padded, true).map_err(|second| {
                    format!(
                        "{} [local retry: {} ({})]",
                        first.message, second.domain, second.code
                    )
                })
            }
            Err(error) => Err(error.message),
        }
    }

    // Flatten alpha explicitly, then pass opaque 8-bit RGB in a known color
    // space rather than asking Vision/Core Image to decode an RGBA PNG.
    fn opaque_rgb(image: &RgbaImage) -> RgbImage {
        image::ImageBuffer::from_fn(image.width(), image.height(), |x, y| {
            let pixel = image.get_pixel(x, y);
            let alpha = u32::from(pixel[3]);
            image::Rgb(std::array::from_fn(|i| {
                ((u32::from(pixel[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
            }))
        })
    }

    fn perform(image: &RgbImage, recovery: bool) -> Result<String, Failure> {
        // Every attempt runs on a Rust background thread. Drain native temporary
        // objects even when Vision fails; none of them escape the pool.
        autoreleasepool(|_| {
            let options = NSDictionary::new();
            let handler = if recovery {
                let mut bytes = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgb8(image.clone())
                    .write_to(&mut bytes, image::ImageFormat::Png)
                    .map_err(|error| local_failure(error.to_string()))?;
                let data = NSData::with_bytes(bytes.get_ref());
                VNImageRequestHandler::initWithData_options(
                    VNImageRequestHandler::alloc(),
                    &data,
                    &options,
                )
            } else {
                let data = CFData::from_bytes(image.as_raw());
                let provider = CGDataProvider::with_cf_data(Some(&data))
                    .ok_or_else(|| local_failure("Cannot create OCR pixel provider".to_owned()))?;
                let space = CGColorSpace::new_device_rgb()
                    .ok_or_else(|| local_failure("Cannot create OCR color space".to_owned()))?;
                // RGB24: no alpha, no premultiplication or byte-order ambiguity.
                let bitmap = unsafe {
                    CGImage::new(
                        image.width() as usize,
                        image.height() as usize,
                        8,
                        24,
                        image.width() as usize * 3,
                        Some(&space),
                        CGBitmapInfo::empty(),
                        Some(&provider),
                        std::ptr::null(),
                        false,
                        CGColorRenderingIntent::RenderingIntentDefault,
                    )
                }
                .ok_or_else(|| local_failure("Cannot create OCR bitmap".to_owned()))?;
                // Bitmap and its provider are retained by the handler; options
                // is an empty dictionary of the expected VNImageOption type.
                unsafe {
                    VNImageRequestHandler::initWithCGImage_options(
                        VNImageRequestHandler::alloc(),
                        &bitmap,
                        &options,
                    )
                }
            };
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            request.setAutomaticallyDetectsLanguage(true);
            request.setUsesLanguageCorrection(true);
            if recovery {
                // Retain compatibility with older supported macOS versions.
                // This retry avoids GPU contention; normal recognition retains
                // hardware acceleration. The property is deprecated, not removed.
                #[allow(deprecated)]
                unsafe {
                    request.setUsesCPUOnly(true);
                }
            }
            let requests = NSArray::<VNRequest>::from_slice(&[&request]);
            handler
                .performRequests_error(&requests)
                .map_err(|error| Failure::from_native(&error))?;
            let mut lines = Vec::new();
            if let Some(results) = request.results() {
                for result in results.iter() {
                    if let Some(candidate) = result.topCandidates(1).firstObject() {
                        lines.push(candidate.string().to_string());
                    }
                }
            }
            Ok(lines.join("\n"))
        })
    }

    fn local_failure(message: String) -> Failure {
        Failure {
            domain: "CrantPeek.OCRInput".to_owned(),
            code: 0,
            message,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn reader_error() -> Failure {
            Failure {
                domain: "TextRecognition.CRImageReaderError".to_owned(),
                code: 1,
                message: "reader failed".to_owned(),
            }
        }

        #[test]
        fn recovery_preserves_pixels_and_language_independent_text() {
            let input = RgbaImage::from_pixel(6, 4, image::Rgba([30, 180, 60, 255]));
            let mut calls = 0;
            let text = recover(&input, |pixels, cpu| {
                calls += 1;
                if calls == 1 {
                    assert!(!cpu);
                    return Err(reader_error());
                }
                assert!(cpu);
                assert_eq!(pixels.dimensions(), (38, 36));
                assert_eq!(pixels.get_pixel(16, 16).0, [30, 180, 60]);
                assert_eq!(pixels.get_pixel(0, 0).0, [255; 3]);
                Ok("imperial\n查询".to_owned())
            })
            .unwrap();
            assert_eq!(calls, 2);
            assert_eq!(text, "imperial\n查询");
        }

        #[test]
        fn recovery_is_bounded_and_does_not_hide_other_errors_or_empty_results() {
            let image = RgbaImage::new(3, 3);
            let mut calls = 0;
            let result = recover(&image, |_, _| {
                calls += 1;
                Err(reader_error())
            });
            assert_eq!(calls, 2);
            assert!(result.unwrap_err().contains("local retry"));
            calls = 0;
            let result = recover(&image, |_, _| {
                calls += 1;
                Err(local_failure("invalid input".to_owned()))
            });
            assert_eq!(calls, 1);
            assert_eq!(result.unwrap_err(), "invalid input");
            calls = 0;
            let result = recover(&image, |_, _| {
                calls += 1;
                Ok(String::new())
            });
            assert_eq!(calls, 1);
            assert_eq!(result.unwrap(), "");
        }

        #[test]
        fn empty_input_is_rejected_and_transparency_is_flattened() {
            assert!(recognize(&RgbaImage::new(0, 0)).is_err());
            let image = RgbaImage::from_pixel(1, 1, image::Rgba([0, 100, 200, 128]));
            assert_eq!(opaque_rgb(&image).get_pixel(0, 0).0, [127, 177, 227]);
        }

        #[test]
        #[ignore = "requires a local OCR fixture and native macOS recognition"]
        fn repeated_native_recognition() {
            let path = std::env::var("PEEK_OCR_TEST_IMAGE").expect("set PEEK_OCR_TEST_IMAGE");
            let expected = std::env::var("PEEK_OCR_TEST_TEXT").expect("set PEEK_OCR_TEST_TEXT");
            let image = image::open(path).unwrap().to_rgba8();
            // Exercise both native image routes, then overlapping snip workers.
            let fallback = recover(&image, |pixels, recovery| {
                if recovery {
                    perform(pixels, true)
                } else {
                    Err(reader_error())
                }
            })
            .unwrap();
            assert!(fallback.contains(&expected));
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    scope.spawn(|| {
                        for _ in 0..5 {
                            let result = recognize(&image).unwrap();
                            assert!(result.contains(&expected));
                        }
                    });
                }
            });
        }
    }
}
#[cfg(windows)]
pub fn recognize(image: &image::RgbaImage) -> Result<String, String> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED).map_err(|e| e.to_string())?;
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    let mut bytes = image.as_raw().clone();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(|e| e.to_string())?;
    writer.WriteBytes(&bytes).map_err(|e| e.to_string())?;
    let buffer = writer.DetachBuffer().map_err(|e| e.to_string())?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        image.width() as i32,
        image.height() as i32,
    )
    .map_err(|e| e.to_string())?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|_| "error-ocr-language-pack".to_owned())?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|e| e.to_string())?;
    Ok(result.Text().map_err(|e| e.to_string())?.to_string())
}
