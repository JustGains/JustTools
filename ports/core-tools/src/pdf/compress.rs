//! `justpdf compress`: write a smaller copy of each PDF.
//!
//! Every level drops unused objects and earlier revisions, recompresses
//! streams at the best Flate setting, and packs objects into object streams.
//! Lossy levels also downsample and re-encode gray and RGB photos as JPEG.

use super::images::{self, Color, Filter};
use super::{Cli, confirm_outputs, dictionary, document_stem, load_pdf, page_size};
use crate::common::{
    absolute_lexical, atomic_write, display_path, format_bytes, same_path, validate_unique_outputs,
};
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, GrayImage, ImageEncoder, RgbImage, imageops};
use lopdf::{Document, Object, ObjectId, Stream};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Largest decoded stream recompressed losslessly.
const MAX_REFLATE_BYTES: usize = 256 << 20;
/// Encoded images smaller than this are left alone.
const MIN_IMAGE_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(super) enum Level {
    /// Restructure only; images keep their exact bytes.
    Lossless,
    /// Images at most 150 ppi of the page, JPEG quality 75.
    #[default]
    Balanced,
    /// Images at most 96 ppi of the page, JPEG quality 50.
    Smallest,
}

impl Level {
    /// The image ceiling in pixels per inch of the largest page, and the JPEG
    /// quality, or `None` when images are kept unchanged.
    fn images(self) -> Option<(f64, u8)> {
        match self {
            Self::Lossless => None,
            Self::Balanced => Some((150.0, 75)),
            Self::Smallest => Some((96.0, 50)),
        }
    }

    fn describe(self) -> String {
        match self.images() {
            None => "lossless, images unchanged".into(),
            Some((ppi, quality)) => format!("images ≤{ppi} ppi, JPEG quality {quality}"),
        }
    }
}

/// An image that can be re-encoded as JPEG without changing how it renders.
struct Job<'a> {
    id: ObjectId,
    stream: &'a Stream,
    filters: Vec<Filter>,
    channels: usize,
}

pub(super) fn run(files: &[PathBuf], options: &Cli) -> Result<()> {
    let plans = outputs(files, options.output.as_deref())?;
    validate_unique_outputs(&plans)?;
    if let Some((input, _)) = plans
        .iter()
        .find(|(input, output)| same_path(input, output))
    {
        bail!(
            "compress output cannot overwrite its input: {}",
            display_path(input)
        );
    }
    let level = options.level;
    if options.dry_run {
        for (input, output) in &plans {
            println!(
                "justpdf: dry run — compress {} ({}) -> {}",
                display_path(input),
                level.describe(),
                display_path(output)
            );
        }
        return Ok(());
    }
    let destinations: Vec<_> = plans.iter().map(|(_, output)| output.clone()).collect();
    confirm_outputs(&destinations, options.yes, false)?;

    let mut failed = 0usize;
    for (input, output) in &plans {
        if let Err(error) = compress(input, output, level) {
            if plans.len() == 1 {
                return Err(error);
            }
            eprintln!("justpdf: {error:#}");
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} PDF(s) could not be compressed", plans.len());
    }
    Ok(())
}

/// `<name>-compressed.pdf` beside each PDF; `--output` is the file for one
/// PDF, or the folder that receives them.
fn outputs(files: &[PathBuf], output: Option<&Path>) -> Result<Vec<(PathBuf, PathBuf)>> {
    let name = |input: &Path| format!("{}-compressed.pdf", document_stem(input));
    let output = output.map(absolute_lexical).transpose()?;
    if let Some(output) = &output
        && files.len() > 1
        && output.is_file()
    {
        bail!(
            "compress output must be a folder for several PDFs: {}",
            display_path(output)
        );
    }
    Ok(files
        .iter()
        .map(|input| {
            let destination = match &output {
                None => input.with_file_name(name(input)),
                Some(output) if files.len() == 1 && !output.is_dir() => output.clone(),
                Some(folder) => folder.join(name(input)),
            };
            (input.clone(), destination)
        })
        .collect())
}

