use std::fs;
use std::io::Write;
use std::path::Path;

use atomic_write_file::AtomicWriteFile;
use image::codecs::png::{CompressionType, FilterType as PngFilterType, PngEncoder};
use image::imageops::{self, FilterType};
use image::{DynamicImage, GrayImage, ImageEncoder, RgbaImage};

use crate::error::{ToolError, ToolResult};

pub const MODEL_SIZE: u32 = 1024;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
const TOOL: &str = "justrmbg";

pub struct PreparedImage {
    pub chw: Vec<f32>,
    original: RgbaImage,
}

impl PreparedImage {
    pub fn load(path: &Path) -> ToolResult<Self> {
        let original = super::super::image_ops::load_oriented(path)
            .map_err(|error| ToolError::new(TOOL, error))?
            .to_rgba8();
        let resized = DynamicImage::ImageRgba8(original.clone())
            .resize_exact(MODEL_SIZE, MODEL_SIZE, FilterType::Lanczos3)
            .to_rgb8();
        Ok(Self {
            chw: normalize_rgb(resized.as_raw()),
            original,
        })
    }

    pub fn write_with_mask(self, mask: &[f32], output: &Path) -> ToolResult {
        let mask = mask_to_u8(mask)?;
        let model_mask = GrayImage::from_raw(MODEL_SIZE, MODEL_SIZE, mask)
            .ok_or_else(|| ToolError::new(TOOL, "model returned an invalid alpha mask"))?;
        let resized_mask = imageops::resize(
            &model_mask,
            self.original.width(),
            self.original.height(),
            FilterType::Lanczos3,
        );
        let mut rgba = self.original;
        for (pixel, alpha) in rgba.pixels_mut().zip(resized_mask.pixels()) {
            pixel.0[3] = ((u16::from(pixel.0[3]) * u16::from(alpha.0[0])) / 255) as u8;
        }
        write_png_atomic(output, &rgba)
    }
}

fn normalize_rgb(rgb: &[u8]) -> Vec<f32> {
    let pixels = rgb.len() / 3;
    let mut chw = vec![0.0; pixels * 3];
    let (pixel_chunks, _) = rgb.as_chunks::<3>();
    for (index, pixel) in pixel_chunks.iter().enumerate() {
        chw[index] = (f32::from(pixel[0]) / 255.0 - MEAN[0]) / STD[0];
        chw[pixels + index] = (f32::from(pixel[1]) / 255.0 - MEAN[1]) / STD[1];
        chw[pixels * 2 + index] = (f32::from(pixel[2]) / 255.0 - MEAN[2]) / STD[2];
    }
    chw
}

pub fn mask_to_u8(mask: &[f32]) -> ToolResult<Vec<u8>> {
    let expected = (MODEL_SIZE * MODEL_SIZE) as usize;
    if mask.len() != expected {
        return Err(ToolError::new(
            TOOL,
            format!(
                "model returned {} alpha values; expected {expected}",
                mask.len()
            ),
        ));
    }
    let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
    for &value in mask {
        min = min.min(value);
        max = max.max(value);
    }
    let needs_sigmoid = min < -0.01 || max > 1.01;
    Ok(mask
        .iter()
        .map(|&value| {
            let probability = if needs_sigmoid {
                1.0 / (1.0 + (-value).exp())
            } else {
                value
            };
            (probability.clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect())
}

fn write_png_atomic(path: &Path, rgba: &RgbaImage) -> ToolResult {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| ToolError::new(TOOL, format!("cannot stage output: {error}")))?;
    let encoder =
        PngEncoder::new_with_quality(&mut file, CompressionType::Default, PngFilterType::Adaptive);
    encoder
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| ToolError::new(TOOL, format!("cannot encode PNG: {error}")))?;
    file.flush()
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    file.commit()
        .map_err(|error| ToolError::new(TOOL, format!("cannot commit output: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_planar_imagenet() {
        let got = normalize_rgb(&[255, 0, 127, 0, 255, 128]);
        assert_eq!(got.len(), 6);
        assert!((got[0] - ((1.0 - MEAN[0]) / STD[0])).abs() < 1e-6);
        assert!((got[2] - ((0.0 - MEAN[1]) / STD[1])).abs() < 1e-6);
    }

    #[test]
    fn converts_logits_when_needed() {
        let mut input = vec![0.5; (MODEL_SIZE * MODEL_SIZE) as usize];
        input[0] = -1.0;
        input[1] = 2.0;
        let got = mask_to_u8(&input).unwrap();
        assert_eq!(&got[..3], &[69, 225, 159]);
    }

    #[test]
    fn output_replacement_is_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("out.png");
        fs::write(&path, b"old").unwrap();
        let rgba = RgbaImage::from_pixel(2, 1, image::Rgba([1, 2, 3, 4]));
        write_png_atomic(&path, &rgba).unwrap();
        assert_eq!(image::open(path).unwrap().to_rgba8(), rgba);
    }
}
