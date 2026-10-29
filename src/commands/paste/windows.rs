//! Per-user Explorer integration, running in the existing executable, never in Explorer.
use super::{
    Settings,
    actions::{self, Action, OnPaste},
    clipboard::Snapshot,
};
use std::{
    ffi::OsString,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
};
use windows::{
    Win32::{
        Foundation::*,
        System::{
            Com::*, DataExchange::GetClipboardSequenceNumber, LibraryLoader::GetModuleHandleW,
            Threading::*,
        },
        UI::{Accessibility::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{Interface, PCWSTR, w},
};

mod ai_window;
mod picker;
mod progress_window;
mod settings;

fn app_icon(size: i32) -> HICON {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return HICON::default();
        };
        LoadImageW(
            Some(module.into()),
            PCWSTR(116_usize as *const u16),
            IMAGE_ICON,
            size,
            size,
            LR_SHARED,
        )
        .map(|handle| HICON(handle.0))
        .unwrap_or_default()
    }
}

fn open_settings() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut child = executable_command()?
        .arg("--settings")
        // Settings may share the resident's job. Requesting breakaway here
        // fails with access denied when that job forbids child detachment.
        .creation_flags(CREATE_NO_WINDOW.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot open settings: {e}"))?;
    for _ in 0..50 {
        if settings::ready_window().is_some() {
            return Ok(());
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Settings closed before its window was ready. Run just paste --settings for details.".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err("Settings did not open. Run just paste --settings for details.".into())
}

const CLASS: PCWSTR = w!("JustTools.SmartPaste.v1");
const TRAY: u32 = WM_APP + 1;
const NOTICE: u32 = WM_APP + 2;
const NOTICE_BUSY: u32 = WM_APP + 3;
const READ_STATE: u32 = WM_APP + 4;
const INSPECT_SHELL: u32 = WM_APP + 5;
const INJECTED: usize = 0x4a505354;
static PAUSED: AtomicBool = AtomicBool::new(false);
static BUSY: AtomicBool = AtomicBool::new(false);
/// Whether copied files may need JustPaste: copy filters or a media default.
static FILTERS: AtomicBool = AtomicBool::new(false);
/// The picker chord: 0 off, 1 Ctrl+Shift+V, 2 Win+Shift+V, 3 Ctrl+Alt+V.
static PICKER_KEY: AtomicU32 = AtomicU32::new(1);
static SWALLOWED_IN: AtomicUsize = AtomicUsize::new(0);
static EXITING: AtomicBool = AtomicBool::new(false);
static MAINTAINING: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static LAST_STATE: AtomicU32 = AtomicU32::new(0);
static REQUESTS: OnceLock<SyncSender<Request>> = OnceLock::new();
static NOTICES: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
static LAST_FOLDER: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct Request {
    window: usize,
    focus: usize,
    sequence: u32,
    /// Pressed with the picker shortcut rather than Ctrl+V.
    picker: bool,
}

fn picker_code(key: &str) -> u32 {
    match key {
        "ctrl+shift+v" => 1,
        "win+shift+v" => 2,
        "ctrl+alt+v" => 3,
        _ => 0,
    }
}

pub(super) fn picker_label() -> &'static str {
    match PICKER_KEY.load(Ordering::SeqCst) {
        1 => "Ctrl+Shift+V",
        2 => "Win+Shift+V",
        3 => "Ctrl+Alt+V",
        _ => "off",
    }
}

/// Refresh what the keyboard hook reads from the saved settings.
fn publish(settings: &Settings) {
    FILTERS.store(
        settings.filters() || settings.paste.media != OnPaste::Act(Action::Native),
        Ordering::SeqCst,
    );
    PICKER_KEY.store(picker_code(settings.picker.key), Ordering::SeqCst);
}

/// Which handled chord, if any, the modifiers make with V: 1 is Ctrl+V,
/// 2 the configured picker shortcut.
fn chord(ctrl: bool, shift: bool, alt: bool, win: bool, picker: u32) -> u32 {
    match (ctrl, shift, alt, win) {
        (true, false, false, false) => 1,
        (true, true, false, false) if picker == 1 => 2,
        (false, true, false, true) if picker == 2 => 2,
        (true, false, true, false) if picker == 3 => 2,
        _ => 0,
    }
}

/// Key repeats belong only to the exact Explorer file control that took the
/// original press. Alt/system keys and a new focus always pass through.
fn suppress_followup(
    owner: usize,
    scope: usize,
    message: u32,
    plain_ctrl_v: bool,
) -> (usize, bool) {
    if owner == 0 || owner != scope {
        return (0, false);
    }
    match message {
        WM_KEYUP => (0, true),
        WM_KEYDOWN if plain_ctrl_v => (owner, true),
        _ => (0, false),
    }
}

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIBV5: u32 = 17;

/// `PNG`, `image/png`, `FileGroupDescriptorW`, `FileGroupDescriptor`, registered
/// before the hook starts so the hook itself only reads them.
static FORMATS: OnceLock<[u32; 4]> = OnceLock::new();
fn registered_formats() -> [u32; 4] {
    *FORMATS.get_or_init(|| {
        [
            "PNG",
            "image/png",
            "FileGroupDescriptorW",
            "FileGroupDescriptor",
        ]
        .map(|name| unsafe {
            windows::Win32::System::DataExchange::RegisterClipboardFormatW(
                &windows::core::HSTRING::from(name),
            )
        })
    })
}

/// Whether the clipboard holds something Smart Paste can act on, checked by
/// format availability alone (no clipboard open) so it is cheap enough for the
/// keyboard hook: images and text (links), and copied files only with copy
/// filters on. Virtual files and everything else keep Explorer's own paste.
fn wants_clipboard(
    filters: bool,
    [png, image_png, descriptor_w, descriptor]: [u32; 4],
    available: impl Fn(u32) -> bool,
) -> bool {
    if available(CF_HDROP) {
        return filters;
    }
    if [descriptor_w, descriptor]
        .into_iter()
        .any(|format| format != 0 && available(format))
    {
        return false;
    }
    [png, image_png, CF_DIBV5, CF_DIB, CF_UNICODETEXT]
        .into_iter()
        .any(|format| format != 0 && available(format))
}

/// The hook must keep pumping even while a tray menu is open or settings I/O
/// stalls. Windows silently removes low-level hooks that take too long.
struct KeyboardListener(u32);
impl KeyboardListener {
    fn new() -> Result<Self, String> {
        let (ready, initialized) = sync_channel(1);
        registered_formats();
        std::thread::spawn(move || unsafe {
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            let hook = GetModuleHandleW(None).and_then(|module| {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(module.into()), 0)
            });
            let hook = match hook {
                Ok(hook) => hook,
                Err(error) => {
                    let _ = ready.send(Err(error.to_string()));
                    return;
                }
            };
            if ready.send(Ok(GetCurrentThreadId())).is_ok() {
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            let _ = UnhookWindowsHookEx(hook);
        });
        initialized
            .recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())?
            .map(Self)
    }
}
impl Drop for KeyboardListener {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.0, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

fn state_label(state: u32) -> &'static str {
    match state {
        1 => "Ctrl+V detected in Explorer",
        2 => "native paste: text field, menu, or other Explorer control",
        3 => "native file paste: copy filters are off",
        4 => "checking the active Explorer folder",
        5 => "native paste: no matching filesystem folder",
        6 => "pasting",
        7 => "paste completed",
        8 => "paste failed (see the JustPaste notification)",
        9 => "native paste: clipboard content is not enabled for Smart Paste",
        10 => "busy with the previous paste",
        11 => "paste skipped: release the shortcut keys and try again",
        12 => "picker closed without a choice",
        _ => "waiting for Ctrl+V in Explorer",
    }
}