fn compress(input: &Path, output: &Path, level: Level) -> Result<()> {
    let before = fs::metadata(input)?.len();
    let mut document = load_pdf(input)?;
    if is_signed(&document) {
        eprintln!(
            "justpdf: {}: the compressed copy's digital signature will no longer verify",
            display_path(input)
        );
    }
    document.prune_objects();
    let images = level.images().map_or(0, |(ppi, quality)| {
        recompress_images(&mut document, ppi, quality)
    });
    reflate(&mut document);
    document.compress();
    document.renumber_objects();
    let mut bytes = Vec::new();
    document
        .save_modern(&mut bytes)
        .map_err(|error| anyhow!("{}: could not encode PDF: {error}", display_path(input)))?;
    let after = bytes.len() as u64;
    if after >= before {
        println!(
            "justpdf: {} is already compact ({}); nothing written",
            display_path(input),
            format_bytes(before)
        );
        return Ok(());
    }
    atomic_write(output, &bytes)?;
    let images = match images {
        0 => String::new(),
        count => format!(", {count} image(s) re-encoded"),
    };
    println!(
        "justpdf: compressed {} {} -> {} (-{}%{images}) -> {}",
        display_path(input),
        format_bytes(before),
        format_bytes(after),
        (before - after) * 100 / before,
        display_path(output)
    );
    Ok(())
}

/// Whether the document declares signature fields, which any rewrite breaks.
fn is_signed(document: &Document) -> bool {
    document
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"AcroForm").ok())
        .and_then(|form| dictionary(document, form))
        .and_then(|form| form.get(b"SigFlags").ok())
        .and_then(|flags| flags.as_i64().ok())
        .is_some_and(|flags| flags & 1 != 0)
}

/// Re-encodes eligible images as JPEG, downsampled so none exceeds the
/// largest page at `ppi`, keeping each only when it saves at least 10%.
fn recompress_images(document: &mut Document, ppi: f64, quality: u8) -> usize {
    let longest = document
        .get_pages()
        .values()
        .filter_map(|page| page_size(document, *page))
        .map(|(width, height)| width.max(height))
        .fold(0.0, f64::max);
    let ceiling = (longest > 0.0).then(|| (longest / 72.0 * ppi).round().max(1.0) as u32);
    let results = {
        let jobs = jobs(document);
        let encoded = parallel(&jobs, |job| reencode(job, ceiling, quality).ok().flatten());
        jobs.iter()
            .map(|job| job.id)
            .zip(encoded)
            .filter_map(|(id, stream)| Some((id, stream?)))
            .collect::<Vec<_>>()
    };
    let count = results.len();
    for (id, stream) in results {
        document.objects.insert(id, Object::Stream(stream));
    }
    count
}

fn jobs(document: &Document) -> Vec<Job<'_>> {
    // Soft masks and stencil masks are alpha, not pictures.
    let masks: HashSet<ObjectId> = document
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .flat_map(|stream| [stream.dict.get(b"SMask"), stream.dict.get(b"Mask")])
        .filter_map(|mask| mask.ok()?.as_reference().ok())
        .collect();
    document
        .objects
        .iter()
        .filter(|(id, _)| !masks.contains(id))
        .filter_map(|(id, object)| {
            let stream = object.as_stream().ok()?;
            let (filters, channels) = eligible(document, stream)?;
            Some(Job {
                id: *id,
                stream,
                filters,
                channels,
            })
        })
        .collect()
}

/// Filters and channel count for an 8-bit gray or RGB image stored raw or as
/// JPEG, with no stencil, color key, or /Decode remapping that JPEG would
/// disturb.
fn eligible(document: &Document, stream: &Stream) -> Option<(Vec<Filter>, usize)> {
    let dictionary = &stream.dict;
    if images::name_entry(dictionary, b"Subtype") != Some(b"Image")
        || stream.content.len() < MIN_IMAGE_BYTES
        || images::is_stencil(dictionary)
        || dictionary.has(b"Decode")
        || images::bits_per_component(dictionary).ok()? != 8
    {
        return None;
    }
    if let Ok(mask) = dictionary.get(b"Mask")
        && !matches!(mask, Object::Reference(_))
    {
        return None;
    }
    let (color, _) = images::color_space(document, dictionary.get(b"ColorSpace").ok()?, 0).ok()?;
    let channels = match color {
        Color::Gray => 1,
        Color::Rgb => 3,
        _ => return None,
    };
    let filters = images::filters(document, dictionary).ok()?;
    let supported = filters.iter().enumerate().all(|(index, filter)| {
        images::TRANSPORTS.contains(&filter.name.as_slice())
            || (index + 1 == filters.len()
                && filter.name == b"DCTDecode"
                && filter.params.is_none())
    });
    supported.then_some((filters, channels))
}

