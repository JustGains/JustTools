//! Native preferences. Independent of console handles inherited by the tray process.
use super::super::{actions, ai};
use super::*;
use windows::Win32::System::SystemServices::SS_ICON;
use windows::Win32::{
    Graphics::Gdi::*,
    UI::{Controls::*, HiDpi::*},
};

const CLASS: PCWSTR = w!("JustTools.SmartPaste.Settings.v1");
const IGNORE: i32 = 101;
const GITIGNORE: i32 = 102;
const STARTUP: i32 = 104;
const EXCLUDES: i32 = 105;
const TABS: i32 = 110;
const TEXT_ACTION: i32 = 120;
const IMAGE_ACTION: i32 = 121;
const LINK_ACTION: i32 = 122;
const MEDIA_ACTION: i32 = 123;
const PICKER_KEY: i32 = 124;
const PREVIEW: i32 = 125;
const SHOW_FILE: i32 = 126;
const SHOW_OCR: i32 = 127;
const SHOW_TRANSCODE: i32 = 128;
const AI: i32 = 130;
const AI_SERVICE: i32 = 131;
const AI_MODEL: i32 = 132;
const AI_ENDPOINT: i32 = 133;
const AI_DEPLOYMENT: i32 = 134;
const AI_VERSION: i32 = 135;
const AI_SYSTEM: i32 = 136;
const AI_MODERATION: i32 = 137;
const AI_PREVIEW: i32 = 138;
const AI_BLUR: i32 = 139;
const SHOW_FIX: i32 = 140;
const SHOW_PROMPT: i32 = 141;
const CUSTOM: i32 = 142;

/// Each combo box's saved field and its `(value, label)` choices.
type Choices = &'static [(&'static str, &'static str)];

fn combos() -> [(i32, &'static str, Choices); 6] {
    [
        (TEXT_ACTION, "text_action", actions::TEXT_CHOICES),
        (IMAGE_ACTION, "image_action", actions::IMAGE_CHOICES),
        (LINK_ACTION, "link_action", actions::LINK_CHOICES),
        (MEDIA_ACTION, "media_action", actions::MEDIA_CHOICES),
        (PICKER_KEY, "picker_key", actions::PICKER_KEYS),
        (AI_SERVICE, "ai_service", ai::SERVICES),
    ]
}

/// Checkboxes saved as `true`/`false`, with their unsaved default.
const CHECKS: [(i32, &str, bool); 13] = [
    (IGNORE, "ignore_node_modules", false),
    (GITIGNORE, "gitignore", false),
    (PREVIEW, "clipboard_preview", true),
    (SHOW_FILE, "show_paste_as_file", true),
    (SHOW_OCR, "show_image_to_text", true),
    (SHOW_TRANSCODE, "show_transcode", true),
    (AI, "ai", false),
    (AI_MODERATION, "ai_moderation", true),
    (AI_PREVIEW, "ai_preview", true),
    (AI_BLUR, "ai_close_on_blur", false),
    (SHOW_FIX, "show_fix_spelling", true),
    (SHOW_PROMPT, "show_ai_prompt", true),
    // Saved separately: the value lives in the Windows Run key.
    (STARTUP, "", false),
];

/// Single-line and multi-line text fields, saved trimmed.
const EDITS: [(i32, &str); 7] = [
    (EXCLUDES, "excludes"),
    (AI_MODEL, "ai_model"),
    (AI_ENDPOINT, "ai_endpoint"),
    (AI_DEPLOYMENT, "ai_deployment"),
    (AI_VERSION, "ai_api_version"),
    (AI_SYSTEM, "ai_system_prompt"),
    (CUSTOM, "custom_actions"),
];

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
    /// The themed tab pane's fill, behind every page control.
    pane: HBRUSH,
    /// Every page control and the tab it belongs to.
    pages: Vec<(HWND, usize)>,
}
impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            for font in &self.fonts {
                let _ = DeleteObject((*font).into());
            }
            let _ = DeleteObject(self.background.into());
            let _ = DeleteObject(self.pane.into());
        }
    }
}

