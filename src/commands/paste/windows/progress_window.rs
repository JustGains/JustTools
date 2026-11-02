//! A compact status card for long pastes, in the Windows 11 style. It waits
//! before appearing, so quick pastes stay quiet, and the paste never waits
//! for it. Closing the card only hides progress; Cancel is explicit.
use super::theme::{self, Palette, glyph};
use super::*;
use crate::progress::{Reporter, Snapshot as State};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::{Graphics::Gdi::*, UI::Controls::WM_MOUSELEAVE};

const CLASS: PCWSTR = w!("JustTools.SmartPaste.Progress.v2");
/// Pastes faster than this never show the card.
const DELAY: Duration = Duration::from_millis(900);
const SUCCESS_LINGER: Duration = Duration::from_millis(1800);
const FAILURE_LINGER: Duration = Duration::from_secs(15);
/// The card's size and spacing, in 96-DPI pixels.
const WIDTH: i32 = 380;
const HEIGHT: i32 = 112;
const TEXT_LEFT: i32 = 52;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hot {
    None,
    Close,
    Cancel,
}

/// The card for one operation.
pub(super) struct Card {
    thread: std::thread::JoinHandle<()>,
    shown: Arc<AtomicBool>,
}

impl Card {
    /// Whether the card appeared, so it already reports the outcome.
    pub fn shown(&self) -> bool {
        self.shown.load(Ordering::SeqCst)
    }

    /// Wait until the card has shown the outcome and closed.
    pub fn wait(self) {
        let _ = self.thread.join();
    }
}

struct View {
    reporter: Reporter,
    folder: String,
    directory: PathBuf,
    /// Show failures even when the card never appeared: nothing else will.
    report_failure: bool,
    palette: Palette,
    scale: f32,
    title: HFONT,
    body: HFONT,
    status: HFONT,
    small: HFONT,
    icon: HICON,
    shown: Arc<AtomicBool>,
    visible: bool,
    hidden: bool,
    finished: Option<Instant>,
    hot: Hot,
    pressed: Hot,
    tracking: bool,
    /// Position of the indeterminate bar, 0–1.
    phase: f32,
    caption: String,
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            for font in [self.title, self.body, self.status, self.small] {
                let _ = DeleteObject(font.into());
            }
        }
    }
}

impl View {
    fn px(&self, value: i32) -> i32 {
        (value as f32 * self.scale).round() as i32
    }

    fn close_area(&self) -> RECT {
        RECT {
            left: self.px(WIDTH - 40),
            top: self.px(8),
            right: self.px(WIDTH - 8),
            bottom: self.px(40),
        }
    }

    fn cancel_area(&self) -> RECT {
        RECT {
            left: self.px(WIDTH - 16 - 84),
            top: self.px(72),
            right: self.px(WIDTH - 16),
            bottom: self.px(100),
        }
    }

    fn hit(&self, x: i32, y: i32, state: &State) -> Hot {
        let inside = |area: RECT| {
            (area.left..area.right).contains(&x) && (area.top..area.bottom).contains(&y)
        };
        if inside(self.close_area()) {
            Hot::Close
        } else if state.finished.is_none() && !state.cancelled && inside(self.cancel_area()) {
            Hot::Cancel
        } else {
            Hot::None
        }
    }

    /// The title line: the stage, with a percentage while one is known.
    fn heading(state: &State) -> String {
        match (state.percent, state.finished) {
            (Some(percent), None) => format!("{} · {percent}%", state.stage),
            _ => state.stage.clone(),
        }
    }

    fn detail(&self, state: &State) -> String {
        if state.detail.is_empty() {
            format!("Saving into {}", self.folder)
        } else {
            tidy(&state.detail, &self.directory)
        }
    }
}

pub(super) fn start(reporter: Reporter, directory: PathBuf, report_failure: bool) -> Card {
    let shown = Arc::new(AtomicBool::new(false));
    let flag = shown.clone();
    let thread = std::thread::spawn(move || {
        if let Err(error) = show(reporter, directory, report_failure, flag) {
            notify(
                format!("Progress card: {error}. The paste continues; Cancel is in the tray menu."),
                true,
            );
        }
    });
    Card { thread, shown }
}

fn fill(dc: HDC, area: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let previous_brush = SelectObject(dc, brush.into());
        let previous_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        // A null pen leaves the right and bottom edge unpainted.
        let _ = RoundRect(
            dc,
            area.left,
            area.top,
            area.right + 1,
            area.bottom + 1,
            radius * 2,
            radius * 2,
        );
        SelectObject(dc, previous_pen);
        SelectObject(dc, previous_brush);
        let _ = DeleteObject(brush.into());
    }
}