fn reencode(job: &Job, ceiling: Option<u32>, quality: u8) -> Result<Option<Stream>> {
    let stream = job.stream;
    let (width, height) = images::dimensions(&stream.dict)?;
    let pixels = if images::codec(&job.filters).is_some() {
        let picture = images::decode_jpeg(&images::payload(stream, &job.filters)?)?;
        if picture.channels != job.channels || (picture.width, picture.height) != (width, height) {
            return Ok(None);
        }
        picture.pixels
    } else {
        images::samples(stream, &job.filters, (width, height), job.channels, 8)?
            .into_iter()
            .map(|sample| sample as u8)
            .collect()
    };
    let (width, height, pixels) = downsample(width, height, job.channels, pixels, ceiling)?;
    let color = if job.channels == 1 {
        ExtendedColorType::L8
    } else {
        ExtendedColorType::Rgb8
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality).write_image(&pixels, width, height, color)?;
    if jpeg.len() * 10 > stream.content.len() * 9 {
        return Ok(None);
    }
    let mut dictionary = stream.dict.clone();
    dictionary.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    dictionary.remove(b"DecodeParms");
    dictionary.set("Width", i64::from(width));
    dictionary.set("Height", i64::from(height));
    Ok(Some(Stream::new(dictionary, jpeg).with_compression(false)))
}

/// Scales the image down so its longer side fits `ceiling`.
fn downsample(
    width: u32,
    height: u32,
    channels: usize,
    pixels: Vec<u8>,
    ceiling: Option<u32>,
) -> Result<(u32, u32, Vec<u8>)> {
    let Some(ceiling) = ceiling.filter(|ceiling| width.max(height) > *ceiling) else {
        return Ok((width, height, pixels));
    };
    let scale = f64::from(ceiling) / f64::from(width.max(height));
    let fit = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    let (new_width, new_height) = (fit(width), fit(height));
    let filter = imageops::FilterType::CatmullRom;
    let truncated = || anyhow!("image data is truncated");
    let pixels = if channels == 1 {
        let image = GrayImage::from_raw(width, height, pixels).ok_or_else(truncated)?;
        imageops::resize(&image, new_width, new_height, filter).into_raw()
    } else {
        let image = RgbImage::from_raw(width, height, pixels).ok_or_else(truncated)?;
        imageops::resize(&image, new_width, new_height, filter).into_raw()
    };
    Ok((new_width, new_height, pixels))
}

/// Recompresses plain Flate streams at the best level when that is smaller.
fn reflate(document: &mut Document) {
    let candidates: Vec<(ObjectId, &Stream)> = document
        .objects
        .iter()
        .filter_map(|(id, object)| {
            let stream = object.as_stream().ok()?;
            let plain_flate = stream.allows_compression
                && !stream.dict.has(b"DecodeParms")
                && images::name_entry(&stream.dict, b"Filter") == Some(b"FlateDecode");
            plain_flate.then_some((*id, stream))
        })
        .collect();
    let smaller = parallel(&candidates, |(_, stream)| {
        let plain = stream
            .decompressed_content_with_limit(MAX_REFLATE_BYTES)
            .ok()?;
        let mut dictionary = stream.dict.clone();
        dictionary.remove(b"Filter");
        let mut fresh = Stream::new(dictionary, plain);
        fresh.compress().ok()?;
        (fresh.content.len() < stream.content.len()).then_some(fresh)
    });
    let replaced: Vec<_> = candidates
        .iter()
        .map(|(id, _)| *id)
        .zip(smaller)
        .filter_map(|(id, stream)| Some((id, stream?)))
        .collect();
    for (id, stream) in replaced {
        document.objects.insert(id, Object::Stream(stream));
    }
}

/// Maps `items` on every available core, keeping their order.
fn parallel<T: Sync, R: Send + Sync>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let slots: Vec<OnceLock<R>> = items.iter().map(|_| OnceLock::new()).collect();
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(items.len());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    let _ = slots[index].set(work(item));
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.into_inner().expect("every item is processed"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampling_fits_the_longer_side_and_keeps_aspect() {
        let (width, height, pixels) =
            downsample(400, 200, 3, vec![128; 400 * 200 * 3], Some(100)).unwrap();
        assert_eq!((width, height), (100, 50));
        assert_eq!(pixels.len(), 100 * 50 * 3);
        let (width, height, _) = downsample(80, 40, 1, vec![0; 80 * 40], Some(100)).unwrap();
        assert_eq!((width, height), (80, 40));
    }

    #[test]
    fn parallel_map_keeps_order() {
        let items: Vec<u32> = (0..100).collect();
        assert_eq!(
            parallel(&items, |item| item * 2),
            (0..100).map(|item| item * 2).collect::<Vec<_>>()
        );
        assert!(parallel(&[] as &[u32], |item| *item).is_empty());
    }
}
