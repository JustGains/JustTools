//! Background-removal pastes use JustRMBG, then publish under unused PNG names.
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use super::{actions, clipboard::Snapshot, free_path};
use crate::{commands::image_ops, common};

pub(super) fn supports_files(paths: &[PathBuf]) -> bool {
    !paths.is_empty()
        && paths.iter().all(|path| {
            path.is_file()
                && image_ops::STILL_EXTENSIONS.contains(&image_ops::extension(path).as_str())
        })
}

pub(super) fn paste(snapshot: Snapshot, directory: &Path, dry_run: bool) -> Result<String, String> {
    paste_with(snapshot, directory, dry_run, |args| {
        // Keep RMBG's dependency confirmation; the picker never grants --download.
        crate::commands::rmbg::run(args).map_err(|e| format!("justrmbg: {}", e.message()))
    })
}

fn paste_with(
    snapshot: Snapshot,
    directory: &Path,
    dry_run: bool,
    run: impl FnOnce(Vec<OsString>) -> Result<(), String>,
) -> Result<String, String> {
    let mut png = None;
    let names = match &snapshot {
        Snapshot::Image(bytes, format) => {
            let image = actions::decode(bytes, *format)?;
            if !dry_run && *format != image::ImageFormat::Png {
                let mut encoded = std::io::Cursor::new(Vec::new());
                image
                    .write_to(&mut encoded, image::ImageFormat::Png)
                    .map_err(|e| format!("cannot prepare clipboard image: {e}"))?;
                png = Some(encoded.into_inner());
            }
            vec![format!("Image {}-nobg.png", actions::stamp())]
        }
        Snapshot::Files { paths, cut: false } if supports_files(paths) => {
            let mut names = Vec::with_capacity(paths.len());
            for path in paths {
                image_ops::assert_static(super::TOOL, path).map_err(|e| e.message().to_owned())?;
                image::image_dimensions(path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                names.push(format!("{stem}-nobg.png"));
            }
            names
        }
        _ => return Err("background removal needs an image or copied still-image files".into()),
    };
    if dry_run {
        for name in &names {
            println!(
                "  justrmbg -> {} (transparent PNG; source kept, no overwrite)",
                free_path(directory, name).display()
            );
        }
        return Ok(format!(
            "dry run: {} image(s), remove background",
            names.len()
        ));
    }
    if common::interrupted() {
        return Err("cancelled".into());
    }
    fs::create_dir_all(directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;
    let workspace = tempfile::Builder::new()
        .prefix(".justpaste-")
        .tempdir_in(directory)
        .map_err(|e| format!("cannot write in {}: {e}", directory.display()))?;
    let results = workspace.path().join("results");
    fs::create_dir(&results).map_err(|e| e.to_string())?;
    // Unique staging names also allow selections with matching source stems.
    let sources = match snapshot {
        Snapshot::Image(bytes, _) => {
            let path = workspace.path().join("0.png");
            fs::write(&path, png.unwrap_or(bytes)).map_err(|e| e.to_string())?;
            vec![path]
        }
        Snapshot::Files { paths, .. } => {
            let mut sources = Vec::with_capacity(paths.len());
            for (index, source) in paths.iter().enumerate() {
                let path = workspace
                    .path()
                    .join(format!("{index}{}", image_ops::extension(source)));
                fs::copy(source, &path)
                    .map_err(|e| format!("cannot read {}: {e}", source.display()))?;
                sources.push(path);
            }
            sources
        }
        _ => unreachable!("validated image sources"),
    };
    let output = if sources.len() == 1 {
        results.join("0-nobg.png")
    } else {
        results.clone()
    };
    let mut args = vec![
        OsString::from("--output"),
        output.into_os_string(),
        "--format".into(),
        "png".into(),
        "--".into(),
    ];
    args.extend(sources.into_iter().map(PathBuf::into_os_string));
    crate::progress::stage("Removing image backgrounds with JustRMBG…");
    run(args)?;
    let mut saved = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        if common::interrupted() {
            return Err("cancelled".into());
        }
        let destination = free_path(directory, name);
        tempfile::TempPath::try_from_path(results.join(format!("{index}-nobg.png")))
            .map_err(|e| e.to_string())?
            .persist_noclobber(&destination)
            .map_err(|e| format!("cannot save {}: {e}", destination.display()))?;
        saved.push(destination);
    }
    Ok(match saved.as_slice() {
        [one] => format!("saved {} (background removed)", one.display()),
        many => format!("saved {} PNG files (background removed)", many.len()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_fn(4, 3, |x, _| {
            image::Rgba([150, 20, 40, if x == 0 { 0 } else { 255 }])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    // Exercise staging and publication without installing a runtime or model.
    fn rmbg_output(args: Vec<OsString>) -> Result<(), String> {
        assert_eq!(&args[0], "--output");
        assert_eq!(&args[2..5], ["--format", "png", "--"]);
        assert!(!args.iter().any(|arg| arg == "--download" || arg == "--yes"));
        let output = PathBuf::from(&args[1]);
        for (index, input) in args[5..].iter().enumerate() {
            let destination = if args.len() == 6 {
                output.clone()
            } else {
                output.join(format!("{index}-nobg.png"))
            };
            assert_eq!(fs::read(input).unwrap(), png());
            fs::write(destination, png()).unwrap();
        }
        Ok(())
    }

    #[test]
    fn clipboard_image_publishes_a_transparent_png_and_cleans_the_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let result = paste_with(
            Snapshot::Image(png(), image::ImageFormat::Png),
            directory.path(),
            false,
            rmbg_output,
        )
        .unwrap();
        assert!(result.contains("background removed"));
        let files: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1);
        assert!(
            files[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-nobg.png")
        );
        let image = image::open(&files[0]).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (4, 3));
        assert_eq!(image.get_pixel(0, 1).0[3], 0);
    }

    #[test]
    fn copied_images_keep_sources_and_number_duplicate_or_existing_names() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        let output = directory.path().join("out");
        for path in [&first, &second, &output] {
            fs::create_dir(path).unwrap();
        }
        let sources = vec![first.join("portrait.png"), second.join("portrait.PNG")];
        for source in &sources {
            fs::write(source, png()).unwrap();
        }
        let occupied = output.join("portrait-nobg.png");
        fs::write(&occupied, "keep existing").unwrap();
        paste_with(
            Snapshot::Files {
                paths: sources.clone(),
                cut: false,
            },
            &output,
            false,
            rmbg_output,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(occupied).unwrap(), "keep existing");
        for source in sources {
            assert_eq!(fs::read(source).unwrap(), png());
        }
        assert_eq!(
            fs::read(output.join("portrait-nobg (2).png")).unwrap(),
            png()
        );
        assert_eq!(
            fs::read(output.join("portrait-nobg (3).png")).unwrap(),
            png()
        );
        assert_eq!(fs::read_dir(output).unwrap().count(), 3);
    }

    #[test]
    fn dry_run_and_invalid_sources_never_invoke_rmbg_or_create_an_output_folder() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out");
        let source = directory.path().join("photo.png");
        fs::write(&source, png()).unwrap();
        for snapshot in [
            Snapshot::Image(png(), image::ImageFormat::Png),
            Snapshot::Files {
                paths: vec![source.clone()],
                cut: false,
            },
        ] {
            let result = paste_with(snapshot, &output, true, |_| panic!("dry run called RMBG"));
            assert!(result.unwrap().contains("dry run"));
        }
        for snapshot in [
            Snapshot::Files {
                paths: vec![source],
                cut: true,
            },
            Snapshot::Image(b"invalid".to_vec(), image::ImageFormat::Png),
            Snapshot::Text("hello".into()),
        ] {
            assert!(
                paste_with(snapshot, &output, false, |_| panic!(
                    "invalid input called RMBG"
                ))
                .is_err()
            );
        }
        assert!(!output.exists());
    }

    #[test]
    fn rmbg_failure_discards_partial_results() {
        let directory = tempfile::tempdir().unwrap();
        let error = paste_with(
            Snapshot::Image(png(), image::ImageFormat::Png),
            directory.path(),
            false,
            |args| {
                rmbg_output(args)?;
                Err("inference failed".into())
            },
        )
        .unwrap_err();
        assert_eq!(error, "inference failed");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
