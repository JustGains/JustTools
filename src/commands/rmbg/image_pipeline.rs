use std::fs;
use std::io::Write;
use std::path::Path;

use atomic_write_file::AtomicWriteFile;
use fast_image_resize::images::{Image as ResizeImage, ImageRef, TypedImage};
use fast_image_resize::pixels::F32;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::codecs::png::{CompressionType, FilterType as PngFilterType, PngEncoder};
use image::{ImageEncoder, RgbaImage};
use rayon::prelude::*;

use crate::error::{ToolError, ToolResult};

/// The model's input size. BRIA's model takes any size and was trained at 1024
/// by 1024, so that is the default; a model exported at a fixed size (such as a
/// smaller one for small devices) runs at its own size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelSize {
    pub width: u32,
    pub height: u32,
}

impl ModelSize {
    pub const DEFAULT: Self = Self {
        width: 1024,
        height: 1024,
    };

    pub fn pixels(self) -> usize {
        self.width as usize * self.height as usize
    }
}

const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
const TOOL: &str = "justrmbg";

/// The picture is scaled to the model's size and the mask back to the picture's
/// size with Lanczos filtering. BRIA's reference pipeline uses bilinear for the
/// picture (torchvision's Resize) and bicubic for the mask (Pillow's default);
/// against human-labelled masks (DIS-VD, 470 pictures) Lanczos for both made 2%
/// less error (mean absolute error 0.0202 against 0.0206 for the reference, the
/// same model in float32; bilinear for the mask was worst at 0.0211).
const INPUT_FILTER: FilterType = FilterType::Lanczos3;
const MASK_FILTER: FilterType = FilterType::Lanczos3;
/// Scaling a finished picture down, like Pillow's bicubic, on premultiplied alpha.
const PICTURE_FILTER: FilterType = FilterType::CatmullRom;

/// The file format of a cut-out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Png,
    WebP,
}

impl Format {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::WebP => "webp",
        }
    }

    /// The format an output path names by its extension, if it names one.
    pub fn of_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(Self::parse)
    }
}

/// How PNG cut-outs are compressed: `Fast` writes a 1 MP cut-out in about 10
/// ms, `Small` takes about a quarter of a second for a file about a fifth smaller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PngCompression {
    Fast,
    Small,
}

/// How a cut-out is saved. The same options exist as command-line flags and as
/// serve-mode request fields, with the same defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputOptions {
    /// `None` takes the format from the output's extension (PNG unless `.webp`).
    pub format: Option<Format>,
    /// Crop to what is visible (every pixel with any alpha).
    pub crop: bool,
    /// Transparent room added around a cropped subject, in pixels.
    pub padding: u32,
    /// Scale down (never up) to fit these.
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    /// WebP: quality and alpha quality 0 to 100, effort 0 to 6.
    pub quality: u8,
    pub alpha_quality: u8,
    pub method: u8,
    pub png: PngCompression,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            format: None,
            crop: false,
            padding: 0,
            max_width: None,
            max_height: None,
            quality: 80,
            alpha_quality: 80,
            method: 4,
            png: PngCompression::Fast,
        }
    }
}

impl OutputOptions {
    /// A finished picture as the JustGains import makes them: cropped to the
    /// subject with 10 px of room, at most 400 px tall, WebP.
    pub fn picture() -> Self {
        Self {
            format: Some(Format::WebP),
            crop: true,
            padding: 10,
            max_height: Some(400),
            ..Self::default()
        }
    }

    pub fn format_for(&self, output: &Path) -> Format {
        self.format
            .or_else(|| Format::of_path(output))
            .unwrap_or(Format::Png)
    }

    fn reshapes(&self) -> bool {
        self.crop || self.padding > 0 || self.max_width.is_some() || self.max_height.is_some()
    }
}

/// What a cut-out turned into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Written {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

pub struct PreparedImage {
    pub chw: Vec<f32>,
    original: RgbaImage,
    size: ModelSize,
}