#[derive(Debug)]
struct ShellView {
    owner: usize,
    window: usize,
    directory: Option<PathBuf>,
}

type ShellReply = SyncSender<Result<Vec<ShellView>, String>>;

/// Explorer automation requires an STA with a message pump. UI Automation stays
/// on the existing MTA worker; only plain handles and paths cross this boundary.
struct ShellResolver {
    thread: u32,
    requests: SyncSender<ShellReply>,
}

struct Apartment;
impl Apartment {
    fn sta() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .ok()
            .map_err(|e| e.to_string())?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

impl ShellResolver {
    fn new() -> Result<Self, String> {
        let (requests, receiver) = sync_channel::<ShellReply>(1);
        let (ready, initialized) = sync_channel(1);
        std::thread::spawn(move || {
            let apartment = Apartment::sta();
            let _apartment = match apartment {
                Ok(apartment) => apartment,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            unsafe {
                let mut message = MSG::default();
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                if ready.send(Ok(GetCurrentThreadId())).is_err() {
                    return;
                }
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    if message.message == INSPECT_SHELL {
                        if let Ok(reply) = receiver.try_recv() {
                            let _ = reply.send(shell_views());
                        }
                    } else {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            }
        });
        let thread = initialized
            .recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())??;
        Ok(Self { thread, requests })
    }

    fn inspect(&self) -> Result<Vec<ShellView>, String> {
        let (reply, response) = sync_channel(1);
        self.requests
            .try_send(reply)
            .map_err(|e| format!("Explorer inspection is busy: {e}"))?;
        unsafe { PostThreadMessageW(self.thread, INSPECT_SHELL, WPARAM(0), LPARAM(0)) }
            .map_err(|e| e.to_string())?;
        response
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| format!("Explorer inspection timed out: {e}"))?
    }
}
impl Drop for ShellResolver {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

fn shell_views() -> Result<Vec<ShellView>, String> {
    unsafe {
        let shell: IShellWindows =
            CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let count = shell.Count().map_err(|e| e.to_string())?;
        let mut views = Vec::new();
        let mut last_error = None;
        for index in 0..count {
            let entry = (|| -> windows::core::Result<ShellView> {
                let dispatch = shell.Item(&index.into())?;
                let browser = dispatch.cast::<IWebBrowserApp>()?;
                let owner = HWND(browser.HWND()?.0 as *mut _);
                let provider = dispatch.cast::<IServiceProvider>()?;
                let browser = provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser)?;
                let view = browser.QueryActiveShellView()?;
                let window = view.GetWindow()?;
                let folder = view.cast::<IFolderView>()?;
                Ok(ShellView {
                    owner: GetAncestor(owner, GA_ROOT).0 as usize,
                    window: window.0 as usize,
                    directory: folder_path(&folder).ok(),
                })
            })();
            match entry {
                Ok(view) => views.push(view),
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if views.is_empty()
            && let Some(error) = last_error
        {
            return Err(format!("cannot query Explorer: {error}"));
        }
        Ok(views)
    }
}

fn executable_command() -> Result<Command, String> {
    let path = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(&path);
    if path
        .file_stem()
        .is_none_or(|s| !s.eq_ignore_ascii_case("justpaste"))
    {
        command.arg("paste");
    }
    Ok(command)
}

fn ready_window() -> Option<HWND> {
    let window = unsafe { FindWindowW(CLASS, None) }.ok()?;
    (!unsafe { GetPropW(window, w!("JustPaste.Ready")) }.is_invalid()).then_some(window)
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}

fn class_name(window: HWND) -> String {
    let mut buffer = [0; 256];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

fn focus(window: HWND) -> Option<HWND> {
    let thread = unsafe { GetWindowThreadProcessId(window, None) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(thread, &mut info) }.ok()?;
    if gui_is_busy(info.flags.0) {
        return None;
    } // menus, move/size, and popup modes pass through
    Some(info.hwndFocus)
}

fn gui_is_busy(flags: u32) -> bool {
    flags & (GUI_INMENUMODE.0 | GUI_INMOVESIZE.0 | GUI_POPUPMENUMODE.0 | GUI_SYSTEMMENUMODE.0) != 0
}

fn same_target(request: &Request) -> bool {
    let window = HWND(request.window as *mut _);
    (unsafe { GetForegroundWindow() == window })
        && focus(window).is_some_and(|handle| handle.0 as usize == request.focus)
}

fn notify(message: impl Into<String>, error: bool) {
    if let Ok(mut notices) = NOTICES.lock() {
        notices.push((message.into(), error));
    }
    if let Ok(window) = unsafe { FindWindowW(CLASS, None) } {
        let _ = unsafe { PostMessageW(Some(window), NOTICE, WPARAM(0), LPARAM(0)) };
    }
}

unsafe extern "system" fn keyboard(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code < 0 || data.0 == 0 {
        return unsafe { CallNextHookEx(None, code, message, data) };
    }
    let event = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
    if event.dwExtraInfo == INJECTED || event.vkCode != u32::from(VK_V.0) {
        return unsafe { CallNextHookEx(None, code, message, data) };
    }
    let pressed = |key: VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 };
    let win = pressed(VK_LWIN) || pressed(VK_RWIN);
    let alt = pressed(VK_MENU);
    let chord = chord(
        pressed(VK_CONTROL),
        pressed(VK_SHIFT),
        alt,
        win,
        PICKER_KEY.load(Ordering::SeqCst),
    );
    let window = unsafe { GetForegroundWindow() };
    let explorer = class_name(window) == "CabinetWClass";
    let focused = explorer.then(|| focus(window)).flatten().filter(|handle| {
        matches!(
            class_name(*handle).as_str(),
            "DirectUIHWND" | "SysListView32"
        )
    });
    let scope = focused
        .filter(|_| chord != 0 && !PAUSED.load(Ordering::SeqCst))
        .map_or(0, |handle| handle.0 as usize);
    let (owner, swallow) = suppress_followup(
        SWALLOWED_IN.load(Ordering::SeqCst),
        scope,
        message.0 as u32,
        chord != 0,
    );
    SWALLOWED_IN.store(owner, Ordering::SeqCst);
    if swallow {
        return LRESULT(1);
    }
    if message.0 as u32 != WM_KEYDOWN || chord == 0 || PAUSED.load(Ordering::SeqCst) || !explorer {
        return unsafe { CallNextHookEx(None, code, message, data) };
    }
    LAST_STATE.store(1, Ordering::SeqCst);
    let Some(focused) = focused else {
        LAST_STATE.store(2, Ordering::SeqCst);
        return unsafe { CallNextHookEx(None, code, message, data) };
    };
    // Anything Smart Paste would hand back stays Explorer's paste, untouched and without delay.
    let available = |format| unsafe {
        windows::Win32::System::DataExchange::IsClipboardFormatAvailable(format).is_ok()
    };
    let picker = chord == 2;
    if !picker
        && !wants_clipboard(
            FILTERS.load(Ordering::SeqCst),
            registered_formats(),
            available,
        )
    {
        LAST_STATE.store(if available(CF_HDROP) { 3 } else { 9 }, Ordering::SeqCst);
        return unsafe { CallNextHookEx(None, code, message, data) };
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        LAST_STATE.store(10, Ordering::SeqCst);
        // Never take a mutex, access disk, or call COM from the keyboard hook.
        if let Ok(tray) = unsafe { FindWindowW(CLASS, None) } {
            let _ = unsafe { PostMessageW(Some(tray), NOTICE_BUSY, WPARAM(0), LPARAM(0)) };
        }
        SWALLOWED_IN.store(scope, Ordering::SeqCst);
        return LRESULT(1);
    }
    let request = Request {
        window: window.0 as usize,
        focus: focused.0 as usize,
        sequence: unsafe { GetClipboardSequenceNumber() },
        picker,
    };
    crate::common::set_interrupted(false);
    if REQUESTS
        .get()
        .is_some_and(|sender| sender.try_send(request).is_ok())
    {
        SWALLOWED_IN.store(scope, Ordering::SeqCst);
        if win || alt {
            // A Windows or Alt key released with nothing between would open
            // Start or Explorer's menu bar; an unassigned key keeps them closed.
            mask_modifier_release();
        }
        LRESULT(1)
    } else {
        BUSY.store(false, Ordering::SeqCst);
        unsafe { CallNextHookEx(None, code, message, data) }
    }
}

fn mask_modifier_release() {
    let key = |up| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0xE8),
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                dwExtraInfo: INJECTED,
                ..Default::default()
            },
        },
    };
    unsafe {
        SendInput(
            &[key(false), key(true)],
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

// UI Automation is queried on the MTA worker; shell COM is owned by its STA.
fn destination(request: &Request, resolver: &ShellResolver) -> Result<Option<PathBuf>, String> {
    if !same_target(request) {
        return Ok(None);
    }
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("cannot inspect Explorer focus: {e}"))?;
        let element = automation.GetFocusedElement().map_err(|e| e.to_string())?;
        let kind = element.CurrentControlType().map_err(|e| e.to_string())?;
        if !file_control(kind) {
            LAST_STATE.store(2, Ordering::SeqCst);
            return Ok(None);
        }
        for view in resolver.inspect()? {
            let window = HWND(view.window as *mut _);
            let focused = HWND(request.focus as *mut _);
            if view.owner != request.window
                || !IsWindowVisible(window).as_bool()
                || !(window == focused || IsChild(window, focused).as_bool())
            {
                continue;
            }
            if let Some(path) = view.directory
                && same_target(request)
                && path.is_dir()
            {
                return Ok(Some(path));
            }
        }
    }
    LAST_STATE.store(5, Ordering::SeqCst);
    Ok(None)
}

