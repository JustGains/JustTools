//! "Paste with AI": an instruction, the answer to review or edit, then Save.
use super::*;
use std::sync::Arc;
use windows::Win32::{
    Graphics::Gdi::*,
    UI::{Controls::*, HiDpi::*},
};

const AI_CLASS: PCWSTR = w!("JustTools.SmartPaste.Ai.v1");
const PROMPT: i32 = 301;
const GENERATE: i32 = 302;
const STATUS: i32 = 303;
const RESULT: i32 = 304;
const ANSWER: u32 = WM_APP + 20;

pub(super) struct Job {
    pub config: super::super::ai::Config,
    pub name: String,
    pub prompt: String,
    pub text: String,
    pub directory: PathBuf,
    pub close_on_blur: bool,
}

struct State {
    job: Job,
    fonts: Vec<HFONT>,
    background: HBRUSH,
    answer: Arc<Mutex<Option<Result<String, String>>>>,
    generation: usize,
    answered: bool,
    saved: Option<String>,
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

fn control(window: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(window), id) }.unwrap_or_default()
}

fn set_text(window: HWND, id: i32, text: &str) {
    let text = wide(std::ffi::OsStr::new(text));
    unsafe {
        let _ = SetWindowTextW(control(window, id), PCWSTR(text.as_ptr()));
    }
}

fn read(window: HWND, id: i32) -> String {
    let handle = control(window, id);
    let length = unsafe { GetWindowTextLengthW(handle) }.max(0) as usize;
    let mut text = vec![0; length + 1];
    let count = unsafe { GetWindowTextW(handle, &mut text) }.max(0) as usize;
    String::from_utf16_lossy(&text[..count])
}

unsafe fn state<'a>(window: HWND) -> Option<&'a mut State> {
    unsafe { (GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State).as_mut() }
}

fn generate(window: HWND, state: &mut State) {
    let prompt = read(window, PROMPT).trim().to_owned();
    if prompt.is_empty() {
        set_text(
            window,
            STATUS,
            "Type an instruction, such as \"Turn this into a bulleted list\".",
        );
        return;
    }
    state.generation += 1;
    let generation = state.generation;
    set_text(
        window,
        STATUS,
        &format!("Asking {}…", state.job.config.describe()),
    );
    unsafe {
        let _ = EnableWindow(control(window, GENERATE), false);
        let _ = EnableWindow(control(window, 1), false);
    }
    let config = state.job.config.clone();
    let text = state.job.text.clone();
    let answer = state.answer.clone();
    let target = window.0 as usize;
    std::thread::spawn(move || {
        let result = super::super::ai::transform(&config, &prompt, &text);
        if let Ok(mut slot) = answer.lock() {
            *slot = Some(result);
        }
        let _ = unsafe {
            PostMessageW(
                Some(HWND(target as *mut _)),
                ANSWER,
                WPARAM(generation),
                LPARAM(0),
            )
        };
    });
}