impl PreparedImage {
    pub fn load(path: &Path, size: ModelSize) -> ToolResult<Self> {
        let original = super::super::image_ops::load_oriented(path)
            .map_err(|error| ToolError::new(TOOL, error))?
            .into_rgba8();
        let chw = model_input(&original, size)?;
        Ok(Self {
            chw,
            original,
            size,
        })
    }

    /// Applies the model's mask as alpha (multiplying any alpha the picture
    /// had) and saves the cut-out as `options` say.
    pub fn write_with_mask(
        self,
        mask: &[f32],
        output: &Path,
        options: &OutputOptions,
    ) -> ToolResult<Written> {
        let alpha = picture_alpha(
            mask,
            self.size,
            self.original.width(),
            self.original.height(),
        )?;
        let mut rgba = self.original;
        let pixels: &mut [u8] = &mut rgba;
        pixels
            .par_chunks_exact_mut(4)
            .zip(alpha.par_iter())
            .for_each(|(pixel, &alpha)| {
                pixel[3] = ((u16::from(pixel[3]) * u16::from(alpha)) / 255) as u8;
            });
        let picture = if options.reshapes() {
            reshape(rgba, options)?
        } else {
            rgba
        };
        let bytes = match options.format_for(output) {
            Format::Png => encode_png(&picture, options.png)?,
            Format::WebP => encode_webp(&picture, options)?,
        };
        write_atomic(output, &bytes)?;
        Ok(Written {
            width: picture.width(),
            height: picture.height(),
            bytes: bytes.len() as u64,
        })
    }
}

/// The picture as the model's input: its colors scaled to the model's size and
/// normalized as ImageNet, one plane per channel.
fn model_input(original: &RgbaImage, size: ModelSize) -> ToolResult<Vec<f32>> {
    let source = ImageRef::new(
        original.width(),
        original.height(),
        original.as_raw(),
        PixelType::U8x4,
    )
    .map_err(resize_error)?;
    let mut scaled = ResizeImage::new(size.width, size.height, PixelType::U8x4);
    // Transparent pixels keep their colors, as when the picture is converted to
    // RGB before scaling.
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(INPUT_FILTER))
        .use_alpha(false);
    Resizer::new()
        .resize(&source, &mut scaled, &options)
        .map_err(resize_error)?;
    Ok(normalize_rgba(scaled.buffer()))
}

/// Planar ImageNet normalization of RGBA bytes (alpha ignored), through one
/// lookup table per channel.
fn normalize_rgba(rgba: &[u8]) -> Vec<f32> {
    let pixels = rgba.len() / 4;
    let tables: [[f32; 256]; 3] = std::array::from_fn(|channel| {
        std::array::from_fn(|value| (value as f32 / 255.0 - MEAN[channel]) / STD[channel])
    });
    let mut chw = vec![0.0; pixels * 3];
    chw.par_chunks_mut(pixels)
        .enumerate()
        .for_each(|(channel, plane)| {
            let table = &tables[channel];
            plane
                .par_chunks_mut(1 << 16)
                .enumerate()
                .for_each(|(chunk, values)| {
                    let start = chunk << 16;
                    for (offset, value) in values.iter_mut().enumerate() {
                        *value = table[usize::from(rgba[(start + offset) * 4 + channel])];
                    }
                });
        });
    chw
}