fn file_control(kind: UIA_CONTROLTYPE_ID) -> bool {
    [
        UIA_ListControlTypeId,
        UIA_ListItemControlTypeId,
        UIA_DataItemControlTypeId,
        UIA_PaneControlTypeId,
    ]
    .contains(&kind)
}

fn native_paste(request: &Request) {
    // Avoid replaying V while the original physical press is still held, or
    // accidentally turning it into an Alt/Shift/Windows shortcut.
    for _ in 0..50 {
        if unsafe { GetAsyncKeyState(i32::from(VK_V.0)) } >= 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if [VK_V, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 })
    {
        LAST_STATE.store(11, Ordering::SeqCst);
        notify(
            "Paste skipped because the shortcut keys changed. Release them and try again.",
            false,
        );
        return;
    }
    if !same_target(request) || unsafe { GetClipboardSequenceNumber() } != request.sequence {
        return;
    }
    let control_down = unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) < 0 };
    let key = |vk, up| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                dwExtraInfo: INJECTED,
                ..Default::default()
            },
        },
    };
    let mut keys = Vec::new();
    if !control_down {
        keys.push(key(VK_CONTROL, false));
    }
    keys.push(key(VK_V, false));
    keys.push(key(VK_V, true));
    if !control_down {
        keys.push(key(VK_CONTROL, true));
    }
    unsafe {
        SendInput(&keys, std::mem::size_of::<INPUT>() as i32);
    }
}