fn read_text(window: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(window) }.clamp(0, 16384) as usize;
    let mut text = vec![0; length + 1];
    let count = unsafe { GetWindowTextW(window, &mut text) }.max(0) as usize;
    String::from_utf16_lossy(&text[..count])
}

fn item(window: HWND, id: i32) -> Result<HWND, String> {
    unsafe { GetDlgItem(Some(window), id) }.map_err(|e| e.to_string())
}

fn show_page(window: HWND, page: usize) {
    unsafe {
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *const State;
        if let Some(state) = state.as_ref() {
            for (control, owner) in &state.pages {
                let _ = ShowWindow(*control, if *owner == page { SW_SHOW } else { SW_HIDE });
            }
        }
    }
}

fn save(window: HWND) -> Result<(), String> {
    unsafe {
        let text = |id| item(window, id).map(read_text);
        let exclusions = super::super::exclusions(&text(EXCLUDES)?)?.join(";");
        let custom = ai::validate_custom_actions(&text(CUSTOM)?)?;
        let checked = |id| IsDlgButtonChecked(window, id) == BST_CHECKED.0;
        let mut values: Vec<(&str, String)> = Vec::new();
        for (id, field, _) in CHECKS {
            if !field.is_empty() {
                values.push((field, checked(id).to_string()));
            }
        }
        for (id, field, choices) in combos() {
            let index = SendMessageW(item(window, id)?, CB_GETCURSEL, None, None).0;
            let value = usize::try_from(index)
                .ok()
                .and_then(|index| choices.get(index))
                .map_or(choices[0].0, |(value, _)| value);
            values.push((field, value.to_owned()));
        }
        for (id, field) in EDITS {
            let value = match id {
                EXCLUDES => exclusions.clone(),
                CUSTOM => custom.clone(),
                _ => text(id)?.trim().to_owned(),
            };
            values.push((field, value));
        }
        if checked(AI) {
            let get = |field: &str| {
                values
                    .iter()
                    .find(|(name, _)| *name == field)
                    .map_or("", |(_, value)| value.as_str())
                    .to_owned()
            };
            ai::Config {
                service: ai::Service::parse(&get("ai_service")).unwrap_or(ai::Service::OpenAi),
                model: get("ai_model"),
                endpoint: get("ai_endpoint"),
                deployment: get("ai_deployment"),
                api_version: get("ai_api_version"),
                system_prompt: get("ai_system_prompt"),
                moderation: checked(AI_MODERATION),
            }
            .validate()?;
        }
        let pairs: Vec<(&str, Option<&str>)> = values
            .iter()
            .map(|(field, value)| (*field, (!value.is_empty()).then_some(value.as_str())))
            .collect();
        crate::preferences::Store::load()
            .map_err(|e| e.to_string())?
            .set_many(super::super::TOOL, &pairs)
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
                let control = HWND(lp.0 as *mut _);
                let paned = (*state).pages.iter().any(|(page, _)| *page == control);
                LRESULT(
                    if paned {
                        (*state).pane
                    } else {
                        (*state).background
                    }
                    .0 as isize,
                )
            }
            WM_NOTIFY => {
                let header = &*(lp.0 as *const NMHDR);
                if header.idFrom == TABS as usize && header.code == TCN_SELCHANGE {
                    let page = SendMessageW(header.hwndFrom, TCM_GETCURSEL, None, None).0;
                    show_page(window, page.max(0) as usize);
                }
                LRESULT(0)
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

/// Every control of the window: class, label, id, area, style, font, page.
/// Page `ALL` is always visible.
const ALL: usize = usize::MAX;
type Layout = (
    PCWSTR,
    &'static str,
    i32,
    [i32; 4],
    WINDOW_STYLE,
    usize,
    usize,
);

fn layout() -> Vec<Layout> {
    let check = WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32);
    let combo = WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32);
    let edit = WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
    let plain = WINDOW_STYLE(0);
    let label = w!("STATIC");
    let mut controls: Vec<Layout> = vec![
        (label, "JustPaste", 11, [96, 22, 480, 36], plain, 1, ALL),
        (
            label,
            "Your clipboard. A file in the right folder.",
            201,
            [97, 60, 475, 24],
            plain,
            2,
            ALL,
        ),
        // Paste
        (
            label,
            "CTRL+V IN EXPLORER",
            202,
            [44, 146, 540, 22],
            plain,
            3,
            0,
        ),
        (label, "Text", 12, [44, 176, 200, 24], plain, 0, 0),
        (label, "Images", 13, [44, 212, 200, 24], plain, 0, 0),
        (label, "Links", 14, [44, 248, 200, 24], plain, 0, 0),
        (
            label,
            "Audio and video files",
            15,
            [44, 284, 200, 24],
            plain,
            0,
            0,
        ),
        (
            label,
            "PASTE AS… PICKER",
            206,
            [44, 330, 540, 22],
            plain,
            3,
            0,
        ),
        (label, "Shortcut", 16, [44, 360, 200, 24], plain, 0, 0),
        (
            w!("BUTTON"),
            "Show what is on the clipboard",
            PREVIEW,
            [44, 396, 552, 26],
            check,
            0,
            0,
        ),
        (
            w!("BUTTON"),
            "Offer paste as file: .txt, .md, .html, .json, PNG, JPEG, WebP",
            SHOW_FILE,
            [44, 424, 552, 26],
            check,
            0,
            0,
        ),
        (
            w!("BUTTON"),
            "Offer text from image (Windows OCR, on this PC)",
            SHOW_OCR,
            [44, 452, 552, 26],
            check,
            0,
            0,
        ),
        (
            w!("BUTTON"),
            "Offer MP3 and MP4 transcoding",
            SHOW_TRANSCODE,
            [44, 480, 552, 26],
            check,
            0,
            0,
        ),
        (
            w!("BUTTON"),
            "Start automatically at sign-in",
            STARTUP,
            [44, 516, 552, 26],
            check,
            0,
            0,
        ),
        (
            label,
            "Only Explorer file panes change. Sources are kept and nothing is overwritten.",
            207,
            [44, 552, 552, 22],
            plain,
            2,
            0,
        ),
        // AI
        (
            w!("BUTTON"),
            "Enable AI paste",
            AI,
            [44, 146, 552, 26],
            check,
            0,
            1,
        ),
        (label, "Service", 17, [44, 182, 200, 24], plain, 0, 1),
        (label, "Model", 18, [44, 216, 200, 24], plain, 0, 1),
        (w!("EDIT"), "", AI_MODEL, [260, 212, 336, 28], edit, 0, 1),
        (
            label,
            "Endpoint URL (optional)",
            19,
            [44, 250, 210, 24],
            plain,
            0,
            1,
        ),
        (w!("EDIT"), "", AI_ENDPOINT, [260, 246, 336, 28], edit, 0, 1),
        (
            label,
            "Azure deployment / version",
            20,
            [44, 284, 210, 24],
            plain,
            0,
            1,
        ),
        (
            w!("EDIT"),
            "",
            AI_DEPLOYMENT,
            [260, 280, 200, 28],
            edit,
            0,
            1,
        ),
        (w!("EDIT"), "", AI_VERSION, [470, 280, 126, 28], edit, 0, 1),
        (label, "System prompt", 21, [44, 318, 210, 24], plain, 0, 1),
        (w!("EDIT"), "", AI_SYSTEM, [260, 314, 336, 28], edit, 0, 1),
        (
            w!("BUTTON"),
            "OpenAI moderation",
            AI_MODERATION,
            [44, 352, 260, 26],
            check,
            0,
            1,
        ),
        (
            w!("BUTTON"),
            "Review answers before saving",
            AI_PREVIEW,
            [320, 352, 276, 26],
            check,
            0,
            1,
        ),
        (
            w!("BUTTON"),
            "Fix spelling and grammar",
            SHOW_FIX,
            [44, 380, 260, 26],
            check,
            0,
            1,
        ),
        (
            w!("BUTTON"),
            "Paste with AI… (any instruction)",
            SHOW_PROMPT,
            [320, 380, 276, 26],
            check,
            0,
            1,
        ),
        (
            w!("BUTTON"),
            "Close the AI window when it loses focus",
            AI_BLUR,
            [44, 408, 552, 26],
            check,
            0,
            1,
        ),
        (
            label,
            "Custom actions, one per line as Name: instruction",
            22,
            [44, 442, 552, 22],
            plain,
            0,
            1,
        ),
        (
            w!("EDIT"),
            "",
            CUSTOM,
            [44, 466, 552, 60],
            WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
            0,
            1,
        ),
        (
            label,
            "Keys come only from OPENAI_API_KEY, AZURE_OPENAI_API_KEY, MISTRAL_API_KEY, GEMINI_API_KEY, ANTHROPIC_API_KEY, or JUSTPASTE_AI_API_KEY and are never saved.",
            208,
            [44, 532, 552, 40],
            plain,
            2,
            1,
        ),
        // Copy filters
        (label, "COPY FILTERS", 209, [44, 146, 540, 22], plain, 3, 2),
        (
            w!("BUTTON"),
            "Ignore node_modules",
            IGNORE,
            [44, 174, 552, 28],
            check,
            0,
            2,
        ),
        (
            label,
            "Skip dependency folders when copying files.",
            203,
            [67, 204, 520, 22],
            plain,
            2,
            2,
        ),
        (
            w!("BUTTON"),
            "Respect .gitignore",
            GITIGNORE,
            [44, 236, 552, 28],
            check,
            0,
            2,
        ),
        (
            label,
            "Use each folder's ignore rules, including nested rules.",
            204,
            [67, 266, 520, 22],
            plain,
            2,
            2,
        ),
        (
            label,
            "Other names or paths to skip",
            23,
            [44, 304, 552, 24],
            plain,
            0,
            2,
        ),
        (w!("EDIT"), "", EXCLUDES, [44, 332, 552, 30], edit, 0, 2),
        (
            label,
            "Separate with semicolons. Example: .cache; build/temp",
            205,
            [44, 370, 552, 22],
            plain,
            2,
            2,
        ),
        (
            label,
            "With no filter, Windows copies files itself. Cut/move always stays with Windows.",
            210,
            [44, 404, 552, 22],
            plain,
            2,
            2,
        ),
        (
            w!("BUTTON"),
            "Cancel",
            2,
            [358, 606, 100, 36],
            WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            0,
            ALL,
        ),
        (
            w!("BUTTON"),
            "Save changes",
            1,
            [470, 606, 146, 36],
            WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            0,
            ALL,
        ),
    ];
    for (id, y, page) in [
        (TEXT_ACTION, 172, 0),
        (IMAGE_ACTION, 208, 0),
        (LINK_ACTION, 244, 0),
        (MEDIA_ACTION, 280, 0),
        (PICKER_KEY, 356, 0),
        (AI_SERVICE, 178, 1),
    ] {
        controls.push((w!("COMBOBOX"), "", id, [260, y, 336, 240], combo, 0, page));
    }
    controls
}