fn write(dc: HDC, text: &str, area: RECT, font: HFONT, color: COLORREF, format: DRAW_TEXT_FORMAT) {
    let mut area = area;
    let mut text: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let previous = SelectObject(dc, font.into());
        SetTextColor(dc, color);
        DrawTextW(dc, &mut text, &mut area, format | DT_NOPREFIX);
        SelectObject(dc, previous);
    }
}

fn draw(dc: HDC, view: &View, client: RECT) {
    let palette = &view.palette;
    let px = |value| view.px(value);
    let state = view.reporter.snapshot();
    fill(dc, client, 0, palette.background);
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        let status = RECT {
            left: px(16),
            top: px(16),
            right: px(40),
            bottom: px(40),
        };
        match state.finished {
            None => {
                let _ = DrawIconEx(
                    dc,
                    status.left,
                    status.top,
                    view.icon,
                    px(24),
                    px(24),
                    0,
                    None,
                    DI_NORMAL,
                );
            }
            Some(success) => {
                let (symbol, color) = if success {
                    (glyph::DONE, palette.success)
                } else if state.cancelled {
                    (glyph::CANCEL, palette.secondary)
                } else {
                    (glyph::WARNING, palette.error)
                };
                let previous = SelectObject(dc, view.status.into());
                theme::draw_glyph(dc, symbol, status, color);
                SelectObject(dc, previous);
            }
        }
    }
    write(
        dc,
        &View::heading(&state),
        RECT {
            left: px(TEXT_LEFT),
            top: px(12),
            right: px(WIDTH - 44),
            bottom: px(36),
        },
        view.title,
        palette.text,
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
    let close = view.close_area();
    if view.hot == Hot::Close {
        let color = if view.pressed == Hot::Close {
            palette.pressed
        } else {
            palette.hover
        };
        fill(dc, close, px(4), color);
    }
    unsafe {
        let previous = SelectObject(dc, view.small.into());
        theme::draw_glyph(dc, glyph::CLOSE, close, palette.secondary);
        SelectObject(dc, previous);
    }
    let running = state.finished.is_none();
    write(
        dc,
        &view.detail(&state),
        RECT {
            left: px(TEXT_LEFT),
            top: px(37),
            right: client.right - px(16),
            bottom: if running {
                px(70)
            } else {
                client.bottom - px(12)
            },
        },
        view.body,
        palette.secondary,
        DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS,
    );
    if !running {
        return;
    }
    let cancel = view.cancel_area();
    let track = RECT {
        left: px(TEXT_LEFT),
        top: px(86),
        right: cancel.left - px(20),
        bottom: px(86) + px(1).max(1),
    };
    fill(dc, track, 0, palette.track);
    let length = track.right - track.left;
    let (start, end) = match state.percent {
        Some(percent) => (0, length * percent.min(100) as i32 / 100),
        None => {
            // A third of the track sweeps across while the length is unknown.
            let segment = length / 3;
            let head = ((length + segment) as f32 * view.phase) as i32 - segment;
            (head.max(0), (head + segment).min(length))
        }
    };
    if end > start {
        fill(
            dc,
            RECT {
                left: track.left + start,
                top: px(85),
                right: track.left + end,
                bottom: px(88),
            },
            px(2),
            palette.accent,
        );
    }
    let face = match (view.hot, view.pressed) {
        (Hot::Cancel, Hot::Cancel) => palette.pressed,
        (Hot::Cancel, _) => palette.hover,
        _ => palette.control,
    };
    fill(dc, cancel, px(4), palette.stroke);
    fill(
        dc,
        RECT {
            left: cancel.left + 1,
            top: cancel.top + 1,
            right: cancel.right - 1,
            bottom: cancel.bottom - 1,
        },
        px(4),
        face,
    );
    let (label, color) = if state.cancelled {
        ("Cancelling…", palette.secondary)
    } else {
        ("Cancel", palette.text)
    };
    write(
        dc,
        label,
        cancel,
        view.body,
        color,
        DT_SINGLELINE | DT_CENTER | DT_VCENTER,
    );
}

unsafe fn paint(window: HWND, view: &View) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(window, &mut paint);
        let mut client = RECT::default();
        let _ = GetClientRect(window, &mut client);
        // Draw off screen first, so the animated bar never flickers.
        let memory = CreateCompatibleDC(Some(dc));
        let bitmap = CreateCompatibleBitmap(dc, client.right, client.bottom);
        let previous = SelectObject(memory, bitmap.into());
        draw(memory, view, client);
        let _ = BitBlt(
            dc,
            0,
            0,
            client.right,
            client.bottom,
            Some(memory),
            0,
            0,
            SRCCOPY,
        );
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        let _ = EndPaint(window, &paint);
    }
}

