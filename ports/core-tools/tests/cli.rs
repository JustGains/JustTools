use assert_cmd::Command;
use lopdf::{Document, Object, ObjectId, Stream, StringFormat, dictionary};
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn json_formats_stdin_and_queries_paths() {
    Command::cargo_bin("justjson")
        .unwrap()
        .write_stdin("{\"user\":{\"name\":\"Ada\"},\"items\":[3]}\n")
        .assert()
        .success()
        .stdout("{\n  \"user\": {\n    \"name\": \"Ada\"\n  },\n  \"items\": [\n    3\n  ]\n}\n");

    Command::cargo_bin("justjson")
        .unwrap()
        .args(["--get", "items[0]"])
        .write_stdin("{\"items\":[3]}")
        .assert()
        .success()
        .stdout("3\n");
}

#[test]
fn json_formats_files_atomically() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("data.json");
    fs::write(&input, "{\"b\":2,\"a\":1}").unwrap();
    Command::cargo_bin("justjson")
        .unwrap()
        .args(["--sort", input.to_str().unwrap()])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(input).unwrap(),
        "{\n  \"a\": 1,\n  \"b\": 2\n}\n"
    );
}

#[test]
fn qr_writes_png_and_svg_with_opinionated_defaults() {
    let directory = tempdir().unwrap();
    let png = directory.path().join("code.png");
    Command::cargo_bin("justqr")
        .unwrap()
        .args(["-o", png.to_str().unwrap(), "hello"])
        .assert()
        .success();
    let image = image::open(&png).unwrap();
    assert_eq!((image.width(), image.height()), (1024, 1024));

    let svg = directory.path().join("code.svg");
    Command::cargo_bin("justqr")
        .unwrap()
        .args(["-o", svg.to_str().unwrap(), "hello"])
        .assert()
        .success();
    let text = fs::read_to_string(svg).unwrap();
    assert!(text.contains("<svg"));
    assert!(text.contains("shape-rendering=\"crispEdges\""));
}

#[test]
fn svg_optimizes_stdin_without_dropping_accessibility() {
    let input = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" role="img" aria-label="Test"><title>Test</title><path id="kept" d="M 0.0000 0.0000 L 10.0000 10.0000" /></svg>"#;
    let output = Command::cargo_bin("justsvg")
        .unwrap()
        .write_stdin(input)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("viewBox"));
    assert!(output.contains("aria-label"));
    assert!(output.contains("role="));
    assert!(output.contains("<title"));
    assert!(output.contains("id=\"kept\"") || output.contains("id='kept'"));
}

#[test]
fn pdf_info_merge_split_extract_and_rotate_round_trip() {
    let directory = tempdir().unwrap();
    let first = directory.path().join("first.pdf");
    let second = directory.path().join("second.pdf");
    create_pdf(&first, &[(100, 200), (300, 400)]);
    create_pdf(&second, &[(500, 600)]);

    Command::cargo_bin("justpdf")
        .unwrap()
        .arg(&first)
        .assert()
        .success()
        .stdout(predicates::str::contains("pages: 2"));

    let merged = directory.path().join("merged.pdf");
    Command::cargo_bin("justpdf")
        .unwrap()
        .args(["merge", "-o"])
        .arg(&merged)
        .arg(&first)
        .arg(&second)
        .assert()
        .success();
    assert_eq!(Document::load(&merged).unwrap().get_pages().len(), 3);

    let split_directory = directory.path().join("split");
    Command::cargo_bin("justpdf")
        .unwrap()
        .args(["split", "-o"])
        .arg(&split_directory)
        .arg(&first)
        .assert()
        .success();
    assert_eq!(
        Document::load(split_directory.join("001.pdf"))
            .unwrap()
            .get_pages()
            .len(),
        1
    );
    assert_eq!(
        Document::load(split_directory.join("002.pdf"))
            .unwrap()
            .get_pages()
            .len(),
        1
    );

    let extracted = directory.path().join("extracted.pdf");
    Command::cargo_bin("justpdf")
        .unwrap()
        .args(["extract", "--pages", "2,1", "-o"])
        .arg(&extracted)
        .arg(&first)
        .assert()
        .success();
    assert_eq!(
        page_sizes(&Document::load(extracted).unwrap()),
        [(300, 400), (100, 200)]
    );

    let rotated = directory.path().join("rotated.pdf");
    Command::cargo_bin("justpdf")
        .unwrap()
        .args(["rotate", "--pages", "2", "-o"])
        .arg(&rotated)
        .arg(&first)
        .assert()
        .success();
    let rotated = Document::load(rotated).unwrap();
    let pages: Vec<_> = rotated.get_pages().into_values().collect();
    assert_eq!(
        rotated
            .get_object(pages[0])
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Rotate")
            .ok(),
        None
    );
    assert_eq!(
        rotated
            .get_object(pages[1])
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Rotate")
            .unwrap()
            .as_i64()
            .unwrap(),
        90
    );
}

