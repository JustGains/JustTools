#![cfg(windows)]

use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};
use windows::{
    Win32::{Foundation::*, System::Threading::*, UI::WindowsAndMessaging::*},
    core::w,
};

#[test]
#[ignore = "opens and closes the native settings window on an interactive Windows desktop"]
fn native_settings_opens_without_console_handles_and_cancel_preserves_defaults() {
    unsafe {
        assert!(
            FindWindowW(w!("JustTools.SmartPaste.Settings.v1"), None).is_err(),
            "close settings before this test"
        );
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("defaults.toml");
        let original = "version = 1\n[tools.justpaste]\nexcludes = 'build/temp'\n";
        std::fs::write(&preferences, original).unwrap();
        let binary = std::env::var("JUSTTOOLS_LIFECYCLE_BINARY")
            .unwrap_or_else(|_| env!("CARGO_BIN_EXE_just").into());
        let mut child = Command::new(&binary)
            .args(["paste", "--settings"])
            .env("JUSTTOOLS_DEFAULTS", &preferences)
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let window = (0..100).find_map(|_| {
            let ready = FindWindowW(w!("JustTools.SmartPaste.Settings.v1"), None)
                .ok()
                .filter(|window| !GetPropW(*window, w!("JustPaste.Settings.Ready")).is_invalid());
            if ready.is_none() {
                std::thread::sleep(Duration::from_millis(100));
            }
            ready
        });
        let Some(window) = window else {
            let _ = child.kill();
            let _ = child.wait();
            panic!("settings did not become ready with null standard handles");
        };
        let visible = IsWindowVisible(window).as_bool();
        let mut owner = 0;
        GetWindowThreadProcessId(window, Some(&mut owner));
        let icon = SendMessageW(window, WM_GETICON, Some(WPARAM(ICON_BIG as usize)), None);
        let edit = GetDlgItem(Some(window), 105).unwrap();
        let mut text = [0; 256];
        let count = GetWindowTextW(edit, &mut text);
        let value = String::from_utf16_lossy(&text[..count as usize]);
        // The product's Cancel command must close without persisting anything.
        PostMessageW(Some(window), WM_COMMAND, WPARAM(2), LPARAM(0)).unwrap();
        let mut ended = false;
        for _ in 0..50 {
            if child.try_wait().unwrap().is_some() {
                ended = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !ended {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(ended, "Cancel did not close settings");
        assert_eq!(owner, child.id());
        assert!(visible, "settings window stayed hidden");
        assert_ne!(icon.0, 0, "the new embedded icon is missing");
        assert_eq!(value, "build/temp");
        assert_eq!(std::fs::read_to_string(preferences).unwrap(), original);
    }
}