pub(super) fn show() -> Result<(), String> {
    unsafe {
        if let Ok(existing) = FindWindowW(CLASS, None) {
            let _ = ShowWindow(existing, SW_RESTORE);
            let _ = SetForegroundWindow(existing);
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES | ICC_STANDARD_CLASSES,
        });
        let scale = GetDpiForSystem() as f32 / 96.0;
        let px = |value: i32| (value as f32 * scale).round() as i32;
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let mut state = Box::new(State {
            fonts: Vec::new(),
            background: CreateSolidBrush(COLORREF(0x00ffffff)),
            pane: CreateSolidBrush(COLORREF(0x00f9f9f9)),
            pages: Vec::new(),
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
            right: px(640),
            bottom: px(662),
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
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | extra,
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
                [32, 24, 48, 48],
                WINDOW_STYLE(SS_ICON.0),
                0,
            )?;
            SendMessageW(
                icon,
                STM_SETICON,
                Some(WPARAM(app_icon(px(48)).0 as usize)),
                None,
            );
            let mut pages = Vec::new();
            for (class, label, id, area, extra, font, page) in layout() {
                let control = create(class, label, id, area, extra, font)?;
                if page != ALL {
                    pages.push((control, page));
                }
            }
            // The tab strip sits behind the page controls, so it is created last
            // and placed at the bottom of the z-order.
            let tabs = create(
                w!("SysTabControl32"),
                "",
                TABS,
                [24, 102, 592, 486],
                WS_TABSTOP,
                0,
            )?;
            let _ = SetWindowPos(tabs, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            for (index, title) in ["Paste", "AI", "Copy filters"].into_iter().enumerate() {
                let mut title = wide(std::ffi::OsStr::new(title));
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: windows::core::PWSTR(title.as_mut_ptr()),
                    ..Default::default()
                };
                SendMessageW(
                    tabs,
                    TCM_INSERTITEMW,
                    Some(WPARAM(index)),
                    Some(LPARAM(&item as *const _ as isize)),
                );
            }
            state.pages = pages;
            let store = crate::preferences::Store::load().map_err(|e| e.to_string())?;
            let tool = super::super::TOOL;
            let paste = actions::Defaults::load(&store, store.get(tool, "text") == Some("true"));
            let picker = actions::Picker::load(&store);
            for (id, field, choices) in combos() {
                let combo = item(window, id)?;
                for (_, label) in choices {
                    let label = wide(std::ffi::OsStr::new(label));
                    SendMessageW(
                        combo,
                        CB_ADDSTRING,
                        None,
                        Some(LPARAM(label.as_ptr() as isize)),
                    );
                }
                // Ctrl+V choices start from the effective defaults, including
                // the older "save plain text" setting.
                let current = match id {
                    TEXT_ACTION => paste.text.value(),
                    IMAGE_ACTION => paste.image.value(),
                    LINK_ACTION => paste.links.value(),
                    MEDIA_ACTION => paste.media.value(),
                    PICKER_KEY => picker.key,
                    _ => store.get(tool, field).unwrap_or(choices[0].0),
                };
                let index = choices
                    .iter()
                    .position(|(value, _)| *value == current)
                    .unwrap_or(0);
                SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(index)), None);
            }
            for (id, field) in EDITS {
                let control = item(window, id)?;
                SendMessageW(
                    control,
                    EM_SETLIMITTEXT,
                    Some(WPARAM(if id == CUSTOM { 16384 } else { 4096 })),
                    None,
                );
                let value = store.get(tool, field).unwrap_or("");
                let value = if id == CUSTOM {
                    value.replace('\n', "\r\n")
                } else {
                    value.to_owned()
                };
                let value = wide(std::ffi::OsStr::new(&value));
                let _ = SetWindowTextW(control, PCWSTR(value.as_ptr()));
            }
            for (id, field, default) in CHECKS {
                let checked = if field.is_empty() {
                    startup_enabled()
                } else {
                    store
                        .get(tool, field)
                        .map_or(default, |value| value == "true")
                };
                CheckDlgButton(
                    window,
                    id,
                    if checked { BST_CHECKED } else { BST_UNCHECKED },
                )
                .map_err(|e| e.to_string())?;
            }
            show_page(window, 0);
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
            let _ = SetFocus(Some(item(window, TEXT_ACTION)?));
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
