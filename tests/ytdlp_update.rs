#![cfg(windows)]

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
};

#[test]
fn update_success_failure_expiry_and_executable_identity_are_persisted_correctly() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    fs::create_dir(&state).unwrap();
    let counter = temp.path().join("calls.txt");
    let helper = temp.path().join("yt-dlp.cmd");
    let script = |code| {
        format!(
            "@echo off\r\necho check>>\"{}\"\r\necho Update fixture\r\nexit /b {code}\r\n",
            counter.display()
        )
    };
    fs::write(&helper, script(1)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/fixture.txt", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for _ in 0..8 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture").unwrap();
        }
    });
    let run = |executable: &Path| {
        let destination = tempfile::tempdir_in(temp.path()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_just"))
            .args(["paste", &url, "--output"])
            .arg(destination.path())
            .env("YTDLP_BIN", executable)
            .env("JUSTTOOLS_DEFAULTS", state.join("defaults.toml"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(destination.path().join("fixture.txt")).unwrap(),
            b"fixture"
        );
    };
    let calls = || fs::read_to_string(&counter).unwrap().lines().count();
    let age = |suffix: &str| {
        for file in fs::read_dir(&state).unwrap().map(Result::unwrap) {
            if file.file_name().to_string_lossy().ends_with(suffix) {
                fs::write(file.path(), "0").unwrap();
            }
        }
    };
    run(&helper);
    assert_eq!(calls(), 1);
    assert!(!fs::read_dir(&state).unwrap().any(|f| {
        f.unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with("success")
    }));
    run(&helper);
    assert_eq!(calls(), 1, "failures should back off briefly");
    age("attempt");
    fs::write(&helper, script(0)).unwrap();
    run(&helper);
    assert_eq!(calls(), 2);
    run(&helper);
    assert_eq!(calls(), 2, "a successful update must be cached");
    age("success");
    age("attempt");
    run(&helper);
    assert_eq!(calls(), 3, "an expired check must run again");
    let other = temp.path().join("another-yt-dlp.cmd");
    fs::copy(&helper, &other).unwrap();
    run(&other);
    assert_eq!(calls(), 4, "another executable needs its own update check");
    age("success");
    age("attempt");
    thread::scope(|scope| {
        scope.spawn(|| run(&helper));
        scope.spawn(|| run(&helper));
    });
    assert_eq!(
        calls(),
        5,
        "concurrent launches must share a single update check"
    );
    server.join().unwrap();
}
