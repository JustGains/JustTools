use assert_cmd::Command;
use lopdf::{Document, Object, dictionary};
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
    for binary in ["justjson", "justqr", "justpdf", "justsvg", "justport"] {
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
