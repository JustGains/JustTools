#![cfg(windows)]

use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::*,
        System::{JobObjects::*, Threading::*},
        UI::WindowsAndMessaging::*,
    },
    core::{PCWSTR, PWSTR, w},
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[test]
#[ignore = "opens native settings through a temporary tray resident on an interactive Windows desktop"]
fn tray_settings_opens_inside_a_job_that_forbids_breakaway() {
    unsafe {
        assert!(
            FindWindowW(w!("JustTools.SmartPaste.v1"), None).is_err(),
            "stop Smart Paste before this test"
        );
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
        let job = Handle(CreateJobObjectW(None, PCWSTR::null()).unwrap());
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // Deliberately omit BREAKAWAY_OK: settings must work in restricted jobs.
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
        .unwrap();
        use std::os::windows::ffi::OsStrExt;
        let mut environment: Vec<_> = std::env::vars_os()
            .filter(|(key, _)| {
                !key.eq_ignore_ascii_case("JUSTTOOLS_DEFAULTS")
                    && !key.eq_ignore_ascii_case("JUSTTOOLS_NO_DOWNLOAD")
            })
            .chain([
                (
                    "JUSTTOOLS_DEFAULTS".into(),
                    preferences.as_os_str().to_owned(),
                ),
                ("JUSTTOOLS_NO_DOWNLOAD".into(), "1".into()),
            ])
            .collect();
        environment.sort_by_key(|(key, _)| key.to_string_lossy().to_ascii_uppercase());
        let mut environment: Vec<u16> = environment
            .iter()
            .flat_map(|(key, value)| {
                key.encode_wide()
                    .chain(Some(b'=' as u16))
                    .chain(value.encode_wide())
                    .chain(Some(0))
            })
            .collect();
        environment.push(0);
        let mut command: Vec<u16> = format!("\"{binary}\" paste --background")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut child = PROCESS_INFORMATION::default();
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            Some(environment.as_ptr().cast()),
            PCWSTR::null(),
            &startup,
            &mut child,
        )
        .unwrap();
        let process = Handle(child.hProcess);
        let thread = Handle(child.hThread);
        AssignProcessToJobObject(job.0, process.0).unwrap();
        assert_ne!(ResumeThread(thread.0), u32::MAX);
        let resident = (0..100)
            .find_map(|_| {
                let ready = FindWindowW(w!("JustTools.SmartPaste.v1"), None)
                    .ok()
                    .filter(|window| !GetPropW(*window, w!("JustPaste.Ready")).is_invalid());
                if ready.is_none() {
                    std::thread::sleep(Duration::from_millis(100));
                }
                ready
            })
            .expect("temporary resident did not become ready");
        for _ in 0..2 {
            PostMessageW(
                Some(resident),
                WM_APP + 1,
                WPARAM(0),
                LPARAM(WM_LBUTTONUP as isize),
            )
            .unwrap();
            let window = (0..50)
                .find_map(|_| {
                    let ready = FindWindowW(w!("JustTools.SmartPaste.Settings.v1"), None)
                        .ok()
                        .filter(|window| {
                            !GetPropW(*window, w!("JustPaste.Settings.Ready")).is_invalid()
                        });
                    if ready.is_none() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    ready
                })
                .unwrap_or_else(|| {
                    let mut text = [0; 1024];
                    let count = GetWindowTextW(resident, &mut text);
                    panic!(
                        "tray settings failed in restricted job: {}",
                        String::from_utf16_lossy(&text[..count as usize])
                    );
                });
            assert!(IsWindowVisible(window).as_bool());
            let mut owner = 0;
            GetWindowThreadProcessId(window, Some(&mut owner));
            assert_ne!(owner, child.dwProcessId);
            let settings =
                Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, owner).unwrap());
            let mut in_job = Default::default();
            IsProcessInJob(settings.0, Some(job.0), &mut in_job).unwrap();
            assert!(
                in_job.as_bool(),
                "settings unexpectedly escaped the resident job"
            );
            PostMessageW(Some(window), WM_COMMAND, WPARAM(2), LPARAM(0)).unwrap();
            for _ in 0..50 {
                if !IsWindow(Some(window)).as_bool() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(
                !IsWindow(Some(window)).as_bool(),
                "Cancel did not close settings"
            );
            assert_eq!(
                WaitForSingleObject(process.0, 0),
                WAIT_TIMEOUT,
                "closing settings stopped the resident"
            );
            assert_eq!(std::fs::read_to_string(&preferences).unwrap(), original);
        }
        drop(job);
        assert_eq!(WaitForSingleObject(process.0, 5000), WAIT_OBJECT_0);
    }
}

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
        // GetWindowTextW cannot read another process's edit-control text.
        let mut count = 0;
        let read = SendMessageTimeoutW(
            edit,
            WM_GETTEXT,
            WPARAM(text.len()),
            LPARAM(text.as_mut_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            Some(&mut count),
        );
        let value = String::from_utf16_lossy(&text[..count.min(text.len())]);
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
        assert_ne!(read.0, 0, "could not read the settings field");
        assert_eq!(value, "build/temp");
        assert_eq!(std::fs::read_to_string(preferences).unwrap(), original);
    }
}