/// The mask as the picture's alpha: probabilities (or logits, which get the
/// sigmoid) scaled to the picture's size in floating point, then rounded.
fn picture_alpha(mask: &[f32], size: ModelSize, width: u32, height: u32) -> ToolResult<Vec<u8>> {
    let probabilities = mask_probabilities(mask, size)?;
    if (width, height) == (size.width, size.height) {
        return Ok(probabilities.iter().map(|&value| to_byte(value)).collect());
    }
    let source = TypedImage::<F32>::from_pixels(
        size.width,
        size.height,
        probabilities.into_iter().map(F32::new).collect(),
    )
    .map_err(|_| ToolError::new(TOOL, "model returned an invalid alpha mask"))?;
    let mut scaled = TypedImage::<F32>::new(width, height);
    Resizer::new()
        .resize_typed(
            &source,
            &mut scaled,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(MASK_FILTER)),
        )
        .map_err(resize_error)?;
    Ok(scaled
        .pixels()
        .par_iter()
        .map(|value| to_byte(value.0))
        .collect())
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The mask's values as probabilities in [0, 1]. Models that return logits
/// (values well outside [0, 1]) get the sigmoid.
fn mask_probabilities(mask: &[f32], size: ModelSize) -> ToolResult<Vec<f32>> {
    let expected = size.pixels();
    if mask.len() != expected {
        return Err(ToolError::new(
            TOOL,
            format!(
                "model returned {} alpha values; expected {expected}",
                mask.len()
            ),
        ));
    }
    let (min, max) = mask
        .par_iter()
        .fold(
            || (f32::INFINITY, f32::NEG_INFINITY),
            |(min, max), &value| (min.min(value), max.max(value)),
        )
        .reduce(
            || (f32::INFINITY, f32::NEG_INFINITY),
            |a, b| (a.0.min(b.0), a.1.max(b.1)),
        );
    let needs_sigmoid = min < -0.01 || max > 1.01;
    Ok(mask
        .par_iter()
        .map(|&value| {
            let probability = if needs_sigmoid {
                1.0 / (1.0 + (-value).exp())
            } else {
                value
            };
            probability.clamp(0.0, 1.0)
        })
        .collect())
}

/// The mask as bytes at the model's size.
#[cfg(test)]
fn mask_to_u8(mask: &[f32], size: ModelSize) -> ToolResult<Vec<u8>> {
    Ok(mask_probabilities(mask, size)?
        .into_iter()
        .map(to_byte)
        .collect())
}

/// Crops a cut-out to what is visible, adds the padding and scales it down to
/// fit, as `options` say.
fn reshape(rgba: RgbaImage, options: &OutputOptions) -> ToolResult<RgbaImage> {
    let (width, height) = rgba.dimensions();
    // Pillow's getbbox on the alpha channel: every pixel with any alpha. A cut-out
    // with nothing visible keeps its whole frame.
    let (left, top, right, bottom) = if options.crop {
        visible_box(&rgba).unwrap_or((0, 0, width, height))
    } else {
        (0, 0, width, height)
    };
    let padding = options.padding;
    let canvas_width = (right - left) + 2 * padding;
    let canvas_height = (bottom - top) + 2 * padding;
    let mut canvas = RgbaImage::new(canvas_width, canvas_height);
    let row_bytes = (right - left) as usize * 4;
    let source = rgba.as_raw();
    let lines: &mut [u8] = &mut canvas;
    lines
        .par_chunks_exact_mut(canvas_width as usize * 4)
        .skip(padding as usize)
        .take((bottom - top) as usize)
        .enumerate()
        .for_each(|(row, line)| {
            let from = ((top as usize + row) * width as usize + left as usize) * 4;
            let to = padding as usize * 4;
            line[to..to + row_bytes].copy_from_slice(&source[from..from + row_bytes]);
        });

    let scale = [
        options
            .max_width
            .map(|max| f64::from(max) / f64::from(canvas_width)),
        options
            .max_height
            .map(|max| f64::from(max) / f64::from(canvas_height)),
    ]
    .into_iter()
    .flatten()
    .fold(1.0_f64, f64::min);
    if scale >= 1.0 {
        return Ok(canvas);
    }
    let scaled_width = ((f64::from(canvas_width) * scale).round() as u32).max(1);
    let scaled_height = ((f64::from(canvas_height) * scale).round() as u32).max(1);
    let source = ImageRef::new(
        canvas_width,
        canvas_height,
        canvas.as_raw(),
        PixelType::U8x4,
    )
    .map_err(resize_error)?;
    let mut scaled = ResizeImage::new(scaled_width, scaled_height, PixelType::U8x4);
    // Colors are weighted by alpha while scaling, so transparent pixels' colors
    // never bleed into the edge.
    Resizer::new()
        .resize(
            &source,
            &mut scaled,
            &ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(PICTURE_FILTER))
                .use_alpha(true),
        )
        .map_err(resize_error)?;
    RgbaImage::from_raw(scaled_width, scaled_height, scaled.into_vec())
        .ok_or_else(|| ToolError::new(TOOL, "scaled picture has the wrong size"))
}

