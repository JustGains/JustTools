//! `justpdf images`: save embedded images without needless re-encoding.
//!
//! JPEG and JPEG 2000 streams are copied byte-for-byte unless a transparency
//! mask would be lost; masked JPEGs and raw pixel images are written as PNG.

use super::{
    Cli, confirm_outputs, dictionary, document_stem, inherited_attribute, load_pdf, pdf_number,
    resolve_object, selected_pages,
};
use crate::common::{
    absolute_lexical, atomic_write, atomic_write_with, display_path, format_bytes,
};
use anyhow::{Context, Result, anyhow, bail};
use image::codecs::jpeg::JpegDecoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ExtendedColorType, GrayImage, ImageDecoder, ImageEncoder, imageops};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Cursor, Write};
use std::path::{Path, PathBuf};

/// Largest decoded buffer accepted for one image or mask.
const MAX_DECODED_BYTES: usize = 1 << 30;
/// Nesting limit for forms, patterns, and color spaces.
const MAX_DEPTH: usize = 32;
/// Filters that produce a complete image rather than raw samples.
const CODECS: [&[u8]; 4] = [
    b"DCTDecode",
    b"JPXDecode",
    b"JBIG2Decode",
    b"CCITTFaxDecode",
];
/// General-purpose filters that lopdf decodes.
const TRANSPORTS: [&[u8]; 3] = [b"FlateDecode", b"LZWDecode", b"ASCII85Decode"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Render {
    /// The stored stream is already a complete file with this extension.
    Copy(&'static str),
    /// Pixels are decoded and written losslessly as PNG.
    Png,
}

impl Render {
    fn extension(self) -> &'static str {
        match self {
            Self::Copy(extension) => extension,
            Self::Png => "png",
        }
    }
}

struct Planned {
    id: ObjectId,
    render: Render,
    size: (u32, u32),
    path: PathBuf,
}

struct Filter {
    name: Vec<u8>,
    params: Option<Dictionary>,
}

#[derive(Clone, Debug, PartialEq)]
enum Color {
    Gray,
    Rgb,
    Cmyk,
    Indexed {
        base: Box<Color>,
        high: usize,
        palette: Vec<u8>,
    },
}

impl Color {
    fn components(&self) -> usize {
        match self {
            Self::Gray | Self::Indexed { .. } => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }
}

/// Decoded 8-bit gray or RGB pixels with optional straight alpha.
struct Picture {
    width: u32,
    height: u32,
    channels: usize,
    pixels: Vec<u8>,
    alpha: Option<Vec<u8>>,
    icc: Option<Vec<u8>>,
}

pub(super) fn run(input: &Path, options: &Cli) -> Result<()> {
    let document = load_pdf(input)?;
    let pages = document.get_pages();
    let selected = selected_pages(options.pages.as_deref().unwrap_or("all"), pages.len())?;
    let default_directory = input.with_file_name(format!("{}-images", document_stem(input)));
    let directory = absolute_lexical(options.output.as_deref().unwrap_or(&default_directory))?;
    if directory.is_file() {
        bail!(
            "images output must be a folder: {}",
            display_path(&directory)
        );
    }

    let width = std::cmp::max(3, pages.len().to_string().len());
    let mut visited = HashSet::new();
    let mut planned = Vec::new();
    let mut skipped = 0usize;
    for page in selected {
        let mut found = Vec::new();
        collect_page(&document, pages[&page], &mut visited, &mut found);
        for (index, id) in found.into_iter().enumerate() {
            let name = format!("p{page:0width$}-{:02}", index + 1);
            match plan(&document, id) {
                Ok((render, size)) => {
                    let path = directory.join(format!("{name}.{}", render.extension()));
                    if render == Render::Copy("jp2") && has_mask(&document, id) {
                        eprintln!(
                            "justpdf: {name}.jp2: JPEG 2000 is copied unchanged; its transparency mask is not applied"
                        );
                    }
                    planned.push(Planned {
                        id,
                        render,
                        size,
                        path,
                    });
                }
                Err(error) => {
                    skipped += 1;
                    eprintln!("justpdf: {name}: skipped: {error:#}");
                }
            }
        }
    }

    let skipped_note = if skipped > 0 {
        format!("; {skipped} unsupported image(s) skipped")
    } else {
        String::new()
    };
    if planned.is_empty() {
        println!("justpdf: no extractable images found{skipped_note}");
        return Ok(());
    }
    if options.dry_run {
        println!(
            "justpdf: dry run — {} image(s) -> {}{skipped_note}",
            planned.len(),
            display_path(&directory)
        );
        for item in &planned {
            let file = item.path.file_name().unwrap_or_default().to_string_lossy();
            println!("  {file}  {}×{}", item.size.0, item.size.1);
        }
        return Ok(());
    }

    let outputs: Vec<_> = planned.iter().map(|item| item.path.clone()).collect();
    confirm_outputs(&outputs, options.yes, false)?;
    let mut written = 0u64;
    let mut saved = [0usize; 3];
    let mut failed = 0usize;
    for item in &planned {
        match save(&document, item) {
            Ok(bytes) => {
                written += bytes;
                saved[match item.render {
                    Render::Copy("jpg") => 0,
                    Render::Copy(_) => 1,
                    Render::Png => 2,
                }] += 1;
            }
            Err(error) => {
                failed += 1;
                eprintln!("justpdf: {}: {error:#}", display_path(&item.path));
            }
        }
    }
    let formats: Vec<_> = saved
        .iter()
        .zip(["original JPEG", "original JPEG 2000", "PNG"])
        .filter(|(count, _)| **count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect();
    println!(
        "justpdf: saved {} image(s) -> {} ({}; {}){skipped_note}",
        saved.iter().sum::<usize>(),
        display_path(&directory),
        format_bytes(written),
        formats.join(", ")
    );
    if failed > 0 {
        bail!("{failed} image(s) could not be saved");
    }
    Ok(())
}

/// Appends image XObjects reachable from one page — through forms, tiling
/// patterns, and annotation appearances — that no earlier page reached.
fn collect_page(
    document: &Document,
    page_id: ObjectId,
    visited: &mut HashSet<ObjectId>,
    found: &mut Vec<ObjectId>,
) {
    if let Some(resources) = inherited_attribute(document, page_id, b"Resources") {
        collect_resources(document, &resources, visited, found, 0);
    }
    for annotation in document.get_page_annotations(page_id).unwrap_or_default() {
        let Some(normal) = annotation
            .get(b"AP")
            .ok()
            .and_then(|appearance| dictionary(document, appearance))
            .and_then(|appearance| appearance.get(b"N").ok())
        else {
            continue;
        };
        match normal {
            Object::Reference(id) if matches!(document.get_object(*id), Ok(Object::Stream(_))) => {
                collect_xobject(document, *id, visited, found, 0);
            }
            states => {
                for (_, state) in dictionary(document, states)
                    .into_iter()
                    .flat_map(Dictionary::iter)
                {
                    if let Ok(id) = state.as_reference() {
                        collect_xobject(document, id, visited, found, 0);
                    }
                }
            }
        }
    }
}

fn collect_resources(
    document: &Document,
    resources: &Object,
    visited: &mut HashSet<ObjectId>,
    found: &mut Vec<ObjectId>,
    depth: usize,
) {
    let Some(resources) = dictionary(document, resources).filter(|_| depth < MAX_DEPTH) else {
        return;
    };
    for key in [b"XObject".as_slice(), b"Pattern"] {
        let Some(entries) = resources
            .get(key)
            .ok()
            .and_then(|entries| dictionary(document, entries))
        else {
            continue;
        };
        for (_, entry) in entries.iter() {
            if let Ok(id) = entry.as_reference() {
                collect_xobject(document, id, visited, found, depth + 1);
            }
        }
    }
}

fn collect_xobject(
    document: &Document,
    id: ObjectId,
    visited: &mut HashSet<ObjectId>,
    found: &mut Vec<ObjectId>,
    depth: usize,
) {
    if !visited.insert(id) {
        return;
    }
    let Ok(Object::Stream(stream)) = document.get_object(id) else {
        return;
    };
    if name_entry(&stream.dict, b"Subtype") == Some(b"Image") {
        found.push(id);
    } else if let Ok(resources) = stream.dict.get(b"Resources") {
        // Form XObjects and tiling patterns carry their own resources.
        collect_resources(document, resources, visited, found, depth);
    }
}

fn plan(document: &Document, id: ObjectId) -> Result<(Render, (u32, u32))> {
    let stream = image_stream(document, id)?;
    let size = dimensions(&stream.dict)?;
    let filters = filters(document, &stream.dict)?;
    let codec = codec(&filters);
    let transports = &filters[..filters.len() - usize::from(codec.is_some())];
    if let Some(filter) = transports
        .iter()
        .find(|filter| !TRANSPORTS.contains(&filter.name.as_slice()))
    {
        bail!(
            "{} data is not supported",
            String::from_utf8_lossy(&filter.name)
        );
    }
    let render = match codec {
        Some(b"DCTDecode") => {
            if mask_alpha(document, &stream.dict, size.0, size.1)?.is_some() {
                Render::Png
            } else {
                Render::Copy("jpg")
            }
        }
        Some(b"JPXDecode") => Render::Copy("jp2"),
        Some(codec) => bail!(
            "{} images are not supported",
            String::from_utf8_lossy(codec)
        ),
        None => {
            if !is_stencil(&stream.dict) {
                let space = stream
                    .dict
                    .get(b"ColorSpace")
                    .map_err(|_| anyhow!("image has no color space"))?;
                color_space(document, space, 0)?;
                bits_per_component(&stream.dict)?;
            }
            Render::Png
        }
    };
    Ok((render, size))
}

fn save(document: &Document, item: &Planned) -> Result<u64> {
    let stream = image_stream(document, item.id)?;
    let filters = filters(document, &stream.dict)?;
    match item.render {
        Render::Copy(_) => atomic_write(&item.path, &payload(stream, &filters)?)?,
        Render::Png => {
            let mut picture = if codec(&filters).is_some() {
                let mut picture = decode_jpeg(&payload(stream, &filters)?)?;
                // The PDF color space is what viewers honor; the JPEG's own
                // profile is only a fallback.
                let profile = stream
                    .dict
                    .get(b"ColorSpace")
                    .ok()
                    .and_then(|space| color_space(document, space, 0).ok())
                    .and_then(|(_, profile)| profile);
                picture.icc = profile.or(picture.icc.take());
                picture
            } else {
                decode_raw(document, stream, &filters, item.size)?
            };
            if let Some(alpha) = mask_alpha(document, &stream.dict, picture.width, picture.height)?
            {
                picture.alpha = Some(alpha);
            }
            picture.icc = matching_profile(picture.icc.take(), picture.channels);
            atomic_write_with(&item.path, |file| write_png(file, &picture))?;
        }
    }
    Ok(fs::metadata(&item.path)?.len())
}

fn image_stream(document: &Document, id: ObjectId) -> Result<&Stream> {
    document
        .get_object(id)?
        .as_stream()
        .map_err(|_| anyhow!("image object is not a stream"))
}

fn has_mask(document: &Document, id: ObjectId) -> bool {
    image_stream(document, id)
        .is_ok_and(|stream| stream.dict.has(b"SMask") || stream.dict.has(b"Mask"))
}

fn name_entry<'a>(dictionary: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    dictionary.get(key).and_then(Object::as_name).ok()
}

fn is_stencil(dictionary: &Dictionary) -> bool {
    dictionary
        .get(b"ImageMask")
        .and_then(Object::as_bool)
        .unwrap_or(false)
}

fn dimensions(dictionary: &Dictionary) -> Result<(u32, u32)> {
    let read = |key: &[u8]| {
        dictionary
            .get(key)
            .and_then(Object::as_i64)
            .ok()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    match (read(b"Width"), read(b"Height")) {
        (Some(width), Some(height)) => Ok((width, height)),
        _ => bail!("image has no valid size"),
    }
}

fn bits_per_component(dictionary: &Dictionary) -> Result<usize> {
    dictionary
        .get(b"BitsPerComponent")
        .and_then(Object::as_i64)
        .ok()
        .and_then(|bits| usize::try_from(bits).ok())
        .filter(|bits| matches!(bits, 1 | 2 | 4 | 8 | 16))
        .ok_or_else(|| anyhow!("unsupported bits per component"))
}

fn filters(document: &Document, dictionary: &Dictionary) -> Result<Vec<Filter>> {
    let resolve = |object| resolve_object(document, object).unwrap_or(object);
    let names = match dictionary.get(b"Filter").map(resolve) {
        Err(_) => Vec::new(),
        Ok(Object::Name(name)) => vec![name.clone()],
        Ok(Object::Array(names)) => names
            .iter()
            .map(|name| resolve(name).as_name().map(<[u8]>::to_vec))
            .collect::<lopdf::Result<_>>()?,
        Ok(_) => bail!("invalid image filter"),
    };
    let params: Vec<Option<Dictionary>> = match dictionary.get(b"DecodeParms").map(resolve) {
        Ok(Object::Dictionary(params)) => vec![Some(params.clone())],
        Ok(Object::Array(params)) => params
            .iter()
            .map(|params| resolve(params).as_dict().ok().cloned())
            .collect(),
        _ => Vec::new(),
    };
    Ok(names
        .into_iter()
        .enumerate()
        .map(|(index, name)| Filter {
            name,
            params: params.get(index).cloned().flatten(),
        })
        .collect())
}

/// The final filter when it yields a complete encoded image.
fn codec(filters: &[Filter]) -> Option<&[u8]> {
    filters
        .last()
        .map(|filter| filter.name.as_slice())
        .filter(|name| CODECS.contains(name))
}

/// Applies general-purpose filters such as FlateDecode, bounded by `limit`.
fn decode_filters(content: &[u8], filters: &[Filter], limit: usize) -> Result<Vec<u8>> {
    let mut data = content.to_vec();
    for filter in filters {
        let mut dictionary = Dictionary::new();
        dictionary.set("Filter", Object::Name(filter.name.clone()));
        if let Some(params) = &filter.params {
            dictionary.set("DecodeParms", params.clone());
        }
        data = Stream::new(dictionary, data)
            .decompressed_content_with_limit(limit)
            .map_err(|error| {
                anyhow!(
                    "{} data could not be decoded: {error}",
                    String::from_utf8_lossy(&filter.name)
                )
            })?;
    }
    Ok(data)
}

/// The encoded JPEG or JPEG 2000 file inside a codec-filtered stream.
fn payload(stream: &Stream, filters: &[Filter]) -> Result<Vec<u8>> {
    let transport = &filters[..filters.len().saturating_sub(1)];
    decode_filters(&stream.content, transport, MAX_DECODED_BYTES)
}

/// Decodes raw samples, one `u16` per component, from byte-aligned rows.
fn samples(
    stream: &Stream,
    filters: &[Filter],
    (width, height): (u32, u32),
    components: usize,
    bits: usize,
) -> Result<Vec<u16>> {
    let (width, height) = (width as usize, height as usize);
    let row = width
        .checked_mul(components * bits)
        .map(|row_bits| row_bits.div_ceil(8))
        .ok_or_else(|| anyhow!("image is too large"))?;
    let expected = row
        .checked_mul(height)
        .filter(|bytes| *bytes <= MAX_DECODED_BYTES)
        .ok_or_else(|| anyhow!("image is too large"))?;
    let limit = expected
        .saturating_mul(2)
        .saturating_add(1 << 16)
        .min(MAX_DECODED_BYTES);
    let data = decode_filters(&stream.content, filters, limit)?;
    if data.len() < expected {
        bail!("image data is truncated");
    }
    Ok(unpack(&data, row, width * components, height, bits))
}

fn unpack(data: &[u8], row: usize, per_row: usize, height: usize, bits: usize) -> Vec<u16> {
    let mut samples = Vec::with_capacity(per_row * height);
    for row in data.chunks_exact(row).take(height) {
        match bits {
            8 => samples.extend(row[..per_row].iter().map(|byte| u16::from(*byte))),
            16 => samples.extend(
                row.chunks_exact(2)
                    .take(per_row)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
            ),
            _ => {
                let per_byte = 8 / bits;
                let mask = (1u16 << bits) - 1;
                samples.extend((0..per_row).map(|index| {
                    let shift = 8 - bits * (index % per_byte + 1);
                    (u16::from(row[index / per_byte]) >> shift) & mask
                }));
            }
        }
    }
    samples
}

/// Per-component /Decode ranges, defaulting to `[0 default_max]`.
fn decode_ranges(dictionary: &Dictionary, components: usize, default_max: f64) -> Vec<(f64, f64)> {
    let values: Vec<f64> = dictionary
        .get(b"Decode")
        .and_then(Object::as_array)
        .map(|values| values.iter().filter_map(pdf_number).collect())
        .unwrap_or_default();
    (0..components)
        .map(|index| match values.get(index * 2..index * 2 + 2) {
            Some([low, high]) => (*low, *high),
            _ => (0.0, default_max),
        })
        .collect()
}

fn color_space(
    document: &Document,
    space: &Object,
    depth: usize,
) -> Result<(Color, Option<Vec<u8>>)> {
    if depth > MAX_DEPTH {
        bail!("color space nesting is too deep");
    }
    let (family, operands) = match resolve_object(document, space)? {
        Object::Name(name) => (name.as_slice(), &[][..]),
        Object::Array(items) if !items.is_empty() => (items[0].as_name()?, &items[1..]),
        _ => bail!("invalid color space"),
    };
    Ok(match family {
        b"DeviceGray" | b"CalGray" => (Color::Gray, None),
        b"DeviceRGB" | b"CalRGB" => (Color::Rgb, None),
        b"DeviceCMYK" => (Color::Cmyk, None),
        b"ICCBased" => {
            let stream = operands
                .first()
                .and_then(|profile| resolve_object(document, profile).ok())
                .and_then(|profile| profile.as_stream().ok())
                .ok_or_else(|| anyhow!("invalid ICC color space"))?;
            let color = match stream.dict.get(b"N").and_then(Object::as_i64) {
                Ok(1) => Color::Gray,
                Ok(3) => Color::Rgb,
                Ok(4) => Color::Cmyk,
                _ => bail!("unsupported ICC component count"),
            };
            (color, stream.get_plain_content_with_limit(16 << 20).ok())
        }
        b"Indexed" => {
            let [base, high, lookup] = operands else {
                bail!("invalid indexed color space");
            };
            let (base, _) = color_space(document, base, depth + 1)?;
            if matches!(base, Color::Indexed { .. }) {
                bail!("nested indexed color spaces are invalid");
            }
            let high = resolve_object(document, high)?
                .as_i64()
                .ok()
                .and_then(|high| usize::try_from(high).ok())
                .filter(|high| *high <= 255)
                .ok_or_else(|| anyhow!("invalid indexed color range"))?;
            let mut palette = match resolve_object(document, lookup)? {
                Object::String(bytes, _) => bytes.clone(),
                Object::Stream(stream) => stream.get_plain_content_with_limit(1 << 16)?,
                _ => bail!("invalid indexed palette"),
            };
            palette.resize((high + 1) * base.components(), 0);
            (
                Color::Indexed {
                    base: Box::new(base),
                    high,
                    palette,
                },
                None,
            )
        }
        other => bail!("{} color is not supported", String::from_utf8_lossy(other)),
    })
}

fn decode_raw(
    document: &Document,
    stream: &Stream,
    filters: &[Filter],
    size: (u32, u32),
) -> Result<Picture> {
    let dictionary = &stream.dict;
    let (color, icc, bits) = if is_stencil(dictionary) {
        (Color::Gray, None, 1)
    } else {
        let space = dictionary
            .get(b"ColorSpace")
            .map_err(|_| anyhow!("image has no color space"))?;
        let (color, icc) = color_space(document, space, 0)?;
        (color, icc, bits_per_component(dictionary)?)
    };
    let components = color.components();
    let samples = samples(stream, filters, size, components, bits)?;
    let max = f64::from((1u32 << bits) - 1);
    let (channels, pixels) = match &color {
        Color::Indexed {
            base,
            high,
            palette,
        } => {
            let (low, top) = decode_ranges(dictionary, 1, max)[0];
            let stride = base.components();
            let mut values = Vec::with_capacity(samples.len() * stride);
            for sample in &samples {
                let index = (low + f64::from(*sample) * (top - low) / max)
                    .round()
                    .clamp(0.0, *high as f64) as usize;
                values.extend_from_slice(&palette[index * stride..(index + 1) * stride]);
            }
            to_gray_or_rgb(base, values)
        }
        _ => {
            let ranges = decode_ranges(dictionary, components, 1.0);
            let values = samples
                .iter()
                .zip(ranges.iter().cycle())
                .map(|(sample, (low, high))| {
                    let value = low + f64::from(*sample) * (high - low) / max;
                    (value.clamp(0.0, 1.0) * 255.0).round() as u8
                })
                .collect();
            to_gray_or_rgb(&color, values)
        }
    };
    Ok(Picture {
        width: size.0,
        height: size.1,
        channels,
        pixels,
        alpha: color_key(document, dictionary, &samples, components),
        icc,
    })
}

fn to_gray_or_rgb(color: &Color, values: Vec<u8>) -> (usize, Vec<u8>) {
    match color {
        Color::Gray => (1, values),
        Color::Cmyk => (3, values.chunks_exact(4).flat_map(cmyk_to_rgb).collect()),
        _ => (3, values),
    }
}

/// Naive device CMYK conversion; ICC-accurate conversion is out of scope.
fn cmyk_to_rgb(cmyk: &[u8]) -> [u8; 3] {
    let white = 255 - u16::from(cmyk[3]);
    [0, 1, 2].map(|index| ((255 - u16::from(cmyk[index])) * white / 255) as u8)
}

/// Alpha from a /Mask color-key array, or `None` when nothing is keyed out.
fn color_key(
    document: &Document,
    dictionary: &Dictionary,
    samples: &[u16],
    components: usize,
) -> Option<Vec<u8>> {
    if dictionary.has(b"SMask") {
        return None;
    }
    let ranges: Vec<i64> = resolve_object(document, dictionary.get(b"Mask").ok()?)
        .ok()?
        .as_array()
        .ok()?
        .iter()
        .map(Object::as_i64)
        .collect::<lopdf::Result<_>>()
        .ok()?;
    if ranges.len() != components * 2 {
        return None;
    }
    let alpha: Vec<u8> = samples
        .chunks_exact(components)
        .map(|pixel| {
            let keyed = pixel
                .iter()
                .zip(ranges.chunks_exact(2))
                .all(|(sample, range)| (range[0]..=range[1]).contains(&i64::from(*sample)));
            if keyed { 0 } else { 255 }
        })
        .collect();
    alpha.iter().any(|value| *value < 255).then_some(alpha)
}

/// The image's /SMask or stencil /Mask as 8-bit alpha scaled to the image,
/// or `None` when there is no mask or it is fully opaque.
fn mask_alpha(
    document: &Document,
    dictionary: &Dictionary,
    width: u32,
    height: u32,
) -> Result<Option<Vec<u8>>> {
    let (mask, soft) = match (dictionary.get(b"SMask"), dictionary.get(b"Mask")) {
        (Ok(mask), _) => (mask, true),
        (Err(_), Ok(mask)) => (mask, false),
        _ => return Ok(None),
    };
    // Color-key arrays are applied with the samples in `decode_raw`.
    let Ok(Object::Stream(mask)) = resolve_object(document, mask) else {
        return Ok(None);
    };
    let filters = filters(document, &mask.dict)?;
    let (mask_width, mask_height, plane) = match codec(&filters) {
        Some(b"DCTDecode") => {
            let picture = decode_jpeg(&payload(mask, &filters)?)?;
            let plane = if picture.channels == 1 {
                picture.pixels
            } else {
                picture
                    .pixels
                    .chunks_exact(3)
                    .map(|pixel| pixel[0])
                    .collect()
            };
            (picture.width, picture.height, plane)
        }
        Some(codec) => bail!("{} masks are not supported", String::from_utf8_lossy(codec)),
        None => {
            let size = dimensions(&mask.dict)?;
            let bits = if soft {
                bits_per_component(&mask.dict)?
            } else {
                1
            };
            let max = f64::from((1u32 << bits) - 1);
            let (low, high) = decode_ranges(&mask.dict, 1, 1.0)[0];
            let plane = samples(mask, &filters, size, 1, bits)?
                .iter()
                .map(|sample| {
                    let value = (low + f64::from(*sample) * (high - low) / max).clamp(0.0, 1.0);
                    // Soft-mask gray is opacity; a stencil value of 1 hides the pixel.
                    if soft {
                        (value * 255.0).round() as u8
                    } else if value < 0.5 {
                        255
                    } else {
                        0
                    }
                })
                .collect();
            (size.0, size.1, plane)
        }
    };
    if plane.iter().all(|value| *value == 255) {
        return Ok(None);
    }
    if (mask_width, mask_height) == (width, height) {
        return Ok(Some(plane));
    }
    let mask = GrayImage::from_raw(mask_width, mask_height, plane)
        .ok_or_else(|| anyhow!("mask data is truncated"))?;
    let filter = if soft {
        imageops::FilterType::Triangle
    } else {
        imageops::FilterType::Nearest
    };
    Ok(Some(
        imageops::resize(&mask, width, height, filter).into_raw(),
    ))
}

fn decode_jpeg(bytes: &[u8]) -> Result<Picture> {
    let mut decoder = JpegDecoder::new(Cursor::new(bytes)).context("could not read JPEG")?;
    let icc = decoder.icc_profile().ok().flatten();
    let image = DynamicImage::from_decoder(decoder).context("could not decode JPEG")?;
    let (width, height) = (image.width(), image.height());
    let (channels, pixels) = if image.color().has_color() {
        (3, image.into_rgb8().into_raw())
    } else {
        (1, image.into_luma8().into_raw())
    };
    Ok(Picture {
        width,
        height,
        channels,
        pixels,
        alpha: None,
        icc,
    })
}

/// Keeps an ICC profile only when its header matches the written color model.
fn matching_profile(profile: Option<Vec<u8>>, channels: usize) -> Option<Vec<u8>> {
    let expected: &[u8] = if channels == 1 { b"GRAY" } else { b"RGB " };
    profile.filter(|profile| profile.get(16..20) == Some(expected))
}

fn write_png(file: &mut File, picture: &Picture) -> Result<()> {
    let (pixels, color) = interleave(picture);
    let mut writer = BufWriter::new(file);
    let mut encoder =
        PngEncoder::new_with_quality(&mut writer, CompressionType::Default, FilterType::Adaptive);
    if let Some(profile) = &picture.icc {
        encoder.set_icc_profile(profile.clone()).ok();
    }
    encoder
        .write_image(&pixels, picture.width, picture.height, color)
        .context("could not encode PNG")?;
    writer.flush().context("could not write PNG")?;
    Ok(())
}

fn interleave(picture: &Picture) -> (Cow<'_, [u8]>, ExtendedColorType) {
    let gray = picture.channels == 1;
    let Some(alpha) = &picture.alpha else {
        let color = if gray {
            ExtendedColorType::L8
        } else {
            ExtendedColorType::Rgb8
        };
        return (Cow::Borrowed(&picture.pixels), color);
    };
    let mut pixels = Vec::with_capacity(alpha.len() * (picture.channels + 1));
    for (color, opacity) in picture.pixels.chunks_exact(picture.channels).zip(alpha) {
        // Fully transparent pixels are invisible; clearing them keeps PNGs compact.
        if *opacity == 0 {
            pixels.extend(std::iter::repeat_n(0, picture.channels));
        } else {
            pixels.extend_from_slice(color);
        }
        pixels.push(*opacity);
    }
    let color = if gray {
        ExtendedColorType::La8
    } else {
        ExtendedColorType::Rgba8
    };
    (Cow::Owned(pixels), color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_sub_byte_and_sixteen_bit_rows() {
        // Two 3-pixel rows of 2-bit samples; each row pads to one byte.
        assert_eq!(
            unpack(&[0b0001_1011, 0b1110_0100], 1, 3, 2, 2),
            [0, 1, 2, 3, 2, 1]
        );
        assert_eq!(
            unpack(&[0x12, 0x34, 0xff, 0xff], 4, 2, 1, 16),
            [0x1234, 0xffff]
        );
    }

    #[test]
    fn converts_device_cmyk_to_rgb() {
        assert_eq!(cmyk_to_rgb(&[0, 0, 0, 0]), [255, 255, 255]);
        assert_eq!(cmyk_to_rgb(&[255, 0, 0, 0]), [0, 255, 255]);
        assert_eq!(cmyk_to_rgb(&[0, 0, 0, 255]), [0, 0, 0]);
    }

    #[test]
    fn decode_arrays_default_and_invert() {
        let mut dictionary = Dictionary::new();
        assert_eq!(decode_ranges(&dictionary, 2, 1.0), [(0.0, 1.0), (0.0, 1.0)]);
        dictionary.set("Decode", vec![1.into(), 0.into()]);
        assert_eq!(decode_ranges(&dictionary, 1, 1.0), [(1.0, 0.0)]);
    }

    #[test]
    fn transparent_pixels_are_cleared_and_opaque_colors_kept() {
        let picture = Picture {
            width: 2,
            height: 1,
            channels: 3,
            pixels: vec![10, 20, 30, 40, 50, 60],
            alpha: Some(vec![0, 128]),
            icc: None,
        };
        let (pixels, color) = interleave(&picture);
        assert_eq!(color, ExtendedColorType::Rgba8);
        assert_eq!(&*pixels, [0, 0, 0, 0, 40, 50, 60, 128]);
    }

    #[test]
    fn icc_profiles_must_match_the_output_model() {
        let mut rgb = vec![0; 20];
        rgb[16..20].copy_from_slice(b"RGB ");
        assert!(matching_profile(Some(rgb.clone()), 3).is_some());
        assert!(matching_profile(Some(rgb), 1).is_none());
    }
}
