#![cfg(windows)]

use std::{process::Command, time::Duration};
use windows::{
    Win32::{
        Foundation::*,
        System::{JobObjects::*, Threading::*},
    },
    core::{PCWSTR, PWSTR},
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn binary() -> String {
    std::env::var("JUSTTOOLS_LIFECYCLE_BINARY")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_just").into())
}

fn status() -> String {
    let output = Command::new(binary())
        .args(["paste", "--status"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

struct StopResident;
impl Drop for StopResident {
    fn drop(&mut self) {
        let _ = Command::new(binary()).args(["paste", "--disable"]).output();
        for _ in 0..50 {
            if status().contains("Smart Paste: stopped") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[test]
#[ignore = "starts and stops the real Smart Paste resident; requires it to be stopped first"]
fn resident_survives_launching_terminal_job_closing() {
    assert!(
        status().contains("Smart Paste: stopped"),
        "stop Smart Paste before this test"
    );
    let _stop = StopResident;
    unsafe {
        let job = Handle(CreateJobObjectW(None, PCWSTR::null()).unwrap());
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
        .unwrap();
        let mut command: Vec<u16> = format!("\"{}\" paste --enable", binary())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut process = PROCESS_INFORMATION::default();
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            None,
            PCWSTR::null(),
            &startup,
            &mut process,
        )
        .unwrap();
        let process_handle = Handle(process.hProcess);
        let thread = Handle(process.hThread);
        AssignProcessToJobObject(job.0, process_handle.0).unwrap();
        assert_ne!(ResumeThread(thread.0), u32::MAX);
        assert_eq!(WaitForSingleObject(process_handle.0, 15000), WAIT_OBJECT_0);
        let mut exit_code = 0;
        GetExitCodeProcess(process_handle.0, &mut exit_code).unwrap();
        assert_eq!(exit_code, 0, "launcher failed");
        assert!(status().contains("Smart Paste: running"));
        drop(job); // Closing a terminal/session must not kill the resident.
        std::thread::sleep(Duration::from_millis(700));
        let state = status();
        assert!(
            state.contains("Smart Paste: running"),
            "resident died with its launcher: {state}"
        );
        assert!(state.contains("Paused: false"));
    }
}