/// The smallest box (left, top, right, bottom; right and bottom exclusive) that
/// holds every pixel with any alpha, or `None` when nothing is visible.
fn visible_box(rgba: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let width = rgba.width() as usize;
    rgba.as_raw()
        .par_chunks_exact(width * 4)
        .enumerate()
        .filter_map(|(y, row)| {
            let mut columns = row
                .as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                .filter(|(_, p)| p[3] > 0);
            let first = columns.next()?.0;
            let last = columns.next_back().map_or(first, |(x, _)| x);
            Some((first, y, last, y))
        })
        .reduce_with(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        .map(|(left, top, right, bottom)| {
            (left as u32, top as u32, right as u32 + 1, bottom as u32 + 1)
        })
}

fn encode_png(rgba: &RgbaImage, compression: PngCompression) -> ToolResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let compression = match compression {
        PngCompression::Fast => CompressionType::Fast,
        PngCompression::Small => CompressionType::Default,
    };
    PngEncoder::new_with_quality(&mut bytes, compression, PngFilterType::Adaptive)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| ToolError::new(TOOL, format!("cannot encode PNG: {error}")))?;
    Ok(bytes)
}

/// Lossy WebP with alpha, as Pillow writes it (quality, alpha quality, method).
/// Quantizing alpha slightly (alpha quality below 100) moves it by at most a
/// level and encodes about five times faster than lossless alpha.
fn encode_webp(rgba: &RgbaImage, options: &OutputOptions) -> ToolResult<Vec<u8>> {
    let mut config = webp::WebPConfig::new()
        .map_err(|()| ToolError::new(TOOL, "cannot set up the WebP encoder"))?;
    config.lossless = 0;
    config.quality = f32::from(options.quality.min(100));
    config.alpha_quality = i32::from(options.alpha_quality.min(100));
    config.method = i32::from(options.method.min(6));
    let encoded = webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
        .encode_advanced(&config)
        .map_err(|error| ToolError::new(TOOL, format!("cannot encode WebP: {error:?}")))?;
    Ok(encoded.to_vec())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> ToolResult {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| ToolError::new(TOOL, format!("cannot stage output: {error}")))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    file.commit()
        .map_err(|error| ToolError::new(TOOL, format!("cannot commit output: {error}")))
}

