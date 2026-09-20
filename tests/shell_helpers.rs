use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_just")
}

fn run_at(directory: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .current_dir(directory)
        .env_remove("JUSTTOOLS_MKCD_RESULT")
        .args(args)
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn mkcd_headless_creation_and_collisions() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        !run_at(temp.path(), &["mkcd", "missing/child"])
            .status
            .success()
    );
    assert!(!temp.path().join("missing").exists());
    let created = run_at(
        temp.path(),
        &["mkcd", "-p", "--print-path", "missing/child"],
    );
    assert_success(&created);
    assert!(created.stderr.is_empty());
    assert_eq!(
        String::from_utf8(created.stdout).unwrap().trim(),
        temp.path()
            .join("missing/child")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert!(
        !run_at(temp.path(), &["mkcd", "missing/child"])
            .status
            .success()
    );
    assert_success(&run_at(temp.path(), &["mkcd", "-p", "missing/child"]));
    fs::write(temp.path().join("kept"), "original").unwrap();
    assert!(
        !run_at(temp.path(), &["mkcd", "-p", "kept"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("kept")).unwrap(),
        "original"
    );
    assert_success(&run_at(
        temp.path(),
        &["mkcd", "--", "--literal [x] ' space"],
    ));
}

#[test]
fn mkcd_invalid_arguments_and_help_do_not_create_anything() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["mkcd"],
        vec!["mkcd", ""],
        vec!["mkcd", "first", "second"],
        vec!["mkcd", "--unknown", "first"],
    ] {
        let output = run_at(temp.path(), &args);
        assert_eq!(output.status.code(), Some(2));
    }
    assert_success(&run_at(temp.path(), &["mkcd", "--help"]));
    assert_success(&run_at(temp.path(), &["mkcd", "--version"]));
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    for args in [
        vec!["init"],
        vec!["init", "cmd"],
        vec!["init", "bash", "extra"],
    ] {
        assert_eq!(run_at(temp.path(), &args).status.code(), Some(2));
    }
}