#[test]
fn pdf_images_keep_original_jpegs_and_export_transparency_as_png() {
    let directory = tempdir().unwrap();
    let pdf = directory.path().join("assets.pdf");
    let photo = create_asset_pdf(&pdf);
    let original = fs::read(&pdf).unwrap();
    let images = directory.path().join("assets-images");

    Command::cargo_bin("justpdf")
        .unwrap()
        .args(["images", "--dry-run"])
        .arg(&pdf)
        .assert()
        .success()
        .stdout(predicates::str::contains("dry run — 5 image(s)"))
        .stdout(predicates::str::contains("p001-03.png  16×8"));
    assert!(!images.exists());

    Command::cargo_bin("justpdf")
        .unwrap()
        .arg("images")
        .arg(&pdf)
        .assert()
        .success()
        .stdout(predicates::str::contains("saved 5 image(s)"))
        .stdout(predicates::str::contains("2 original JPEG, 3 PNG"))
        .stderr(predicates::str::contains(
            "p001-05: skipped: CCITTFaxDecode images are not supported",
        ));
    let mut names: Vec<_> = fs::read_dir(&images)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "p001-01.jpg",
            "p001-02.jpg",
            "p001-03.png",
            "p001-04.png",
            "p002-01.png"
        ]
    );
    // Unmasked and fully opaque JPEGs are the original bytes.
    assert_eq!(fs::read(images.join("p001-01.jpg")).unwrap(), photo);
    assert_eq!(fs::read(images.join("p001-02.jpg")).unwrap(), photo);

    let masked = image::open(images.join("p001-03.png")).unwrap();
    assert_eq!(masked.color(), image::ColorType::Rgba8);
    let masked = masked.into_rgba8();
    assert_eq!(masked.dimensions(), (16, 8));
    assert_eq!(masked.get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(masked.get_pixel(15, 7)[3], 255);

    let raw = image::open(images.join("p001-04.png")).unwrap();
    assert_eq!(raw.color(), image::ColorType::Rgb8);
    assert_eq!(
        raw.into_rgb8().into_raw(),
        [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]
    );
    let indexed = image::open(images.join("p002-01.png")).unwrap();
    assert_eq!(
        indexed.into_rgba8().into_raw(),
        [255, 0, 0, 255, 0, 0, 0, 0]
    );
    assert_eq!(fs::read(&pdf).unwrap(), original);

    Command::cargo_bin("justpdf")
        .unwrap()
        .arg("images")
        .arg(&pdf)
        .assert()
        .failure()
        .stderr(predicates::str::contains("--yes"));
    Command::cargo_bin("justpdf")
        .unwrap()
        .args(["images", "--yes"])
        .arg(&pdf)
        .assert()
        .success();
}

#[test]
fn pdf_links_are_unique_in_first_seen_order() {
    let directory = tempdir().unwrap();
    let pdf = directory.path().join("assets.pdf");
    create_asset_pdf(&pdf);

    Command::cargo_bin("justpdf")
        .unwrap()
        .arg("links")
        .arg(&pdf)
        .assert()
        .success()
        .stdout(predicates::str::contains("5 unique link(s) from 6 found"));
    assert_eq!(
        fs::read_to_string(directory.path().join("assets-links.txt")).unwrap(),
        "https://a.test\nhttps://b.test\nhttps://c.test\nhttps://d.test\nhttps://e.test\n"
    );

    // A page range excludes document-wide bookmarks; a folder output gets the default name.
    let folder = directory.path().join("lists");
    fs::create_dir(&folder).unwrap();
    Command::cargo_bin("justpdf")
        .unwrap()
        .args(["links", "--pages", "2", "-o"])
        .arg(&folder)
        .arg(&pdf)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(folder.join("assets-links.txt")).unwrap(),
        "https://a.test\nhttps://c.test\nhttps://d.test\n"
    );
}

