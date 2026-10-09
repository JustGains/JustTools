//! Native preferences. Independent of console handles inherited by the tray process.
use super::*;
use windows::Win32::System::SystemServices::{SS_ETCHEDHORZ, SS_ICON};
use windows::Win32::{
    Graphics::Gdi::*,
    UI::{Controls::*, HiDpi::*},
};

const CLASS: PCWSTR = w!("JustTools.SmartPaste.Settings.v1");
const IGNORE: i32 = 101;
const GITIGNORE: i32 = 102;
const TEXT: i32 = 103;
const STARTUP: i32 = 104;
const EXCLUDES: i32 = 105;

pub(super) fn ready_window() -> Option<HWND> {
    unsafe {
        FindWindowW(CLASS, None)
            .ok()
            .filter(|window| !GetPropW(*window, w!("JustPaste.Settings.Ready")).is_invalid())
    }
}

struct State {
    fonts: Vec<HFONT>,
    background: HBRUSH,
}
impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            for font in &self.fonts {
                let _ = DeleteObject((*font).into());
            }
            let _ = DeleteObject(self.background.into());
        }
    }
}

fn read_text(window: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(window) }.clamp(0, 4096) as usize;
    let mut text = vec![0; length + 1];
    let count = unsafe { GetWindowTextW(window, &mut text) }.max(0) as usize;
    String::from_utf16_lossy(&text[..count])
}