#[test]
fn mkcd_handoff_contains_literal_path_only_after_success() {
    let temp = tempfile::tempdir().unwrap();
    let result = temp.path().join("result");
    let invoke = |args: &[&str]| {
        Command::new(binary())
            .current_dir(temp.path())
            .env("JUSTTOOLS_MKCD_RESULT", &result)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(!invoke(&["mkcd", "new"]).status.success());
    assert!(!temp.path().join("new").exists());
    fs::write(&result, "").unwrap();
    assert_success(&invoke(&["mkcd", "new"]));
    let mut expected = temp
        .path()
        .join("new")
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .as_bytes()
        .to_vec();
    expected.push(0);
    assert_eq!(fs::read(&result).unwrap(), expected);
    assert!(!invoke(&["mkcd", "new"]).status.success());
    assert!(fs::read(&result).unwrap().is_empty());
}

fn integration(shell: &str) -> String {
    let output = Command::new(binary())
        .args(["init", shell])
        .output()
        .unwrap();
    assert_success(&output);
    String::from_utf8(output.stdout).unwrap()
}

#[test]
#[cfg(windows)]
fn mkcd_windows_install_works_without_init_and_preserves_pipelines() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin [x] ' quoted");
    let installed = Command::new(binary())
        .args(["install", "--bin-dir"])
        .arg(&bin)
        .args(["--yes", "--no-path"])
        .output()
        .unwrap();
    assert_success(&installed);
    // Also exercise an upgrade: every generated script is a managed install file.
    let reinstall = Command::new(binary())
        .args(["install", "--bin-dir"])
        .arg(&bin)
        .args(["--yes", "--no-path"])
        .output()
        .unwrap();
    assert_success(&reinstall);
    for shell in ["powershell", "pwsh"] {
        for entrypoint in [
            "mkcd",
            "justmkcd",
            "just mkcd",
            "claude_",
            "codex_",
            "pipeline",
        ] {
            let work = tempfile::tempdir().unwrap();
            let script = work.path().join("run.ps1");
            let command = match entrypoint {
                "claude_" => {
                    "claude_ --resume 'two words'\nif ($LASTEXITCODE -ne 19 -or ($global:forwarded -join '|') -ne '--dangerously-skip-permissions|--resume|two words') { throw 'claude forwarding failed' }"
                }
                "codex_" => {
                    "codex_ resume 'two words'\nif ($LASTEXITCODE -ne 23 -or ($global:forwarded -join '|') -ne '--yolo|resume|two words') { throw 'codex forwarding failed' }"
                }
                "pipeline" => {
                    "$formatted = '{\"b\":2,\"a\":1}' | just json --minify\nif ($LASTEXITCODE -ne 0 -or $formatted -ne '{\"b\":2,\"a\":1}') { throw 'native stdin forwarding failed' }"
                }
                _ => "",
            };
            let command = if command.is_empty() {
                format!(
                    "{entrypoint} 'new folder [x] café'\nif ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFileName((Get-Location).Path) -ne 'new folder [x] café' -or -not [IO.Directory]::Exists((Get-Location).Path)) {{ throw 'automatic mkcd did not change the calling shell' }}"
                )
            } else {
                command.into()
            };
            fs::write(&script, format!("\u{feff}{}\n{command}\nexit 0\n", r#"
$ErrorActionPreference = 'Stop'
foreach ($name in @('mkcd', 'justmkcd', 'just', 'claude_', 'codex_')) {
    if ((Get-Command $name).CommandType -ne 'ExternalScript') { throw "Expected automatic script launcher: $name" }
}
function global:claude { $global:forwarded = @($args); $global:LASTEXITCODE = 19 }
function global:codex { $global:forwarded = @($args); $global:LASTEXITCODE = 23 }
"#)).unwrap();
            let mut search_path = vec![bin.clone()];
            search_path.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            let output = Command::new(shell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(&script)
                .env("PATH", std::env::join_paths(search_path).unwrap())
                .current_dir(work.path())
                .output()
                .unwrap();
            assert_success(&output);
        }
    }
}

#[test]
fn mkcd_powershell_changes_parent_and_shortcuts_forward_arguments() {
    let shells: &[&str] = if cfg!(windows) {
        &["powershell", "pwsh"]
    } else {
        &["pwsh"]
    };
    for shell in shells {
        if Command::new(shell).arg("-Version").output().is_err() {
            if cfg!(windows) {
                panic!("required Windows shell missing: {shell}");
            }
            continue;
        }
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("test.ps1");
        let body = format!(
            "\u{feff}{}\n{}",
            integration("powershell"),
            r#"
$ErrorActionPreference = 'Stop'
$start = (Get-Location).Path
$env:JUSTTOOLS_MKCD_RESULT = 'previous-value'
mkcd 'my folder [x] café'
if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFileName((Get-Location).Path) -ne 'my folder [x] café' -or -not [IO.Directory]::Exists((Get-Location).Path)) { throw 'mkcd did not enter literal directory' }
if ($env:JUSTTOOLS_MKCD_RESULT -ne 'previous-value') { throw 'environment was not restored' }
$inside = (Get-Location).Path
mkcd --help | Out-Null
mkcd --version | Out-Null
just --version | Out-Null
if ((Get-Location).Path -ne $inside) { throw 'help/version moved directory' }
mkcd --print-path 'printed' | Out-Null
if ((Get-Location).Path -ne $inside) { throw 'print-path moved directory' }
mkcd 'printed' 2>$null
if ($LASTEXITCODE -eq 0 -or (Get-Location).Path -ne $inside) { throw 'collision moved directory or reported success' }
mkcd 'first' 'second' 2>$null
if ($LASTEXITCODE -ne 2 -or (Test-Path -LiteralPath 'first')) { throw 'invalid arguments changed filesystem' }
just mkcd -p 'parent/child'
if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFileName((Get-Location).Path) -ne 'child') { throw 'short dispatch did not enter' }
justmkcd '--' '--literal'
if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFileName((Get-Location).Path) -ne '--literal') { throw 'alias did not enter' }
function global:claude { $script:forwarded = @($args); $global:LASTEXITCODE = 19 }
function global:codex { $script:forwarded = @($args); $global:LASTEXITCODE = 23 }
claude_ --resume 'two words' '$(literal)' ''
if ($LASTEXITCODE -ne 19 -or ($script:forwarded | ConvertTo-Json -Compress) -ne '["--dangerously-skip-permissions","--resume","two words","$(literal)",""]') { throw 'claude shortcut changed arguments/status' }
codex_ resume 'two words' '$(literal)' ''
if ($LASTEXITCODE -ne 23 -or ($script:forwarded | ConvertTo-Json -Compress) -ne '["--yolo","resume","two words","$(literal)",""]') { throw 'codex shortcut changed arguments/status' }
Set-Location -LiteralPath $start
exit 0
"#
        );
        fs::write(&script, body).unwrap();
        let output = Command::new(shell)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert_success(&output);
    }
}

#[test]
fn mkcd_bash_zsh_changes_parent_and_shortcuts_forward_arguments() {
    let shells: Vec<String> = if cfg!(windows) {
        vec![format!(
            "{}/Git/bin/bash.exe",
            std::env::var("ProgramFiles").unwrap_or_else(|_| "C:/Program Files".into())
        )]
    } else {
        vec!["bash".into(), "zsh".into()]
    };
    for shell in shells {
        if Command::new(&shell).arg("--version").output().is_err() {
            eprintln!("skipping unavailable shell {shell}");
            continue;
        }
        let temp = tempfile::tempdir().unwrap();
        let body = format!(
            "{}\n{}",
            integration(if shell.ends_with("zsh") {
                "zsh"
            } else {
                "bash"
            }),
            include_str!("shell/posix.sh")
        );
        let output = Command::new(&shell)
            .args(["-c", &body])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert_success(&output);
    }
}

#[test]
fn mkcd_fish_changes_parent_and_shortcuts_forward_arguments() {
    if Command::new("fish").arg("--version").output().is_err() {
        eprintln!("skipping unavailable shell fish");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let body = format!(
        "{}\n{}",
        integration("fish"),
        include_str!("shell/fish.fish")
    );
    let output = Command::new("fish")
        .args(["--no-config", "-c", &body])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_success(&output);
}
