//! Text from a clipboard image with the Windows OCR engine. Runs locally.

#[cfg(windows)]
pub fn recognize(image: &image::DynamicImage) -> Result<String, String> {
    use windows::{
        Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize},
    };
    // Already initialized threads keep their apartment; either way WinRT works.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
        "Windows has no OCR language installed; add one in Settings > Time & language > Language".to_owned()
    })?;
    let limit = OcrEngine::MaxImageDimension().unwrap_or(10000).max(1);
    let image = if image.width() > limit || image.height() > limit {
        image.resize(limit, limit, image::imageops::FilterType::Triangle)
    } else {
        image.clone()
    };
    let mut pixels = image.to_rgba8().into_raw();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let failed = |e: windows::core::Error| format!("Windows OCR failed: {}", e.message());
    let writer = DataWriter::new().map_err(failed)?;
    writer.WriteBytes(&pixels).map_err(failed)?;
    let buffer = writer.DetachBuffer().map_err(failed)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        image.width() as i32,
        image.height() as i32,
    )
    .map_err(failed)?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(failed)?
        .get()
        .map_err(failed)?;
    let mut text = String::new();
    for line in result.Lines().map_err(failed)? {
        text.push_str(&line.Text().map_err(failed)?.to_string_lossy());
        text.push('\n');
    }
    Ok(text)
}

#[cfg(not(windows))]
pub fn recognize(_image: &image::DynamicImage) -> Result<String, String> {
    Err("text from images uses the Windows OCR engine and is Windows-only".into())
}