#[test]
fn links_combine_pdf_office_and_text_files_into_titled_csv() {
    let directory = tempdir().unwrap();
    create_asset_pdf(&directory.path().join("assets.pdf"));
    create_workbook(&directory.path().join("plan.xlsx"));
    let notes = directory.path().join("notes").join("deep");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("list.md"), "- [Bench](https://bench.test)\n").unwrap();

    Command::cargo_bin("justlinks")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "assets.pdf",
            "plan.xlsx",
            "notes",
            "--recursive",
            "--csv",
            "-o",
            "out.csv",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "8 unique link(s) from 10 found in 3 file(s)",
        ));
    let markdown = Path::new("notes").join("deep").join("list.md");
    assert_eq!(
        fs::read_to_string(directory.path().join("out.csv")).unwrap(),
        format!(
            "\u{feff}url,title,file,location,occurrences\r\n\
             https://a.test,,assets.pdf,page 1,2\r\n\
             https://b.test,Squat,assets.pdf,page 1,2\r\n\
             https://c.test,,assets.pdf,page 2,1\r\n\
             https://d.test,,assets.pdf,page 2,1\r\n\
             https://e.test,Website,assets.pdf,bookmarks,1\r\n\
             https://www.youtube.com/watch?v=NDmJNX9JrLs,OMNI-GRIP LAT PULLDOWN,plan.xlsx,Plan!A1,1\r\n\
             https://rel.test,Squat,plan.xlsx,Plan!C2,1\r\n\
             https://bench.test,Bench,{},line 1,1\r\n",
            markdown.display()
        )
    );

    // The default list lands in the current folder and is never re-read.
    for _ in 0..2 {
        Command::cargo_bin("justlinks")
            .unwrap()
            .current_dir(directory.path())
            .args([".", "--recursive", "--yes"])
            .assert()
            .success();
    }
    let list = fs::read_to_string(directory.path().join("links.txt")).unwrap();
    assert_eq!(list.lines().count(), 8);
    assert!(list.starts_with("https://a.test\n"));

    Command::cargo_bin("justlinks")
        .unwrap()
        .write_stdin("Row: https://row.test\nwww.site.test and https://row.test\n")
        .assert()
        .success()
        .stdout("https://row.test\nwww.site.test\n");
}

#[test]
fn links_deduplicate_case_and_video_identity_across_files_and_job_counts() {
    let directory = tempdir().unwrap();
    let first = "Site: HTTPS://Example.test/Path?Q=Value\nShort: youtu.be/AbC_dEf-123?t=42\nFull: https://music.youtube.com/watch?v=AbC_dEf-123&list=PL1\n";
    let second = "Site: https://example.test/path?q=value\nFull: https://youtube.com/shorts/abc_def-123?si=token\nFull: https://www.youtube-nocookie.com/embed/AbC_dEf-123?start=30\nOther: https://youtu.be/XyZ_dEf-123#t=5\n";
    fs::write(directory.path().join("first.txt"), first).unwrap();
    fs::write(directory.path().join("second.txt"), second).unwrap();
    for jobs in ["1", "4"] {
        Command::cargo_bin("justlinks")
            .unwrap()
            .current_dir(directory.path())
            .args(["first.txt", "second.txt", "--jobs", jobs, "-o", "-"])
            .assert()
            .success()
            .stdout("HTTPS://Example.test/Path?Q=Value\nhttps://www.youtube.com/watch?v=AbC_dEf-123\nhttps://www.youtube.com/watch?v=XyZ_dEf-123\n")
            .stderr(predicates::str::contains("first.txt: 2 unique link(s) from 3 found")
                .and(predicates::str::contains("3 unique link(s) from 7 found in 2 file(s)")));
    }
    Command::cargo_bin("justlinks")
        .unwrap()
        .args(["--csv"])
        .write_stdin(format!("{first}{second}"))
        .assert()
        .success()
        .stdout("url,title,file,location,occurrences\r\nHTTPS://Example.test/Path?Q=Value,Site,stdin,line 1,2\r\nhttps://www.youtube.com/watch?v=AbC_dEf-123,Full,stdin,line 2,4\r\nhttps://www.youtube.com/watch?v=XyZ_dEf-123,Other,stdin,line 7,1\r\n");
    assert_eq!(
        fs::read_to_string(directory.path().join("first.txt")).unwrap(),
        first
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("second.txt")).unwrap(),
        second
    );
}

