//! The paste-as picker: one menu of every action that fits the clipboard,
//! grouped, with an icon per action and a number key on the right.
use super::popup::{self, Menu};
use super::theme::glyph;
use super::*;
use actions::Format;

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
    if message == WM_MENUCHAR
        && let Some(result) = popup::menu_char(wp)
    {
        return result;
    }
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

/// The icon for what is on the clipboard.
fn content(snapshot: &Snapshot) -> char {
    match snapshot {
        Snapshot::Image(..) => glyph::PICTURE,
        Snapshot::Files { .. } => glyph::FOLDER,
        Snapshot::Text(text) | Snapshot::Html { text, .. }
            if !super::super::links(text).is_empty() =>
        {
            glyph::LINK
        }
        Snapshot::Html { .. } => glyph::GLOBE,
        Snapshot::Text(_) => glyph::DOCUMENT,
        Snapshot::Unsupported => glyph::UNKNOWN,
    }
}

/// The icon for one action.
fn icon(action: &Action, snapshot: &Snapshot) -> char {
    match action {
        Action::Native => glyph::PASTE,
        Action::Smart if matches!(snapshot, Snapshot::Files { .. }) => glyph::COPY,
        Action::Smart => glyph::DOWNLOAD,
        Action::PageMarkdown => glyph::LINK,
        Action::As(format) => match format {
            Format::Txt => glyph::DOCUMENT,
            Format::Md => glyph::LIST,
            Format::Html => glyph::GLOBE,
            Format::Json => glyph::CODE,
            Format::Png | Format::Jpg | Format::Webp => glyph::PICTURE,
            Format::Rmbg => glyph::CUT,
            Format::Ocr => glyph::FONT,
            Format::Mp3 => glyph::AUDIO,
            Format::Mp4 => glyph::VIDEO,
        },
        Action::Ai { prompt, .. } if prompt.is_empty() => glyph::CHAT,
        Action::Ai { name, .. } if name == super::super::ai::FIX_SPELLING => glyph::EDIT,
        Action::Ai { .. } => glyph::BOLT,
    }
}

/// Fill `menu` with the caption and grouped entries; entry `i` gets ID `i + 1`.
pub(super) fn build(
    menu: &mut Menu,
    snapshot: &Snapshot,
    entries: &[actions::Entry],
    preview: bool,
    default: &OnPaste,
) {
    if preview {
        menu.caption(content(snapshot), &actions::preview(snapshot), "");
        menu.separator();
    }
    let mut section = None;
    for (index, entry) in entries.iter().enumerate() {
        if section.is_some_and(|section| section != entry.section()) {
            menu.separator();
        }
        section = Some(entry.section());
        let key = popup::number(index);
        let hint = key.map(String::from).unwrap_or_default();
        menu.item(
            index + 1,
            icon(&entry.action, snapshot),
            &entry.label,
            &hint,
            key,
            true,
        );
    }
    // The Ctrl+V default is drawn bold, so the picker also teaches it.
    if let OnPaste::Act(default) = default
        && let Some(index) = entries.iter().position(|entry| &entry.action == default)
    {
        menu.default(index + 1);
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
        let mut menu = Menu::new(point)?;
        build(
            &mut menu,
            snapshot,
            &entries,
            settings.picker.preview,
            default,
        );
        take_foreground(owner);
        let chosen = menu.show(
            owner,
            point,
            TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
        );
        drop(menu);
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
    fn every_action_has_its_own_fitting_icon() {
        let image = Snapshot::Image(Vec::new(), image::ImageFormat::Png);
        let files = Snapshot::Files {
            paths: Vec::new(),
            cut: false,
        };
        assert_eq!(icon(&Action::Native, &image), glyph::PASTE);
        assert_eq!(icon(&Action::Smart, &files), glyph::COPY);
        assert_eq!(
            icon(&Action::Smart, &Snapshot::Text("https://x.test".into())),
            glyph::DOWNLOAD
        );
        assert_eq!(icon(&Action::As(Format::Rmbg), &image), glyph::CUT);
        assert_eq!(
            icon(
                &Action::Ai {
                    name: "AI paste".into(),
                    prompt: String::new()
                },
                &image
            ),
            glyph::CHAT
        );
        assert_eq!(
            icon(
                &Action::Ai {
                    name: super::super::super::ai::FIX_SPELLING.into(),
                    prompt: "fix".into()
                },
                &image
            ),
            glyph::EDIT
        );
        assert_eq!(content(&image), glyph::PICTURE);
        assert_eq!(
            content(&Snapshot::Text("see https://x.test".into())),
            glyph::LINK
        );
    }
}
