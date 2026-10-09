//! A delayed, non-modal progress window. The paste worker never waits for UI.
use super::*;
use crate::progress::Reporter;
use std::time::{Duration, Instant};
use windows::Win32::{
    Graphics::Gdi::*,
    UI::{Controls::*, HiDpi::*},
};

const CLASS: PCWSTR = w!("JustTools.SmartPaste.Progress.v1");
const DELAY: Duration = Duration::from_millis(900);
struct View {
    reporter: Reporter,
    directory: PathBuf,
    font: HFONT,
    shown: bool,
    hidden: bool,
    completed: Option<Instant>,
}
impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.font.into());
        }
    }
}

pub(super) fn start(reporter: Reporter, directory: PathBuf) {
    std::thread::spawn(move || {
        if let Err(error) = show(reporter, directory) {
            notify(
                format!("Progress window: {error}. Paste continues; Cancel is in the tray menu."),
                true,
            );
        }
    });
}
fn text(window: HWND, id: i32, value: &str) {
    unsafe {
        if let Ok(control) = GetDlgItem(Some(window), id) {
            let value = wide(std::ffi::OsStr::new(value));
            let _ = SetWindowTextW(control, PCWSTR(value.as_ptr()));
        }
    }
}
unsafe extern "system" fn procedure(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut View;
        if pointer.is_null() {
            return DefWindowProcW(window, message, wp, lp);
        }
        match message {
            WM_TIMER => {
                let view = &mut *pointer;
                let state = view.reporter.snapshot();
                if state.finished.is_some() && !view.shown {
                    let _ = DestroyWindow(window);
                    return LRESULT(0);
                }
                if !view.shown && !view.hidden && state.started.elapsed() >= DELAY {
                    let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
                    view.shown = true;
                }
                let heading = state
                    .percent
                    .map_or_else(|| state.stage.clone(), |p| format!("{}  {p}%", state.stage));
                text(window, 101, &heading);
                text(window, 102, &state.detail);
                let elapsed = state.started.elapsed().as_secs();
                text(
                    window,
                    103,
                    &format!("{}:{:02} elapsed", elapsed / 60, elapsed % 60),
                );
                if let Ok(bar) = GetDlgItem(Some(window), 104) {
                    let indeterminate = state.percent.is_none() && state.finished.is_none();
                    let style = GetWindowLongW(bar, GWL_STYLE) as u32;
                    SetWindowLongW(
                        bar,
                        GWL_STYLE,
                        if indeterminate {
                            style | PBS_MARQUEE
                        } else {
                            style & !PBS_MARQUEE
                        } as i32,
                    );
                    SendMessageW(
                        bar,
                        PBM_SETMARQUEE,
                        Some(WPARAM(usize::from(indeterminate))),
                        Some(LPARAM(35)),
                    );
                    if let Some(percent) = state.percent {
                        SendMessageW(bar, PBM_SETPOS, Some(WPARAM(percent as usize)), None);
                    }
                }
                if let Some(success) = state.finished {
                    text(window, 1, "Close");
                    if let Ok(button) = GetDlgItem(Some(window), 1) {
                        let _ = EnableWindow(button, true);
                    }
                    let finished = view.completed.get_or_insert_with(Instant::now);
                    if view.hidden || (success && finished.elapsed() >= Duration::from_secs(2)) {
                        let _ = DestroyWindow(window);
                    }
                }
                LRESULT(0)
            }
            WM_COMMAND if wp.0 as u16 == 1 => {
                let view = &mut *pointer;
                if view.reporter.snapshot().finished.is_some() {
                    let _ = DestroyWindow(window);
                } else {
                    view.reporter.cancel();
                    text(window, 1, "Cancelling…");
                    if let Ok(button) = GetDlgItem(Some(window), 1) {
                        let _ = EnableWindow(button, false);
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let view = &mut *pointer;
                // Closing the window hides progress; cancellation is explicit.
                if view.reporter.snapshot().finished.is_some() {
                    let _ = DestroyWindow(window);
                } else {
                    view.hidden = true;
                    let _ = ShowWindow(window, SW_HIDE);
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                SetBkMode(HDC(wp.0 as *mut _), TRANSPARENT);
                LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}
fn show(reporter: Reporter, directory: PathBuf) -> Result<(), String> {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let scale = GetDpiForSystem() as f32 / 96.0;
        let px = |n: i32| (n as f32 * scale).round() as i32;
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        let _ = InitCommonControlsEx(&controls);
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            hIcon: app_icon(px(32)),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err("cannot create progress window class".into());
        }
        let mut info = LOGFONTW {
            lfHeight: -px(15),
            ..Default::default()
        };
        fill(&mut info.lfFaceName, "Segoe UI");
        let mut view = Box::new(View {
            reporter,
            directory,
            font: CreateFontIndirectW(&info),
            shown: false,
            hidden: false,
            completed: None,
        });
        let style = WS_CAPTION | WS_SYSMENU;
        let mut bounds = RECT {
            right: px(480),
            bottom: px(250),
            ..Default::default()
        };
        AdjustWindowRectEx(&mut bounds, style, false, WS_EX_TOOLWINDOW)
            .map_err(|e| e.to_string())?;
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_CONTROLPARENT,
            CLASS,
            w!("JustPaste"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            Some(module.into()),
            Some((&mut *view as *mut View).cast()),
        )
        .map_err(|e| e.to_string())?;
        let create = |class, label: &str, id, area: [i32; 4], extra| -> Result<(), String> {
            let label = wide(std::ffi::OsStr::new(label));
            let control = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR(label.as_ptr()),
                WS_CHILD | WS_VISIBLE | extra,
                px(area[0]),
                px(area[1]),
                px(area[2]),
                px(area[3]),
                Some(window),
                Some(HMENU(id as usize as *mut _)),
                Some(module.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(view.font.0 as usize)),
                Some(LPARAM(1)),
            );
            Ok(())
        };
        let result = (|| {
            create(
                w!("STATIC"),
                "Preparing your paste…",
                101,
                [24, 22, 432, 26],
                WINDOW_STYLE(0),
            )?;
            create(
                w!("STATIC"),
                &format!("To: {}", view.directory.display()),
                105,
                [24, 57, 432, 22],
                WINDOW_STYLE(0x4000),
            )?; // SS_PATHELLIPSIS
            create(
                PROGRESS_CLASSW,
                "",
                104,
                [24, 94, 432, 9],
                WINDOW_STYLE(PBS_MARQUEE),
            )?;
            create(w!("STATIC"), "", 102, [24, 117, 432, 58], WINDOW_STYLE(0))?;
            create(w!("STATIC"), "", 103, [24, 205, 250, 22], WINDOW_STYLE(0))?;
            create(
                w!("BUTTON"),
                "Cancel paste",
                1,
                [320, 193, 136, 34],
                WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            )?;
            SetTimer(Some(window), 1, 150, None);
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                if !IsDialogMessageW(window, &message).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            Ok(())
        })();
        if IsWindow(Some(window)).as_bool() {
            let _ = DestroyWindow(window);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "opens a delayed progress window and exercises its Cancel and Close buttons"]
    fn native_progress_delays_then_shows_stream_progress_and_cancels_only_its_operation() {
        unsafe {
            assert!(
                FindWindowW(CLASS, None).is_err(),
                "finish the current paste first"
            );
            let operation = crate::progress::Operation::begin();
            let reporter = operation.reporter.clone();
            reporter.report("Downloading media…", "50% of this stream", Some(50));
            let ui = std::thread::spawn(move || show(reporter, PathBuf::from("C:\\Paste test")));
            let window = (0..100)
                .find_map(|_| {
                    let window = FindWindowW(CLASS, None).ok();
                    if window.is_none() {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    window
                })
                .expect("progress window was not created");
            assert!(
                !IsWindowVisible(window).as_bool(),
                "quick pastes must stay quiet"
            );
            std::thread::sleep(Duration::from_millis(1200));
            assert!(
                IsWindowVisible(window).as_bool(),
                "long paste must show progress"
            );
            let bar = GetDlgItem(Some(window), 104).unwrap();
            assert_eq!(SendMessageW(bar, PBM_GETPOS, None, None).0, 50);
            SendMessageW(window, WM_COMMAND, Some(WPARAM(1)), None);
            assert!(crate::progress::cancelled());
            operation.finish(&Err("Cancelled; source files were kept.".into()));
            SendMessageW(window, WM_COMMAND, Some(WPARAM(1)), None);
            assert!(ui.join().unwrap().is_ok());
            drop(operation);
            assert!(!crate::progress::cancelled());
        }
    }
}