#[test]
fn port_finds_a_live_tcp_listener_and_reports_free_ports() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let used = listener.local_addr().unwrap().port();
    let output = Command::cargo_bin("justport")
        .unwrap()
        .args(["--json", &used.to_string()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["Port"], used);
    assert_eq!(value[0]["Available"], false);
    assert!(
        value[0]["Endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["Protocol"] == "TCP")
    );
}

#[test]
fn every_binary_has_help() {
    for binary in [
        "justjson",
        "justlinks",
        "justqr",
        "justpdf",
        "justsvg",
        "justport",
    ] {
        Command::cargo_bin(binary)
            .unwrap()
            .arg("--help")
            .assert()
            .success()
            .stdout(predicates::str::contains("Usage:"));
    }
}

fn create_pdf(path: &Path, sizes: &[(i64, i64)]) {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    for (width, height) in sizes {
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), (*width).into(), (*height).into()],
        });
        kids.push(page_id.into());
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => sizes.len() as i64,
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    document.save(path).unwrap();
}

/// Writes a two-page PDF with representative images and links, returning the
/// JPEG bytes embedded in it.
fn create_asset_pdf(path: &Path) -> Vec<u8> {
    let photo = image::RgbImage::from_fn(16, 8, |x, y| {
        image::Rgb([(x * 16) as u8, (y * 32) as u8, 128])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode_image(&photo)
        .unwrap();

    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let page_ids = [document.new_object_id(), document.new_object_id()];
    let image = |extra: lopdf::Dictionary| {
        let mut dictionary = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 16,
            "Height" => 8,
            "BitsPerComponent" => 8,
        };
        dictionary.extend(&extra);
        dictionary
    };
    let mut opaque = Stream::new(
        image(dictionary! { "ColorSpace" => "DeviceGray" }),
        vec![255; 16 * 8],
    );
    opaque.compress().unwrap();
    let opaque = document.add_object(opaque);
    let half = (0..16 * 8)
        .map(|index| if index % 16 < 8 { 0 } else { 255 })
        .collect();
    let half = document.add_object(Stream::new(
        image(dictionary! { "ColorSpace" => "DeviceGray" }),
        half,
    ));
    let jpeg_image = |mask: Option<ObjectId>| {
        let mut dictionary = image(dictionary! {
            "ColorSpace" => "DeviceRGB",
            "Filter" => "DCTDecode",
        });
        if let Some(mask) = mask {
            dictionary.set("SMask", mask);
        }
        Stream::new(dictionary, jpeg.clone())
    };
    let plain = document.add_object(jpeg_image(None));
    let opaque_masked = document.add_object(jpeg_image(Some(opaque)));
    let transparent = document.add_object(jpeg_image(Some(half)));
    let mut raw = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 2,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
        },
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
    );
    raw.compress().unwrap();
    let raw = document.add_object(raw);
    let form = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im4" => raw } },
        },
        b"/Im4 Do".to_vec(),
    ));
    let fax = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 8,
            "Height" => 8,
            "ImageMask" => true,
            "Filter" => "CCITTFaxDecode",
        },
        vec![0; 8],
    ));
    // Palette index 1 (blue) is keyed out by the color-key mask.
    let indexed = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 1,
            "BitsPerComponent" => 1,
            "ColorSpace" => vec![
                "Indexed".into(),
                "DeviceRGB".into(),
                1.into(),
                Object::String(vec![255, 0, 0, 0, 0, 255], StringFormat::Hexadecimal),
            ],
            "Mask" => vec![1.into(), 1.into()],
        },
        vec![0b0100_0000],
    ));

    let uri = |target: &str| {
        Object::Dictionary(dictionary! { "S" => "URI", "URI" => Object::string_literal(target) })
    };
    let mut link = |action: Object| {
        document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "A" => action,
        })
    };
    let first_links: Vec<Object> = vec![
        link(uri("https://a.test")).into(),
        link(Object::Dictionary(dictionary! {
            "S" => "GoTo",
            "D" => vec![page_ids[1].into(), "Fit".into()],
        }))
        .into(),
        link(uri("https://b.test")).into(),
    ];
    let repeated = link(uri("https://a.test"));
    // Chained actions c -> d -> c form a cycle that must terminate.
    let chained = document.new_object_id();
    let next = document.add_object(dictionary! {
        "S" => "URI",
        "URI" => Object::string_literal("https://d.test"),
        "Next" => chained,
    });
    document.objects.insert(
        chained,
        Object::Dictionary(dictionary! {
            "S" => "URI",
            "URI" => Object::string_literal("https://c.test"),
            "Next" => next,
        }),
    );
    let chain = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        "A" => chained,
    });

    for (page_id, xobjects, annotations) in [
        (
            page_ids[0],
            dictionary! {
                "Im1" => plain,
                "Im2" => opaque_masked,
                "Im3" => transparent,
                "Fm1" => form,
                "Im5" => fax,
            },
            first_links,
        ),
        (
            page_ids[1],
            dictionary! { "Im1" => plain, "Im6" => indexed },
            vec![repeated.into(), chain.into()],
        ),
    ] {
        document.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Resources" => dictionary! { "XObject" => xobjects },
                "Annots" => annotations,
            }),
        );
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.map(Object::from).to_vec(),
            "Count" => 2,
        }),
    );
    let outlines = document.new_object_id();
    let bookmark = document.add_object(dictionary! {
        "Title" => Object::string_literal("Website"),
        "Parent" => outlines,
        "A" => uri("https://e.test"),
    });
    document.objects.insert(
        outlines,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => bookmark,
            "Last" => bookmark,
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "Outlines" => outlines,
    });
    document.trailer.set("Root", catalog_id);
    document.save(path).unwrap();
    jpeg
}