/// Shrink a finished card to its message, keeping its bottom edge in place.
fn fit(window: HWND, view: &View, state: &State) {
    let px = |value| view.px(value);
    let mut text: Vec<u16> = view.detail(state).encode_utf16().collect();
    let mut area = RECT {
        left: px(TEXT_LEFT),
        right: px(WIDTH - 16),
        ..Default::default()
    };
    unsafe {
        let dc = GetDC(Some(window));
        let previous = SelectObject(dc, view.body.into());
        DrawTextW(
            dc,
            &mut text,
            &mut area,
            DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        ReleaseDC(Some(window), dc);
        // Room for the title, up to four lines of detail, and the margin.
        let height = (px(37) + area.bottom.min(px(64)) + px(18)).clamp(px(76), px(HEIGHT));
        let mut bounds = RECT::default();
        let _ = GetWindowRect(window, &mut bounds);
        let _ = SetWindowPos(
            window,
            None,
            bounds.left,
            bounds.bottom - height,
            bounds.right - bounds.left,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn appear(window: HWND, view: &mut View) {
    unsafe {
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
    }
    view.visible = true;
    view.shown.store(true, Ordering::SeqCst);
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut View;
        let Some(view) = pointer.as_mut() else {
            return DefWindowProcW(window, message, wp, lp);
        };
        match message {
            WM_TIMER => {
                let state = view.reporter.snapshot();
                let now = Instant::now();
                if let Some(success) = state.finished {
                    if view.finished.is_none() {
                        fit(window, view, &state);
                    }
                    let at = *view.finished.get_or_insert(now);
                    if !view.visible {
                        if success || state.cancelled || !view.report_failure || view.hidden {
                            let _ = DestroyWindow(window);
                            return LRESULT(0);
                        }
                        appear(window, view);
                    }
                    let linger = if success || state.cancelled {
                        SUCCESS_LINGER
                    } else {
                        FAILURE_LINGER
                    };
                    if view.hot != Hot::None {
                        // Never take a message away while it is being read.
                        view.finished = Some(now);
                    } else if now.duration_since(at) >= linger {
                        let _ = DestroyWindow(window);
                        return LRESULT(0);
                    }
                } else if !view.visible && !view.hidden && state.started.elapsed() >= DELAY {
                    appear(window, view);
                }
                if view.visible {
                    view.phase = (view.phase + 0.016) % 1.0;
                    let caption = View::heading(&state);
                    if caption != view.caption {
                        // Screen readers announce the card by its title.
                        let text = wide(std::ffi::OsStr::new(&caption));
                        let _ = SetWindowTextW(window, PCWSTR(text.as_ptr()));
                        view.caption = caption;
                    }
                    let _ = InvalidateRect(Some(window), None, false);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                paint(window, view);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE => {
                let (x, y) = (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32);
                let hot = view.hit(x, y, &view.reporter.snapshot());
                if !view.tracking {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: window,
                        dwHoverTime: 0,
                    };
                    view.tracking = TrackMouseEvent(&mut track).is_ok();
                }
                if hot != view.hot {
                    view.hot = hot;
                    let _ = InvalidateRect(Some(window), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                view.tracking = false;
                view.hot = Hot::None;
                view.pressed = Hot::None;
                let _ = InvalidateRect(Some(window), None, false);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                view.pressed = view.hot;
                SetCapture(window);
                let _ = InvalidateRect(Some(window), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32);
                let state = view.reporter.snapshot();
                let clicked = view.hit(x, y, &state);
                let pressed = std::mem::replace(&mut view.pressed, Hot::None);
                match (pressed == clicked).then_some(clicked) {
                    Some(Hot::Close) if state.finished.is_some() => {
                        let _ = DestroyWindow(window);
                        return LRESULT(0);
                    }
                    Some(Hot::Close) => {
                        view.hidden = true;
                        view.visible = false;
                        let _ = ShowWindow(window, SW_HIDE);
                    }
                    Some(Hot::Cancel) => view.reporter.cancel(),
                    _ => {}
                }
                let _ = InvalidateRect(Some(window), None, false);
                LRESULT(0)
            }
            WM_CLOSE => {
                // Closing hides progress; cancelling stays a deliberate click.
                if view.reporter.snapshot().finished.is_some() {
                    let _ = DestroyWindow(window);
                } else {
                    view.hidden = true;
                    view.visible = false;
                    let _ = ShowWindow(window, SW_HIDE);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}

fn show(
    reporter: Reporter,
    directory: PathBuf,
    report_failure: bool,
    shown: Arc<AtomicBool>,
) -> Result<(), String> {
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err("cannot create the progress card".into());
        }
        // Appear on the monitor the user is working on: the one under the cursor.
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let scale = theme::monitor_dpi(point) as f32 / 96.0;
        let px = |value: i32| (value as f32 * scale).round() as i32;
        let area = theme::work_area(point);
        let folder = directory.file_name().map_or_else(
            || directory.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let mut view = Box::new(View {
            reporter,
            folder,
            directory,
            report_failure,
            palette: Palette::current(),
            scale,
            title: theme::ui_font(px(14), 600),
            body: theme::ui_font(px(12), 400),
            status: theme::icon_font(px(20)),
            small: theme::icon_font(px(10)),
            icon: app_icon(px(24)),
            shown,
            visible: false,
            hidden: false,
            finished: None,
            hot: Hot::None,
            pressed: Hot::None,
            tracking: false,
            phase: 0.0,
            caption: String::new(),
        });
        let (width, height) = (px(WIDTH), px(HEIGHT));
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            CLASS,
            w!("JustPaste"),
            WS_POPUP,
            // Bottom center, where Windows shows its own transient status;
            // notifications own the bottom-right corner.
            (area.left + area.right - width) / 2,
            area.bottom - height - px(24),
            width,
            height,
            None,
            None,
            Some(module.into()),
            Some((&mut *view as *mut View).cast()),
        )
        .map_err(|e| e.to_string())?;
        theme::round_corners(window);
        SetTimer(Some(window), 1, 33, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if IsWindow(Some(window)).as_bool() {
            let _ = DestroyWindow(window);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Option<HWND> {
        (0..100).find_map(|_| {
            let window = unsafe { FindWindowW(CLASS, None) }.ok();
            if window.is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
            window
        })
    }

    #[test]
    #[ignore = "opens the progress card and exercises its Cancel button"]
    fn card_waits_then_shows_stream_progress_and_cancels_only_its_operation() {
        unsafe {
            assert!(
                FindWindowW(CLASS, None).is_err(),
                "finish the current paste first"
            );
            let operation = crate::progress::Operation::begin();
            let reporter = operation.reporter.clone();
            reporter.report("Downloading media…", "50% of this stream", Some(50));
            let shown = Arc::new(AtomicBool::new(false));
            let flag = shown.clone();
            let ui = std::thread::spawn(move || {
                show(reporter, PathBuf::from("C:\\Paste test"), false, flag)
            });
            let window = card().expect("progress card was not created");
            assert!(
                !IsWindowVisible(window).as_bool(),
                "quick pastes must stay quiet"
            );
            std::thread::sleep(Duration::from_millis(1200));
            assert!(IsWindowVisible(window).as_bool(), "long paste must show");
            assert!(shown.load(Ordering::SeqCst));
            let view = &*(GetWindowLongPtrW(window, GWLP_USERDATA) as *const View);
            let cancel = view.cancel_area();
            let at = LPARAM(((cancel.top + 4) << 16 | (cancel.left + 4)) as isize);
            SendMessageW(window, WM_MOUSEMOVE, None, Some(at));
            SendMessageW(window, WM_LBUTTONDOWN, None, Some(at));
            SendMessageW(window, WM_LBUTTONUP, None, Some(at));
            assert!(crate::progress::cancelled());
            operation.finish(&Err("Cancelled; source files were kept.".into()));
            assert!(ui.join().unwrap().is_ok());
            drop(operation);
            assert!(!crate::progress::cancelled());
        }
    }

    #[test]
    #[ignore = "opens the progress card for a failure that finished at once"]
    fn quick_failures_appear_only_when_nothing_else_reports_them() {
        for report_failure in [false, true] {
            let operation = crate::progress::Operation::begin();
            operation.finish(&Err("The clipboard is empty.".into()));
            let shown = Arc::new(AtomicBool::new(false));
            let flag = shown.clone();
            let reporter = operation.reporter.clone();
            let ui = std::thread::spawn(move || {
                show(
                    reporter,
                    PathBuf::from("C:\\Paste test"),
                    report_failure,
                    flag,
                )
            });
            if report_failure {
                let window = card().expect("progress card was not created");
                std::thread::sleep(Duration::from_millis(200));
                assert!(unsafe { IsWindowVisible(window) }.as_bool());
                unsafe { SendMessageW(window, WM_CLOSE, None, None) };
            }
            assert!(ui.join().unwrap().is_ok());
            assert_eq!(shown.load(Ordering::SeqCst), report_failure);
        }
    }
}
