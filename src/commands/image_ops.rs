use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilterType, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};

use crate::common;
use crate::error::{ToolError, ToolResult};

const MAX_INPUT_PIXELS: u64 = 100_000_000;

pub(crate) const STILL_EXTENSIONS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".webp", ".bmp", ".tif", ".tiff", ".qoi",
];

pub(crate) const ALPHA_EXTENSIONS: &[&str] = &[".png", ".webp", ".tif", ".tiff", ".qoi"];

pub(crate) fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .clamp(1, 8)
}

pub(crate) fn normalize_output_directory(
    tool: &str,
    output: Option<PathBuf>,
) -> ToolResult<Option<PathBuf>> {
    let Some(output) = output else {
        return Ok(None);
    };
    let absolute = if output.is_absolute() {
        output
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(output)
    };
    if absolute.exists() && !absolute.is_dir() {
        return Err(ToolError::usage(
            tool,
            format!(
                "--output must be a directory: {}",
                common::display_path(&absolute)
            ),
        ));
    }
    Ok(Some(absolute.canonicalize().unwrap_or(absolute)))
}

pub(crate) fn extension(path: &Path) -> String {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|value| format!(".{}", value.to_ascii_lowercase()))
        .unwrap_or_default()
}

pub(crate) fn assert_static(tool: &str, path: &Path) -> ToolResult {
    let format = ImageReader::open(path)
        .map_err(|error| {
            ToolError::new(
                tool,
                format!("cannot open {}: {error}", common::display_path(path)),
            )
        })?
        .with_guessed_format()
        .map_err(|error| {
            ToolError::new(
                tool,
                format!(
                    "cannot detect image format for {}: {error}",
                    common::display_path(path)
                ),
            )
        })?
        .format();
    let unsafe_kind = if format == Some(ImageFormat::Png) && super::media::animated_png(path) {
        Some("animated PNG")
    } else if format == Some(ImageFormat::WebP) && super::media::animated_webp(path) {
        Some("animated WebP")
    } else if format == Some(ImageFormat::Tiff) && super::media::multipage_tiff(path) {
        Some("multi-page TIFF")
    } else {
        None
    };
    if let Some(kind) = unsafe_kind {
        return Err(ToolError::new(
            tool,
            format!(
                "{kind} is not supported and was left unchanged: {}",
                common::display_path(path)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn load_oriented(path: &Path) -> Result<DynamicImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|error| format!("cannot open image: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("cannot detect image format: {error}"))?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| format!("unsupported or corrupt image: {error}"))?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_INPUT_PIXELS {
        return Err(format!(
            "image is {width}x{height} ({:.1} megapixels); the safety limit is {:.0} megapixels",
            pixels as f64 / 1_000_000.0,
            MAX_INPUT_PIXELS as f64 / 1_000_000.0
        ));
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("cannot decode image: {error}"))?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub(crate) fn temp_path(tool: &str, output: &Path) -> Result<PathBuf, String> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let extension = output.extension().and_then(OsStr::to_str).unwrap_or("tmp");
    let temporary = tempfile::Builder::new()
        .prefix(&format!(".{tool}-"))
        .suffix(&format!(".tmp.{extension}"))
        .tempfile_in(parent)
        .map_err(|error| error.to_string())?;
    let (file, path) = temporary.keep().map_err(|error| error.error.to_string())?;
    drop(file);
    Ok(path)
}

fn output_format(output: &Path) -> Result<ImageFormat, String> {
    ImageFormat::from_extension(output.extension().unwrap_or_default()).ok_or_else(|| {
        format!(
            "cannot determine output format from {}",
            common::display_path(output)
        )
    })
}

pub(crate) fn encode_preserving_format(
    image: &DynamicImage,
    output: &Path,
    jpeg_quality: u32,
) -> Result<(), String> {
    let format = output_format(output)?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    match format {
        ImageFormat::Jpeg => {
            let rgb = image.to_rgb8();
            JpegEncoder::new_with_quality(&mut writer, jpeg_quality as u8)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|error| error.to_string())?;
        }
        ImageFormat::Png => {
            PngEncoder::new_with_quality(
                &mut writer,
                CompressionType::Default,
                PngFilterType::Adaptive,
            )
            .write_image(
                image.as_bytes(),
                image.width(),
                image.height(),
                image.color().into(),
            )
            .map_err(|error| error.to_string())?;
        }
        ImageFormat::WebP => {
            let rgba = image.to_rgba8();
            WebPEncoder::new_lossless(&mut writer)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|error| error.to_string())?;
        }
        format => image
            .write_to(&mut writer, format)
            .map_err(|error| error.to_string())?,
    }
    writer.flush().map_err(|error| error.to_string())
}

pub(crate) fn validate_encoded_image(
    temporary: &Path,
    expected_dimensions: (u32, u32),
    expected_format: Option<ImageFormat>,
) -> Result<u64, String> {
    let bytes = match fs::metadata(temporary) {
        Ok(metadata) if metadata.len() > 0 => metadata.len(),
        Ok(_) => return Err("encoder produced an empty output".into()),
        Err(error) => return Err(format!("encoder produced no output: {error}")),
    };
    let reader = ImageReader::open(temporary)
        .map_err(|error| format!("cannot reopen encoded output: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("cannot detect encoded output format: {error}"))?;
    if let Some(expected) = expected_format
        && reader.format() != Some(expected)
    {
        return Err(format!(
            "encoder produced {:?} instead of {:?}",
            reader.format(),
            expected
        ));
    }
    let dimensions = reader
        .into_dimensions()
        .map_err(|error| format!("cannot read encoded output dimensions: {error}"))?;
    if dimensions != expected_dimensions {
        return Err(format!(
            "encoder produced {}x{} instead of {}x{}",
            dimensions.0, dimensions.1, expected_dimensions.0, expected_dimensions.1
        ));
    }
    Ok(bytes)
}

pub(crate) fn preserve_permissions(
    source: &Path,
    output: &Path,
    temporary: &Path,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        let permissions = fs::metadata(output)
            .or_else(|_| fs::metadata(source))
            .map_err(|error| error.to_string())?
            .permissions();
        fs::set_permissions(temporary, permissions).map_err(|error| error.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = (source, output, temporary);
    }
    Ok(())
}

pub(crate) fn output_readonly(source: &Path, output: &Path) -> Result<bool, String> {
    #[cfg(windows)]
    {
        fs::metadata(output)
            .or_else(|_| fs::metadata(source))
            .map(|metadata| metadata.permissions().readonly())
            .map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (source, output);
        Ok(false)
    }
}

#[allow(clippy::permissions_set_readonly_false)]
pub(crate) fn restore_readonly(output: &Path, readonly: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(output)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(output, permissions).map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = (output, readonly);
        Ok(())
    }
}

pub(crate) fn duration(value: Duration) -> String {
    if value.as_secs() < 1 {
        format!("{} ms", value.as_millis())
    } else {
        format!("{:.1} s", value.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_checks_use_file_signatures_not_extensions() {
        let directory = tempfile::tempdir().unwrap();

        let disguised_png = directory.path().join("animation.webp");
        let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
        png.extend_from_slice(&0_u32.to_be_bytes());
        png.extend_from_slice(b"acTL");
        png.extend_from_slice(&0_u32.to_be_bytes());
        fs::write(&disguised_png, png).unwrap();
        let error = assert_static("justcrop", &disguised_png).unwrap_err();
        assert!(error.message().contains("animated PNG"));

        let disguised_webp = directory.path().join("animation.png");
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&4_u32.to_le_bytes());
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(b"ANIM");
        webp.extend_from_slice(&0_u32.to_le_bytes());
        fs::write(&disguised_webp, webp).unwrap();
        let error = assert_static("justjpg", &disguised_webp).unwrap_err();
        assert!(error.message().contains("animated WebP"));
    }
}