/// Writes a minimal workbook: a labeled URL, a row heading, a repeated URL,
/// and a hyperlinked empty cell.
fn create_workbook(path: &Path) {
    const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const RELATIONSHIPS: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    let parts = [
        (
            "[Content_Types].xml".to_owned(),
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#
                .to_owned(),
        ),
        (
            "xl/workbook.xml".into(),
            format!(
                r#"<workbook xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}"><sheets><sheet name="Plan" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels".into(),
            format!(
                r#"<Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{RELATIONSHIPS}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml".into(),
            format!(
                r#"<worksheet xmlns="{MAIN}" xmlns:r="{RELATIONSHIPS}"><sheetData>
                <row r="1"><c r="A1" t="inlineStr"><is><t>OMNI-GRIP LAT PULLDOWN: https://youtu.be/NDmJNX9JrLs?t=4m7s</t></is></c></row>
                <row r="2"><c r="A2" t="inlineStr"><is><t>Squat</t></is></c><c r="B2" t="inlineStr"><is><t>https://b.test</t></is></c><c r="C2"/></row>
                </sheetData><hyperlinks><hyperlink ref="C2" r:id="rId1"/></hyperlinks></worksheet>"#
            ),
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            format!(
                r#"<Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{RELATIONSHIPS}/hyperlink" Target="https://rel.test" TargetMode="External"/></Relationships>"#
            ),
        ),
    ];
    let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, xml) in parts {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut archive, xml.as_bytes()).unwrap();
    }
    archive.finish().unwrap();
}

fn page_sizes(document: &Document) -> Vec<(i64, i64)> {
    document
        .get_pages()
        .into_values()
        .map(|page_id| {
            let page = document.get_object(page_id).unwrap().as_dict().unwrap();
            let bounds = page.get(b"MediaBox").unwrap().as_array().unwrap();
            (
                bounds[2].as_i64().unwrap() - bounds[0].as_i64().unwrap(),
                bounds[3].as_i64().unwrap() - bounds[1].as_i64().unwrap(),
            )
        })
        .collect()
}