fn folder_path(view: &IFolderView) -> windows::core::Result<PathBuf> {
    unsafe {
        let folder: IPersistFolder2 = view.GetFolder()?;
        let pidl = folder.GetCurFolder()?;
        let item = SHCreateItemFromIDList::<IShellItem>(pidl);
        CoTaskMemFree(Some(pidl.cast()));
        let name = item?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0.cast()));
        Ok(PathBuf::from(path?))
    }
}

fn diagnose() -> Result<(), String> {
    // No clipboard payloads are read; URLs, filenames, and paths are not printed.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let window = GetForegroundWindow();
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            GetGUIThreadInfo(GetWindowThreadProcessId(window, None), &mut info)
                .map_err(|e| e.to_string())?;
            println!(
                "Foreground class: {}; focus class: {}; GUI flags: {:#x}",
                class_name(window),
                class_name(info.hwndFocus),
                info.flags.0
            );
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| e.to_string())?;
            match automation
                .GetFocusedElement()
                .and_then(|e| e.CurrentControlType())
            {
                Ok(kind) => println!(
                    "Focused control type: {}; file control: {}",
                    kind.0,
                    file_control(kind)
                ),
                Err(error) => println!("Focus inspection: {error}"),
            }
            let resolver = ShellResolver::new()?;
            let views = resolver.inspect()?;
            println!("Explorer shell views (STA): {}", views.len());
            for (index, view) in views.iter().enumerate() {
                let owner = HWND(view.owner as *mut _);
                let view_window = HWND(view.window as *mut _);
                println!(
                    "  view {index}: owner_class={} visible={} foreground={} contains_focus={} filesystem_folder={}",
                    class_name(owner),
                    IsWindowVisible(view_window).as_bool(),
                    owner == window,
                    view_window == info.hwndFocus || IsChild(view_window, info.hwndFocus).as_bool(),
                    view.directory.as_ref().is_some_and(|p| p.is_dir())
                );
            }
            Ok(())
        })();
        CoUninitialize();
        result
    }
}