fn save(window: HWND) -> Result<(), String> {
    unsafe {
        let exclusions = read_text(GetDlgItem(Some(window), EXCLUDES).map_err(|e| e.to_string())?);
        let exclusions = super::super::exclusions(&exclusions)?.join(";");
        let checked = |id| IsDlgButtonChecked(window, id) == BST_CHECKED.0;
        let value = |id| if checked(id) { "true" } else { "false" };
        crate::preferences::Store::load()
            .map_err(|e| e.to_string())?
            .set_many(
                super::super::TOOL,
                &[
                    ("ignore_node_modules", Some(value(IGNORE))),
                    ("gitignore", Some(value(GITIGNORE))),
                    ("text", Some(value(TEXT))),
                    (
                        "excludes",
                        (!exclusions.is_empty()).then_some(exclusions.as_str()),
                    ),
                ],
            )
            .map_err(|e| e.to_string())?;
        if checked(STARTUP) != startup_enabled() {
            super::startup(checked(STARTUP))?;
        }
    }
    Ok(())
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_NCCREATE => {
                let create = &*(lp.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
                DefWindowProcW(window, message, wp, lp)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *const State;
                if state.is_null() {
                    return DefWindowProcW(window, message, wp, lp);
                }
                let dc = HDC(wp.0 as *mut _);
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(
                    dc,
                    COLORREF(if GetDlgCtrlID(HWND(lp.0 as *mut _)) >= 200 {
                        0x00766c66
                    } else {
                        0x003c2925
                    }),
                );
                LRESULT((*state).background.0 as isize)
            }
            WM_COMMAND => {
                match wp.0 as u16 as i32 {
                    1 => match save(window) {
                        Ok(()) => {
                            let _ = DestroyWindow(window);
                        }
                        Err(error) => {
                            let text = wide(std::ffi::OsStr::new(&error));
                            MessageBoxW(
                                Some(window),
                                PCWSTR(text.as_ptr()),
                                w!("Check your settings"),
                                MB_OK | MB_ICONWARNING,
                            );
                        }
                    },
                    2 => {
                        let _ = DestroyWindow(window);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(window);
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

pub(super) fn show() -> Result<(), String> {
    unsafe {
        if let Ok(existing) = FindWindowW(CLASS, None) {
            let _ = ShowWindow(existing, SW_RESTORE);
            let _ = SetForegroundWindow(existing);
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let scale = GetDpiForSystem() as f32 / 96.0;
        let px = |value: i32| (value as f32 * scale).round() as i32;
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let mut state = Box::new(State {
            fonts: Vec::new(),
            background: CreateSolidBrush(COLORREF(0x00ffffff)),
        });
        for (height, weight) in [(15, 400), (27, 600), (13, 400), (12, 600)] {
            let mut info = LOGFONTW {
                lfHeight: -px(height),
                lfWeight: weight,
                ..Default::default()
            };
            fill(&mut info.lfFaceName, "Segoe UI");
            state.fonts.push(CreateFontIndirectW(&info));
        }
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: state.background,
            hIcon: app_icon(px(32)),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err("Cannot register the settings window".into());
        }
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut bounds = RECT {
            right: px(620),
            bottom: px(612),
            ..Default::default()
        };
        AdjustWindowRectEx(&mut bounds, style, false, WS_EX_CONTROLPARENT)
            .map_err(|e| e.to_string())?;
        let window = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            CLASS,
            w!("JustPaste settings"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            Some(module.into()),
            Some((&mut *state as *mut State).cast()),
        )
        .map_err(|e| e.to_string())?;
        let create = |class: PCWSTR,
                      label: &str,
                      id: i32,
                      area: [i32; 4],
                      extra: WINDOW_STYLE,
                      font: usize|
         -> Result<HWND, String> {
            let title = wide(std::ffi::OsStr::new(label));
            let control = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR(title.as_ptr()),
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
                Some(WPARAM(state.fonts[font].0 as usize)),
                Some(LPARAM(1)),
            );
            let _ = SetWindowTheme(control, w!("Explorer"), None);
            Ok(control)
        };
        let result = (|| -> Result<(), String> {
            let icon = create(
                w!("STATIC"),
                "",
                10,
                [32, 32, 48, 48],
                WINDOW_STYLE(SS_ICON.0),
                0,
            )?;
            SendMessageW(
                icon,
                STM_SETICON,
                Some(WPARAM(app_icon(px(48)).0 as usize)),
                None,
            );
            create(
                w!("STATIC"),
                "JustPaste",
                11,
                [96, 28, 480, 36],
                WINDOW_STYLE(0),
                1,
            )?;
            create(
                w!("STATIC"),
                "Your clipboard. A file in the right folder.",
                201,
                [97, 68, 475, 24],
                WINDOW_STYLE(0),
                2,
            )?;
            create(
                w!("STATIC"),
                "",
                12,
                [32, 112, 556, 1],
                WINDOW_STYLE(SS_ETCHEDHORZ.0),
                0,
            )?;
            create(
                w!("STATIC"),
                "COPY FILTERS",
                202,
                [32, 131, 540, 22],
                WINDOW_STYLE(0),
                3,
            )?;
            create(
                w!("BUTTON"),
                "Ignore node_modules",
                IGNORE,
                [32, 160, 556, 28],
                WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                0,
            )?;
            create(
                w!("STATIC"),
                "Skip dependency folders when copying files.",
                203,
                [55, 192, 530, 22],
                WINDOW_STYLE(0),
                2,
            )?;
            create(
                w!("BUTTON"),
                "Respect .gitignore",
                GITIGNORE,
                [32, 223, 556, 28],
                WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                0,
            )?;
            create(
                w!("STATIC"),
                "Use each folder's ignore rules, including nested rules.",
                204,
                [55, 255, 530, 22],
                WINDOW_STYLE(0),
                2,
            )?;
            create(
                w!("STATIC"),
                "Other names or paths to skip",
                13,
                [32, 294, 556, 24],
                WINDOW_STYLE(0),
                0,
            )?;
            let store = crate::preferences::Store::load().map_err(|e| e.to_string())?;
            let edit = create(
                w!("EDIT"),
                store.get(super::super::TOOL, "excludes").unwrap_or(""),
                EXCLUDES,
                [32, 324, 556, 30],
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
            )?;
            SendMessageW(edit, EM_SETLIMITTEXT, Some(WPARAM(4096)), None);
            create(
                w!("STATIC"),
                "Separate with semicolons. Example: .cache; build/temp",
                205,
                [32, 362, 556, 22],
                WINDOW_STYLE(0),
                2,
            )?;
            create(
                w!("STATIC"),
                "WINDOWS",
                206,
                [32, 404, 556, 22],
                WINDOW_STYLE(0),
                3,
            )?;
            create(
                w!("BUTTON"),
                "Save plain text as .txt",
                TEXT,
                [32, 434, 556, 28],
                WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                0,
            )?;
            create(
                w!("BUTTON"),
                "Start automatically at sign-in",
                STARTUP,
                [32, 472, 556, 28],
                WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                0,
            )?;
            create(
                w!("STATIC"),
                "Images and links are always enabled. Originals are kept.",
                207,
                [32, 516, 556, 22],
                WINDOW_STYLE(0),
                2,
            )?;
            create(
                w!("BUTTON"),
                "Cancel",
                2,
                [326, 556, 100, 36],
                WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                0,
            )?;
            create(
                w!("BUTTON"),
                "Save changes",
                1,
                [438, 556, 150, 36],
                WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                0,
            )?;
            for (id, checked) in [
                (IGNORE, setting("ignore_node_modules")),
                (GITIGNORE, setting("gitignore")),
                (TEXT, setting("text")),
                (STARTUP, startup_enabled()),
            ] {
                CheckDlgButton(
                    window,
                    id,
                    if checked { BST_CHECKED } else { BST_UNCHECKED },
                )
                .map_err(|e| e.to_string())?;
            }
            SendMessageW(
                window,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(app_icon(px(16)).0 as isize)),
            );
            SendMessageW(
                window,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(app_icon(px(32)).0 as isize)),
            );
            let _ = ShowWindow(window, SW_SHOW);
            SetPropW(
                window,
                w!("JustPaste.Settings.Ready"),
                Some(HANDLE(window.0)),
            )
            .map_err(|e| e.to_string())?;
            let _ = SetForegroundWindow(window);
            let _ = SetFocus(Some(
                GetDlgItem(Some(window), IGNORE).map_err(|e| e.to_string())?,
            ));
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
        let _ = UnregisterClassW(CLASS, Some(module.into()));
        result
    }
}
