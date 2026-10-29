//! The paste-as picker: one popup menu of every action that fits the clipboard.
use super::*;

const PICKER_CLASS: PCWSTR = w!("JustTools.SmartPaste.Picker.v1");

/// Make `window` the foreground window. The hook gave this process no
/// activation right, so borrow the current foreground thread's input state.
pub(super) fn take_foreground(window: HWND) {
    unsafe {
        let current = GetForegroundWindow();
        let theirs = GetWindowThreadProcessId(current, None);
        let ours = GetCurrentThreadId();
        let attached = theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let _ = SetForegroundWindow(window);
        let _ = BringWindowToTop(window);
        if attached {
            let _ = AttachThreadInput(ours, theirs, false);
        }
    }
}

unsafe extern "system" fn owner(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(window, message, wp, lp) }
}

fn register() -> Result<HINSTANCE, String> {
    static REGISTERED: OnceLock<Result<usize, String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| unsafe {
            let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(owner),
                hInstance: module.into(),
                lpszClassName: PICKER_CLASS,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err("cannot register the paste picker".into());
            }
            Ok(module.0 as usize)
        })
        .clone()
        .map(|module| HINSTANCE(module as *mut _))
}

/// Mnemonic for entry `index`: 1–9, then 0, then none.
fn mnemonic(index: usize, label: &str) -> String {
    let label = label.replace('&', "&&");
    match index {
        0..=8 => format!("&{}   {label}", index + 1),
        9 => format!("&0   {label}"),
        _ => format!("     {label}"),
    }
}

/// Show the picker near the cursor (or over the Explorer window) and return
/// the chosen action, or `None` when it was dismissed. Explorer is given the
/// foreground back either way.
pub(super) fn choose(
    request: &Request,
    snapshot: &Snapshot,
    settings: &Settings,
    default: &OnPaste,
) -> Result<Option<Action>, String> {
    let entries = actions::entries(
        snapshot,
        settings,
        super::super::cloudflare::Client::new().enabled(),
    );
    let explorer = HWND(request.window as *mut _);
    unsafe {
        let module = register()?;
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let mut bounds = RECT::default();
        let inside = |point: &POINT, bounds: &RECT| {
            (bounds.left..bounds.right).contains(&point.x)
                && (bounds.top..bounds.bottom).contains(&point.y)
        };
        if GetWindowRect(explorer, &mut bounds).is_ok() && !inside(&point, &bounds) {
            point = POINT {
                x: (bounds.left + bounds.right) / 2,
                y: (bounds.top + bounds.bottom) / 3,
            };
        }
        let owner = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PICKER_CLASS,
            w!("JustPaste"),
            WS_POPUP,
            point.x,
            point.y,
            0,
            0,
            None,
            None,
            Some(module),
            None,
        )
        .map_err(|e| e.to_string())?;
        let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
        if settings.picker.preview {
            let preview = wide(std::ffi::OsStr::new(&actions::preview(snapshot)));
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(preview.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        for (index, entry) in entries.iter().enumerate() {
            if index == 1 {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            }
            let label = wide(std::ffi::OsStr::new(&mnemonic(index, &entry.label)));
            let _ = AppendMenuW(menu, MF_STRING, index + 1, PCWSTR(label.as_ptr()));
        }
        // The Ctrl+V default is drawn bold, so the picker also teaches it.
        if let OnPaste::Act(default) = default
            && let Some(index) = entries.iter().position(|entry| &entry.action == default)
        {
            let _ = SetMenuDefaultItem(menu, index as u32 + 1, 0);
        }
        take_foreground(owner);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            Some(0),
            owner,
            None,
        )
        .0 as usize;
        let _ = DestroyMenu(menu);
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        take_foreground(explorer);
        let _ = DestroyWindow(owner);
        Ok(chosen
            .checked_sub(1)
            .and_then(|index| entries.into_iter().nth(index))
            .map(|entry| entry.action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_get_number_keys_and_escape_ampersands() {
        assert_eq!(mnemonic(0, "Paste normally"), "&1   Paste normally");
        assert_eq!(mnemonic(9, "A & B"), "&0   A && B");
        assert_eq!(mnemonic(10, "Late"), "     Late");
    }
}
