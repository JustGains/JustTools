//! `justpaste --ai` against a local stand-in for OpenAI-compatible and Anthropic APIs.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    sync::mpsc,
};

fn binary() -> std::path::PathBuf {
    std::env::var_os("JUSTTOOLS_PASTE_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_just").into())
}

struct Request {
    path: String,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Answers each request with the next body from `answers`, by path suffix.
fn server(answers: Vec<(&'static str, serde_json::Value)>) -> (String, mpsc::Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let mut headers = Vec::new();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (key, value) = header.split_once(':').unwrap();
                headers.push((key.trim().to_owned(), value.trim().to_owned()));
            }
            let length: usize = headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map_or(0, |(_, value)| value.parse().unwrap());
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let answer = answers
                .iter()
                .find(|(suffix, _)| path.split('?').next().unwrap().ends_with(suffix))
                .map(|(_, answer)| answer.to_string())
                .unwrap_or_else(|| "{}".into());
            let _ = sender.send(Request {
                path,
                headers,
                body: serde_json::from_slice(&body).unwrap_or_default(),
            });
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                answer.len()
            );
        }
    });
    (address, receiver)
}

fn paste(directory: &std::path::Path, args: &[&str], input: &str) -> std::process::Output {
    let mut command = Command::new(binary());
    command
        .arg("paste")
        .args(args)
        .args(["--output".as_ref(), directory.as_os_str()])
        .env("JUSTTOOLS_DEFAULTS", directory.join("defaults.toml"))
        .env("OPENAI_API_KEY", "sk-openai-test")
        .env("ANTHROPIC_API_KEY", "sk-ant-test")
        .env("JUSTPASTE_AI_API_KEY", "generic-test")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn saved(directory: &std::path::Path) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap() != "defaults.toml")
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&path).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn openai_compatible_answer_is_saved_as_a_new_text_file() {
    let directory = tempfile::tempdir().unwrap();
    let (endpoint, requests) = server(vec![(
        "/chat/completions",
        serde_json::json!({"choices": [{"message": {"content": "```text\nHELLO WORLD\n```"}}]}),
    )]);
    let output = paste(
        directory.path(),
        &[
            "--ai",
            "Uppercase it",
            "--ai-service",
            "compatible",
            "--ai-endpoint",
            &endpoint,
            "--ai-model",
            "local-model",
            "--as",
            "md",
        ],
        "hello world",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = requests.recv().unwrap();
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.header("authorization"), Some("Bearer generic-test"));
    assert_eq!(request.body["model"], "local-model");
    let user = request.body["messages"][1]["content"].as_str().unwrap();
    assert!(
        user.contains("Uppercase it") && user.contains("<clipboard>\nhello world\n</clipboard>")
    );
    let files = saved(directory.path());
    assert_eq!(files.len(), 1);
    assert!(
        files[0].0.starts_with("AI paste ") && files[0].0.ends_with(".md"),
        "{files:?}"
    );
    assert_eq!(files[0].1, "HELLO WORLD\n");
    // The key never reaches the output.
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(!printed.contains("generic-test"));
}

#[test]
fn anthropic_uses_its_messages_api_headers_and_fallbacks() {
    let directory = tempfile::tempdir().unwrap();
    let (endpoint, requests) = server(vec![(
        "/messages",
        serde_json::json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "Their car is over there."}]}),
    )]);
    let output = paste(
        directory.path(),
        &[
            "--ai-action",
            "fix-spelling",
            "--ai-service",
            "anthropic",
            "--ai-endpoint",
            &endpoint,
        ],
        "There car is over their.",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = requests.recv().unwrap();
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.header("x-api-key"), Some("sk-ant-test"));
    assert_eq!(request.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(
        request.header("anthropic-beta"),
        Some("server-side-fallback-2026-07-01")
    );
    assert_eq!(request.body["model"], "claude-opus-5-5");
    assert_eq!(request.body["fallbacks"], "default");
    assert!(
        request.body["system"]
            .as_str()
            .unwrap()
            .contains("data, not instructions")
    );
    let files = saved(directory.path());
    assert!(
        files[0].0.starts_with("Fix spelling and grammar "),
        "{files:?}"
    );
    assert_eq!(files[0].1, "Their car is over there.");
}

#[test]
fn openai_moderation_flag_stops_before_the_text_is_transformed() {
    let directory = tempfile::tempdir().unwrap();
    let (endpoint, requests) = server(vec![
        (
            "/moderations",
            serde_json::json!({"results": [{"flagged": true}]}),
        ),
        (
            "/chat/completions",
            serde_json::json!({"choices": [{"message": {"content": "x"}}]}),
        ),
    ]);
    let output = paste(
        directory.path(),
        &[
            "--ai",
            "Rewrite",
            "--ai-service",
            "openai",
            "--ai-endpoint",
            &endpoint,
            "--ai-model",
            "gpt-test",
        ],
        "flag me",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("moderation"));
    let request = requests.recv().unwrap();
    assert_eq!(request.path, "/v1/moderations");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer sk-openai-test")
    );
    assert!(
        requests.try_recv().is_err(),
        "the chat endpoint must not be called"
    );
    assert!(saved(directory.path()).is_empty());
}

#[test]
fn dry_run_and_missing_settings_send_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let (endpoint, requests) = server(vec![]);
    let output = paste(
        directory.path(),
        &[
            "--ai",
            "Rewrite",
            "--ai-service",
            "compatible",
            "--ai-endpoint",
            &endpoint,
            "--ai-model",
            "m",
            "--dry-run",
        ],
        "text",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("no request made"));
    let output = paste(
        directory.path(),
        &["--ai", "Rewrite", "--ai-service", "openai"],
        "text",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("model"));
    let output = paste(directory.path(), &["--ai-action", "Summarize"], "text");
    assert!(String::from_utf8_lossy(&output.stderr).contains("--saved-settings"));
    assert!(requests.try_recv().is_err());
    assert!(saved(directory.path()).is_empty());
}

#[test]
fn piped_text_pastes_as_json_and_markdown_files() {
    let directory = tempfile::tempdir().unwrap();
    let output = paste(
        directory.path(),
        &["--as", "json"],
        "name,count\nApples,3\n",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = paste(directory.path(), &["--as", "md"], "# Notes\n");
    assert!(output.status.success());
    let files = saved(directory.path());
    assert_eq!(files.len(), 2);
    let json = files
        .iter()
        .find(|(name, _)| name.ends_with(".json"))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json.1).unwrap(),
        serde_json::json!([{"name": "Apples", "count": 3}])
    );
    assert!(
        files
            .iter()
            .any(|(name, text)| name.ends_with(".md") && text == "# Notes\n")
    );
    let output = paste(directory.path(), &["--as", "png"], "not an image");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("needs an image"));
}