fn handle(request: &Request, resolver: &ShellResolver) -> Result<(), String> {
    LAST_STATE.store(4, Ordering::SeqCst);
    let Some(directory) = destination(request, resolver)? else {
        native_paste(request);
        return Ok(());
    };
    let settings = Settings::saved()?;
    if unsafe { GetClipboardSequenceNumber() } != request.sequence {
        return Err("Clipboard changed before paste; try again.".into());
    }
    let snapshot = super::clipboard::read()?;
    if unsafe { GetClipboardSequenceNumber() } != request.sequence || !same_target(request) {
        return Err("Clipboard or Explorer focus changed before paste; try again.".into());
    }
    let choice = if request.picker {
        OnPaste::Ask
    } else {
        settings.paste.choose(&snapshot, settings.filters())
    };
    let action = match choice {
        OnPaste::Act(action) => action,
        OnPaste::Ask => {
            let default = settings.paste.choose(&snapshot, settings.filters());
            match picker::choose(request, &snapshot, &settings, &default)? {
                Some(action) => action,
                None => {
                    LAST_STATE.store(12, Ordering::SeqCst);
                    return Ok(());
                }
            }
        }
    };
    let action = match action {
        Action::Smart if !smart_handles(&snapshot, &settings) => Action::Native,
        action => action,
    };
    if action == Action::Native {
        LAST_STATE.store(9, Ordering::SeqCst);
        native_paste(request);
        return Ok(());
    }
    if let Action::Ai { name, prompt } = &action
        && (settings.picker.ai_preview || prompt.is_empty())
    {
        let config = settings
            .ai
            .clone()
            .ok_or("AI paste is turned off in settings")?;
        if let Ok(mut last) = LAST_FOLDER.lock() {
            *last = Some(directory.clone());
        }
        LAST_STATE.store(6, Ordering::SeqCst);
        if let Some(result) = ai_window::run(ai_window::Job {
            config,
            name: name.clone(),
            prompt: prompt.clone(),
            text: snapshot.text().unwrap_or_default().to_owned(),
            directory: directory.clone(),
            close_on_blur: settings.picker.close_on_blur,
        })? {
            LAST_STATE.store(7, Ordering::SeqCst);
            notify(result, false);
        }
        return Ok(());
    }
    if let Ok(mut last) = LAST_FOLDER.lock() {
        *last = Some(directory.clone());
    }
    if crate::common::interrupted() {
        return Err("cancelled".into());
    }
    let progress = crate::progress::Operation::begin();
    progress_window::start(progress.reporter.clone(), directory.clone());
    LAST_STATE.store(6, Ordering::SeqCst);
    let result = match action {
        Action::As(format) => actions::paste_as(format, snapshot, &directory, false),
        Action::PageMarkdown => {
            super::smart_paste(snapshot, &directory, &settings, super::LinkMode::Markdown)
        }
        Action::Ai { name, prompt } => match &settings.ai {
            Some(config) => actions::paste_ai(
                config,
                &name,
                &prompt,
                &snapshot,
                &directory,
                actions::Format::Txt,
                false,
            ),
            None => Err("AI paste is turned off in settings".into()),
        },
        _ => super::smart_paste(snapshot, &directory, &settings, super::LinkMode::Auto),
    };
    progress.finish(&result);
    let result = result?;
    LAST_STATE.store(7, Ordering::SeqCst);
    notify(result, false);
    Ok(())
}

/// Whether JustPaste's own handling covers `snapshot`; otherwise Explorer pastes.
fn smart_handles(snapshot: &Snapshot, settings: &Settings) -> bool {
    match snapshot {
        Snapshot::Image(..) => true,
        Snapshot::Text(_) | Snapshot::Html { .. } => {
            settings.text || snapshot.text().is_some_and(|t| !super::links(t).is_empty())
        }
        Snapshot::Files { cut, .. } => !cut && settings.filters(),
        Snapshot::Unsupported => false,
    }
}

fn fill(buffer: &mut [u16], value: &str) {
    for (target, character) in buffer
        .iter_mut()
        .take(value.encode_utf16().count())
        .zip(value.encode_utf16())
    {
        *target = character;
    }
    if let Some(last) = buffer.last_mut() {
        *last = 0;
    }
}

fn icon(window: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: TRAY,
        ..Default::default()
    };
    data.hIcon = app_icon(unsafe { GetSystemMetrics(SM_CXSMICON) });
    fill(
        &mut data.szTip,
        if PAUSED.load(Ordering::SeqCst) {
            "JustPaste · paused"
        } else {
            "JustPaste · Ctrl+V in Explorer"
        },
    );
    data
}

fn setting(field: &str) -> bool {
    crate::preferences::Store::load()
        .ok()
        .is_some_and(|store| store.get(super::TOOL, field) == Some("true"))
}

fn toggle(field: &str) -> Result<(), String> {
    let enabled = !setting(field);
    crate::preferences::Store::load()
        .map_err(|e| e.to_string())?
        .set(
            super::TOOL,
            field,
            Some(if enabled { "true" } else { "false" }),
        )
        .map_err(|e| e.to_string())?;
    publish(&Settings::saved()?);
    Ok(())
}

fn startup_enabled() -> bool {
    use winreg::{RegKey, enums::*};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .ok()
        .and_then(|key| key.get_value::<String, _>("JustTools Smart Paste").ok())
        .is_some()
}

