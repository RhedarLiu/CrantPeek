//! Local OS OCR, no image upload.
#[cfg(target_os = "macos")]
pub fn recognize(image: &image::RgbaImage) -> Result<String, String> {
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let data = NSData::with_bytes(bytes.get_ref());
    let handler = VNImageRequestHandler::initWithData_options(
        VNImageRequestHandler::alloc(),
        &data,
        &NSDictionary::new(),
    );
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setAutomaticallyDetectsLanguage(true);
    request.setUsesLanguageCorrection(true);
    let requests = NSArray::<VNRequest>::from_slice(&[&request]);
    handler
        .performRequests_error(&requests)
        .map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    if let Some(results) = request.results() {
        for result in results.iter() {
            if let Some(candidate) = result.topCandidates(1).firstObject() {
                lines.push(candidate.string().to_string());
            }
        }
    }
    Ok(lines.join("\n"))
}
#[cfg(windows)]
pub fn recognize(image: &image::RgbaImage) -> Result<String, String> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
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
        .map_err(|e| format!("Install an OCR language pack: {e}"))?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|e| e.to_string())?;
    Ok(result.Text().map_err(|e| e.to_string())?.to_string())
}