fn resize_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::new(TOOL, format!("cannot scale the picture: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_planar_imagenet() {
        let got = normalize_rgba(&[255, 0, 127, 9, 0, 255, 128, 9]);
        assert_eq!(got.len(), 6);
        assert!((got[0] - ((1.0 - MEAN[0]) / STD[0])).abs() < 1e-6);
        assert!((got[2] - ((0.0 - MEAN[1]) / STD[1])).abs() < 1e-6);
        assert!((got[5] - ((128.0 / 255.0 - MEAN[2]) / STD[2])).abs() < 1e-6);
    }

    #[test]
    fn converts_logits_when_needed() {
        let mut input = vec![0.5; ModelSize::DEFAULT.pixels()];
        input[0] = -1.0;
        input[1] = 2.0;
        let got = mask_to_u8(&input, ModelSize::DEFAULT).unwrap();
        assert_eq!(&got[..3], &[69, 225, 159]);
    }

    #[test]
    fn a_fixed_size_model_gets_its_own_size() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("in.png");
        RgbaImage::from_pixel(10, 6, image::Rgba([200, 100, 50, 255]))
            .save(&input)
            .unwrap();
        let size = ModelSize {
            width: 8,
            height: 4,
        };
        let prepared = PreparedImage::load(&input, size).unwrap();
        assert_eq!(prepared.chw.len(), 3 * 8 * 4);
        assert!(mask_to_u8(&[1.0; 32], ModelSize::DEFAULT).is_err());
        let output = directory.path().join("out.png");
        let written = prepared
            .write_with_mask(&[1.0; 32], &output, &OutputOptions::default())
            .unwrap();
        assert_eq!((written.width, written.height), (10, 6));
        let saved = image::open(&output).unwrap().to_rgba8();
        assert_eq!(saved.dimensions(), (10, 6));
        assert_eq!(saved.get_pixel(5, 3).0, [200, 100, 50, 255]);
        assert_eq!(written.bytes, fs::metadata(&output).unwrap().len());
    }

    /// A 40 x 20 picture whose mask keeps only the box from (10, 4) to (19, 13).
    fn boxed() -> (tempfile::TempDir, PreparedImage, Vec<f32>) {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("in.png");
        RgbaImage::from_pixel(40, 20, image::Rgba([10, 200, 30, 255]))
            .save(&input)
            .unwrap();
        let size = ModelSize {
            width: 40,
            height: 20,
        };
        let prepared = PreparedImage::load(&input, size).unwrap();
        let mut mask = vec![0.0; 40 * 20];
        for y in 4..14 {
            for x in 10..20 {
                mask[y * 40 + x] = 1.0;
            }
        }
        (directory, prepared, mask)
    }

    #[test]
    fn a_picture_is_cropped_to_the_subject_padded_and_scaled_down() {
        let (directory, prepared, mask) = boxed();
        let output = directory.path().join("picture.webp");
        let options = OutputOptions {
            padding: 5,
            max_height: Some(10),
            ..OutputOptions::picture()
        };
        let written = prepared.write_with_mask(&mask, &output, &options).unwrap();
        // 10 x 10 visible plus 5 px all around is 20 x 20, scaled to 10 tall
        assert_eq!((written.width, written.height), (10, 10));
        let bytes = fs::read(&output).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (10, 10));
        assert!(
            decoded.get_pixel(5, 5).0[3] > 200,
            "the subject stays opaque"
        );
        assert!(
            decoded.get_pixel(0, 0).0[3] < 30,
            "the padding is transparent"
        );
    }

    #[test]
    fn cropping_keeps_full_size_without_limits_and_never_scales_up() {
        let (directory, prepared, mask) = boxed();
        let output = directory.path().join("cropped.png");
        let options = OutputOptions {
            crop: true,
            padding: 2,
            max_height: Some(400),
            ..OutputOptions::default()
        };
        let written = prepared.write_with_mask(&mask, &output, &options).unwrap();
        assert_eq!((written.width, written.height), (14, 14));
        let saved = image::open(&output).unwrap().to_rgba8();
        assert_eq!(saved.get_pixel(2, 2).0, [10, 200, 30, 255]);
        assert_eq!(saved.get_pixel(1, 1).0[3], 0);
    }

    #[test]
    fn nothing_visible_keeps_the_whole_frame() {
        let (directory, prepared, _) = boxed();
        let output = directory.path().join("empty.png");
        let options = OutputOptions {
            crop: true,
            ..OutputOptions::default()
        };
        let written = prepared
            .write_with_mask(&vec![0.0; 40 * 20], &output, &options)
            .unwrap();
        assert_eq!((written.width, written.height), (40, 20));
    }

    #[test]
    fn the_format_comes_from_the_option_then_the_extension() {
        let options = OutputOptions::default();
        assert_eq!(options.format_for(Path::new("a.webp")), Format::WebP);
        assert_eq!(options.format_for(Path::new("a.WEBP")), Format::WebP);
        assert_eq!(options.format_for(Path::new("a.png")), Format::Png);
        assert_eq!(options.format_for(Path::new("a")), Format::Png);
        let webp = OutputOptions {
            format: Some(Format::WebP),
            ..OutputOptions::default()
        };
        assert_eq!(webp.format_for(Path::new("a.png")), Format::WebP);
    }

    #[test]
    fn output_replacement_is_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("out.png");
        fs::write(&path, b"old").unwrap();
        let rgba = RgbaImage::from_pixel(2, 1, image::Rgba([1, 2, 3, 4]));
        for compression in [PngCompression::Small, PngCompression::Fast] {
            write_atomic(&path, &encode_png(&rgba, compression).unwrap()).unwrap();
            assert_eq!(image::open(&path).unwrap().to_rgba8(), rgba);
        }
    }
}