fn save(window: HWND, state: &mut State) {
    let answer = read(window, RESULT).replace("\r\n", "\n");
    if answer.trim().is_empty() {
        set_text(
            window,
            STATUS,
            "The answer is empty; generate again or type the text to save.",
        );
        return;
    }
    match actions::save_ai(
        &state.job.directory,
        &state.job.name,
        actions::Format::Txt,
        &answer,
    ) {
        Ok(result) => {
            state.saved = Some(result);
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        Err(error) => set_text(window, STATUS, &error),
    }
}

unsafe extern "system" fn procedure(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(window, message, wp, lp);
        }
        let Some(state) = state(window) else {
            return DefWindowProcW(window, message, wp, lp);
        };
        match message {
            WM_CTLCOLORSTATIC => {
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
                LRESULT(state.background.0 as isize)
            }
            WM_COMMAND => {
                match wp.0 as u16 as i32 {
                    // Enter: generate until there is an answer, then save it.
                    1 if !state.answered => generate(window, state),
                    1 => save(window, state),
                    GENERATE => generate(window, state),
                    2 => {
                        let _ = DestroyWindow(window);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            ANSWER if wp.0 == state.generation => {
                let answer = state.answer.lock().ok().and_then(|mut slot| slot.take());
                let _ = EnableWindow(control(window, GENERATE), true);
                match answer {
                    Some(Ok(answer)) => {
                        state.answered = true;
                        set_text(
                            window,
                            RESULT,
                            &answer.replace("\r\n", "\n").replace('\n', "\r\n"),
                        );
                        set_text(window, STATUS, "Review or edit the answer, then Save.");
                        let _ = EnableWindow(control(window, 1), true);
                        let _ = SetFocus(Some(control(window, RESULT)));
                    }
                    Some(Err(error)) => {
                        set_text(window, STATUS, &error);
                        let _ = EnableWindow(control(window, 1), state.answered);
                    }
                    None => {}
                }
                LRESULT(0)
            }
            WM_ACTIVATE if state.job.close_on_blur => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                    let _ = DestroyWindow(window);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(window);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}

/// Show the AI window and block until it closes. Returns the saved result line.
pub(super) fn run(job: Job) -> Result<Option<String>, String> {
    job.config.validate()?;
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let scale = GetDpiForSystem() as f32 / 96.0;
        let px = |value: i32| (value as f32 * scale).round() as i32;
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let mut state = Box::new(State {
            job,
            fonts: Vec::new(),
            background: CreateSolidBrush(COLORREF(0x00ffffff)),
            answer: Arc::new(Mutex::new(None)),
            generation: 0,
            answered: false,
            saved: None,
        });
        for (height, weight) in [(15, 400), (22, 600), (13, 400)] {
            let mut info = LOGFONTW {
                lfHeight: -px(height),
                lfWeight: weight,
                ..Default::default()
            };
            fill(&mut info.lfFaceName, "Segoe UI");
            state.fonts.push(CreateFontIndirectW(&info));
        }
        static REGISTERED: OnceLock<bool> = OnceLock::new();
        let registered = *REGISTERED.get_or_init(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: module.into(),
                lpszClassName: AI_CLASS,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
                hIcon: app_icon(32),
                ..Default::default()
            };
            RegisterClassW(&class) != 0
        });
        if !registered {
            return Err("cannot register the AI paste window".into());
        }
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let mut bounds = RECT {
            right: px(640),
            bottom: px(520),
            ..Default::default()
        };
        AdjustWindowRectEx(&mut bounds, style, false, WS_EX_CONTROLPARENT)
            .map_err(|e| e.to_string())?;
        let title = wide(std::ffi::OsStr::new(&format!(
            "JustPaste · {}",
            state.job.name
        )));
        let window = CreateWindowExW(
            WS_EX_CONTROLPARENT | WS_EX_TOPMOST,
            AI_CLASS,
            PCWSTR(title.as_ptr()),
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
                      font: usize| {
            let text = wide(std::ffi::OsStr::new(label));
            let control = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR(text.as_ptr()),
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
            Ok::<HWND, String>(control)
        };
        let folder = state.job.directory.file_name().map_or_else(
            || state.job.directory.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let result = (|| -> Result<(), String> {
            create(
                w!("STATIC"),
                &state.job.name,
                10,
                [24, 16, 592, 30],
                WINDOW_STYLE(0),
                1,
            )?;
            create(
                w!("STATIC"),
                &format!(
                    "{} · the answer is saved as a new .txt in {folder}. Only the copied text and instruction are sent.",
                    state.job.config.describe()
                ),
                201,
                [24, 48, 592, 22],
                WINDOW_STYLE(0),
                2,
            )?;
            create(
                w!("STATIC"),
                "Instruction",
                11,
                [24, 82, 470, 22],
                WINDOW_STYLE(0),
                0,
            )?;
            let prompt = create(
                w!("EDIT"),
                &state.job.prompt,
                PROMPT,
                [24, 106, 470, 30],
                WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
            )?;
            SendMessageW(prompt, EM_SETLIMITTEXT, Some(WPARAM(4000)), None);
            create(
                w!("BUTTON"),
                "Generate",
                GENERATE,
                [506, 106, 110, 30],
                WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                0,
            )?;
            create(
                w!("STATIC"),
                "",
                STATUS,
                [24, 146, 592, 22],
                WINDOW_STYLE(0),
                2,
            )?;
            create(
                w!("EDIT"),
                "",
                RESULT,
                [24, 172, 592, 280],
                WS_TABSTOP
                    | WS_BORDER
                    | WS_VSCROLL
                    | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
                0,
            )?;
            create(
                w!("BUTTON"),
                "Cancel",
                2,
                [396, 468, 100, 34],
                WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                0,
            )?;
            let save = create(
                w!("BUTTON"),
                "Save",
                1,
                [508, 468, 108, 34],
                WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                0,
            )?;
            let _ = EnableWindow(save, false);
            let _ = ShowWindow(window, SW_SHOW);
            super::picker::take_foreground(window);
            if state.job.prompt.is_empty() {
                let _ = SetFocus(Some(prompt));
            } else {
                let _ = PostMessageW(
                    Some(window),
                    WM_COMMAND,
                    WPARAM(GENERATE as usize),
                    LPARAM(0),
                );
            }
            let mut message = MSG::default();
            while IsWindow(Some(window)).as_bool() && GetMessageW(&mut message, None, 0, 0).0 > 0 {
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
        result?;
        Ok(state.saved.take())
    }
}