fn startup(enabled: bool) -> Result<(), String> {
    use winreg::{RegKey, enums::*};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| e.to_string())?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dispatch = if exe
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("justpaste"))
        {
            ""
        } else {
            " paste"
        };
        key.set_value(
            "JustTools Smart Paste",
            &format!("\"{}\"{dispatch} --enable", exe.display()),
        )
        .map_err(|e| e.to_string())?;
    } else {
        match key.delete_value("JustTools Smart Paste") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn menu(window: HWND) -> Result<(), String> {
    unsafe {
        let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
        let picker_line = format!("Paste as…  ·  {}", picker_label());
        let entries = [
            (0, "JustPaste", false, false),
            (0, "Images, text, links && files  ·  Ctrl+V", false, false),
            (0, &picker_line, false, false),
            (6, "Open settings…", false, true),
            (
                1,
                if PAUSED.load(Ordering::SeqCst) {
                    "Resume Smart Paste"
                } else {
                    "Pause Smart Paste"
                },
                false,
                true,
            ),
            (0, "", false, false),
            (
                2,
                "Ignore node_modules",
                setting("ignore_node_modules"),
                true,
            ),
            (3, "Respect .gitignore", setting("gitignore"), true),
            (0, "", false, false),
            (5, "Start at sign-in", startup_enabled(), true),
            (
                7,
                "Open last folder",
                false,
                LAST_FOLDER.lock().is_ok_and(|p| p.is_some()),
            ),
            (
                8,
                "Cancel current paste",
                false,
                BUSY.load(Ordering::SeqCst),
            ),
            (0, "", false, false),
            (9, "Quit JustPaste", false, true),
        ];
        for (id, label, checked, enabled) in entries {
            let label: &str = label;
            let flags = if label.is_empty() {
                MF_SEPARATOR
            } else {
                MF_STRING
            } | if checked { MF_CHECKED } else { MF_UNCHECKED }
                | if enabled { MF_ENABLED } else { MF_GRAYED };
            let label = wide(std::ffi::OsStr::new(label));
            let _ = AppendMenuW(menu, flags, id, PCWSTR(label.as_ptr()));
        }
        let _ = SetMenuDefaultItem(menu, 6, 0);
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(window);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            Some(0),
            window,
            None,
        )
        .0;
        let _ = DestroyMenu(menu);
        let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
        match chosen {
            1 => {
                PAUSED.fetch_xor(true, Ordering::SeqCst);
                let _ = Shell_NotifyIconW(NIM_MODIFY, &icon(window));
            }
            2 => toggle("ignore_node_modules")?,
            3 => toggle("gitignore")?,
            5 => startup(!startup_enabled())?,
            6 => open_settings()?,
            7 => {
                if let Ok(last) = LAST_FOLDER.lock()
                    && let Some(folder) = &*last
                {
                    Command::new("explorer.exe")
                        .arg(folder)
                        .spawn()
                        .map_err(|e| e.to_string())?;
                }
            }
            8 => {
                crate::common::set_interrupted(true);
                notify("Cancellation requested; completed files are kept.", false);
            }
            9 => {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            _ => {}
        }
    }
    Ok(())
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message != 0 && message == TASKBAR_CREATED.load(Ordering::SeqCst) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &icon(window));
        }
        return LRESULT(0);
    }
    match message {
        READ_STATE => LRESULT(
            (LAST_STATE.load(Ordering::SeqCst)
                | (u32::from(PAUSED.load(Ordering::SeqCst)) << 16)
                | (u32::from(BUSY.load(Ordering::SeqCst)) << 17)) as isize,
        ),
        NOTICE_BUSY => {
            notify(
                "A paste is already running. Use the tray menu to cancel it.",
                false,
            );
            LRESULT(0)
        }
        TRAY if lparam.0 as u32 == WM_LBUTTONUP => {
            if let Err(error) = open_settings() {
                notify(error, true);
            }
            LRESULT(0)
        }
        TRAY if lparam.0 as u32 == WM_RBUTTONUP => {
            if let Err(error) = menu(window) {
                notify(error, true);
            }
            LRESULT(0)
        }
        NOTICE => {
            if let Ok(mut notices) = NOTICES.lock() {
                // Show the most recent state, not an obsolete queued progress message.
                if let Some((message, error)) = notices.pop() {
                    notices.clear();
                    let mut data = icon(window);
                    data.uFlags = NIF_INFO;
                    data.dwInfoFlags = if error { NIIF_ERROR } else { NIIF_INFO };
                    fill(&mut data.szInfoTitle, "JustPaste");
                    fill(&mut data.szInfo, &message);
                    unsafe {
                        let title = wide(std::ffi::OsStr::new(&message));
                        let _ = SetWindowTextW(window, PCWSTR(title.as_ptr()));
                        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
                    }
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if EXITING.load(Ordering::SeqCst)
                && !BUSY.load(Ordering::SeqCst)
                && !MAINTAINING.load(Ordering::SeqCst)
            {
                unsafe {
                    let _ = DestroyWindow(window);
                }
                return LRESULT(0);
            }
            if let Ok(settings) = Settings::saved() {
                publish(&settings);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            EXITING.store(true, Ordering::SeqCst);
            crate::common::stop_background();
            PAUSED.store(true, Ordering::SeqCst);
            crate::common::set_interrupted(true);
            if BUSY.load(Ordering::SeqCst) || MAINTAINING.load(Ordering::SeqCst) {
                notify(
                    "Stopping Smart Paste; waiting for its background work to finish cancelling.",
                    false,
                );
            } else {
                unsafe {
                    let _ = DestroyWindow(window);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon(window));
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn background() -> Result<(), String> {
    crate::common::set_background();
    unsafe {
        // Named mutex is scoped to this login session. Only one hook owns paste.
        let mutex = CreateMutexW(None, false, w!("Local\\JustTools.SmartPaste.v1"))
            .map_err(|e| e.to_string())?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return Ok(());
        }
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::SeqCst,
        );
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err("cannot register Smart Paste window".into());
        }
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("JustPaste"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        publish(&Settings::saved()?);
        let (sender, receiver) = sync_channel(1);
        REQUESTS
            .set(sender)
            .map_err(|_| "Smart Paste already initialized")?;
        let (initialized, initialization) = sync_channel(1);
        std::thread::spawn(move || {
            let result = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
            if let Err(error) = result {
                let _ = initialized.send(Err(error.to_string()));
                return;
            }
            let resolver = match ShellResolver::new() {
                Ok(resolver) => resolver,
                Err(error) => {
                    let _ = initialized.send(Err(error));
                    CoUninitialize();
                    return;
                }
            };
            let _ = initialized.send(Ok(()));
            for request in receiver {
                if let Err(error) = handle(&request, &resolver) {
                    LAST_STATE.store(8, Ordering::SeqCst);
                    notify(error, true);
                }
                BUSY.store(false, Ordering::SeqCst);
            }
            CoUninitialize();
        });
        initialization
            .recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())??;
        let _hook = KeyboardListener::new()?;
        // The update itself enforces the 12-hour success / 15-minute retry gate.
        // This thread never owns the keyboard hook or the tray message pump.
        std::thread::spawn(|| {
            while !EXITING.load(Ordering::SeqCst) {
                MAINTAINING.store(true, Ordering::SeqCst);
                if !EXITING.load(Ordering::SeqCst) {
                    crate::commands::download::refresh(super::TOOL);
                }
                MAINTAINING.store(false, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_secs(15 * 60));
            }
        });
        if !Shell_NotifyIconW(NIM_ADD, &icon(window)).as_bool() {
            return Err("cannot add Smart Paste tray icon".into());
        }
        SetTimer(Some(window), 1, 1000, None);
        SetPropW(window, w!("JustPaste.Ready"), Some(HANDLE(window.0)))
            .map_err(|e| e.to_string())?;
        notify(
            format!(
                "Smart Paste is ready. Ctrl+V saves images and links in Explorer; {} picks a format. Click the tray icon for settings.",
                picker_label()
            ),
            false,
        );
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = CloseHandle(mutex);
    }
    Ok(())
}

pub fn control(args: &[OsString]) -> Result<(), String> {
    let action = args[0].to_string_lossy();
    if (action == "--startup" && args.len() != 2) || (action != "--startup" && args.len() != 1) {
        return Err("Smart Paste controls must be used alone; --startup takes on or off".into());
    }
    let running = ready_window();
    match action.as_ref() {
        "--settings" => settings::show(),
        "--diagnose" => diagnose(),
        "--background" => background(),
        "--status" => {
            println!(
                "JustPaste Smart Paste: {}\nStart at login: {}\nCopy filters: {}",
                if running.is_some() {
                    "running"
                } else {
                    "stopped"
                },
                startup_enabled(),
                if Settings::saved().is_ok_and(|settings| settings.filters()) {
                    "on (JustPaste copies files)"
                } else {
                    "off (Explorer copies files)"
                }
            );
            if let Some(window) = running {
                let mut state = 0_usize;
                let result = unsafe {
                    SendMessageTimeoutW(
                        window,
                        READ_STATE,
                        WPARAM(0),
                        LPARAM(0),
                        SMTO_ABORTIFHUNG,
                        1000,
                        Some(&mut state),
                    )
                };
                if result.0 != 0 {
                    println!(
                        "Paused: {}\nBusy: {}\nLast Explorer paste: {}",
                        state & (1 << 16) != 0,
                        state & (1 << 17) != 0,
                        state_label((state & 0xffff) as u32)
                    );
                } else {
                    println!("Background process is not responding to status requests.");
                }
                let mut text = [0_u16; 1024];
                let length = unsafe { GetWindowTextW(window, &mut text) };
                if length > 0 {
                    println!(
                        "Last notification: {}",
                        String::from_utf16_lossy(&text[..length as usize])
                    );
                }
            }
            Ok(())
        }
        "--enable" => {
            if running.is_none() {
                use std::os::windows::process::CommandExt;
                executable_command()?
                    .arg("--background")
                    // A tray resident must outlive terminal/agent jobs. Hiding
                    // the console alone still leaves it in a kill-on-close job.
                    // Do not silently retry without breakaway when a host denies
                    // it: that would report success for a short-lived listener.
                    .creation_flags(CREATE_NO_WINDOW.0 | CREATE_BREAKAWAY_FROM_JOB.0)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| {
                        format!("Cannot start independent Smart Paste process: {e}. Start it from the Explorer launcher if this host forbids detached processes.")
                    })?;
                for _ in 0..50 {
                    if ready_window().is_some() {
                        println!("JustPaste Smart Paste started; settings are in the tray menu.");
                        return Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                return Err(
                    "Smart Paste did not start; run just paste --background for diagnostics".into(),
                );
            }
            println!("JustPaste Smart Paste is already running.");
            Ok(())
        }
        "--disable" => {
            if let Some(window) = running {
                unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                    .map_err(|e| e.to_string())?;
            }
            println!(
                "Smart Paste stop requested. An active paste is cancelled first, then the background process exits."
            );
            Ok(())
        }
        "--startup" => match args[1].to_str() {
            Some("on") => startup(true),
            Some("off") => startup(false),
            _ => Err("--startup expects on or off".into()),
        },
        _ => Err("unknown Smart Paste control".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_suppression_never_leaks_to_other_apps_or_alt_v() {
        assert_eq!(suppress_followup(100, 100, WM_KEYDOWN, true), (100, true));
        assert_eq!(suppress_followup(100, 100, WM_KEYUP, false), (0, true));
        // Other app, Explorer address/rename field, changed pane, and lost key-up.
        for scope in [0, 101] {
            for message in [WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP] {
                assert_eq!(suppress_followup(100, scope, message, true), (0, false));
            }
        }
        // Alt+V must pass even if the preceding Ctrl+V's key-up was missed.
        assert_eq!(
            suppress_followup(100, 100, WM_SYSKEYDOWN, false),
            (0, false)
        );
        assert_eq!(suppress_followup(100, 100, WM_SYSKEYUP, false), (0, false));
        assert_eq!(suppress_followup(100, 100, WM_KEYDOWN, false), (0, false));
        assert_eq!(suppress_followup(0, 100, WM_KEYDOWN, true), (0, false));
    }

    #[test]
    fn only_ctrl_v_and_the_one_configured_picker_chord_are_handled() {
        // ctrl, shift, alt, win
        assert_eq!(chord(true, false, false, false, 0), 1);
        assert_eq!(chord(true, true, false, false, 1), 2);
        assert_eq!(chord(true, true, false, false, 2), 0);
        assert_eq!(chord(false, true, false, true, 2), 2);
        assert_eq!(chord(true, false, true, false, 3), 2);
        assert_eq!(chord(true, false, true, false, 1), 0);
        // Alt+V, Win+V (clipboard history), and Ctrl+Win+V stay native.
        assert_eq!(chord(false, false, true, false, 3), 0);
        assert_eq!(chord(false, false, false, true, 2), 0);
        assert_eq!(chord(true, false, false, true, 2), 0);
        assert_eq!(chord(true, true, false, false, 0), 0);
    }

    #[test]
    fn caret_blinking_does_not_disable_paste_but_menus_do() {
        assert!(!gui_is_busy(0));
        assert!(!gui_is_busy(GUI_CARETBLINKING.0));
        for flag in [
            GUI_INMENUMODE,
            GUI_INMOVESIZE,
            GUI_POPUPMENUMODE,
            GUI_SYSTEMMENUMODE,
        ] {
            assert!(gui_is_busy(flag.0 | GUI_CARETBLINKING.0));
        }
    }

    #[test]
    fn editing_and_navigation_controls_keep_native_paste() {
        assert!(file_control(UIA_ListControlTypeId));
        assert!(file_control(UIA_ListItemControlTypeId));
        assert!(!file_control(UIA_EditControlTypeId));
        assert!(!file_control(UIA_ComboBoxControlTypeId));
        assert!(!file_control(UIA_TreeItemControlTypeId));
    }

    #[test]
    fn only_clipboards_smart_paste_handles_leave_explorer() {
        const PNG: u32 = 0xC100;
        const DESCRIPTOR: u32 = 0xC102;
        let formats = [PNG, 0xC101, DESCRIPTOR, 0xC103];
        let with = |held: &'static [u32]| move |format| held.contains(&format);
        // Copied files are Explorer's unless copy filters are on, even beside an image or text.
        assert!(!wants_clipboard(
            false,
            formats,
            with(&[CF_HDROP, CF_UNICODETEXT])
        ));
        assert!(wants_clipboard(true, formats, with(&[CF_HDROP])));
        // Outlook attachments, zip contents and phones are virtual files: always Explorer's.
        assert!(!wants_clipboard(
            true,
            formats,
            with(&[DESCRIPTOR, CF_UNICODETEXT])
        ));
        assert!(wants_clipboard(false, formats, with(&[PNG])));
        assert!(wants_clipboard(false, formats, with(&[CF_DIB])));
        assert!(wants_clipboard(false, formats, with(&[CF_UNICODETEXT])));
        assert!(!wants_clipboard(false, formats, with(&[])));
        assert!(!wants_clipboard(false, [0; 4], with(&[0])));
    }

    #[test]
    #[ignore = "requires at least one open filesystem folder in Windows Explorer; read-only"]
    fn shell_apartment_queries_live_explorer_and_keeps_pumping() {
        let resolver = ShellResolver::new().unwrap();
        for _ in 0..2 {
            let views = resolver.inspect().unwrap();
            assert!(
                views
                    .iter()
                    .any(|view| view.directory.as_ref().is_some_and(|path| path.is_dir())),
                "no filesystem folder available in Explorer"
            );
            assert!(views.iter().all(|view| view.owner != 0 && view.window != 0));
        }
    }
}
