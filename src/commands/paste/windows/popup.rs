//! Native popup menus in the Windows 11 style: Fluent icons, right-aligned
//! hints, number keys, and dark mode whenever Windows apps are dark. Native
//! menus keep the system's keyboard handling, placement, and screen readers.
use super::theme::{self, Palette};
use super::*;
use std::cell::RefCell;
use windows::Win32::{Graphics::Gdi::*, UI::HiDpi::GetSystemMetricsForDpi};

thread_local! {
    /// While a menu is open on this thread: each number key and the menu
    /// position it runs. The owner's `WM_MENUCHAR` reads it.
    static KEYS: RefCell<Vec<(u16, u32)>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct Menu {
    handle: HMENU,
    bitmaps: Vec<HBITMAP>,
    keys: Vec<(u16, u32)>,
    count: u32,
    icon: i32,
    color: COLORREF,
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.handle);
            for bitmap in &self.bitmaps {
                let _ = DeleteObject((*bitmap).into());
            }
        }
    }
}

/// A label as menus draw it: `&` is literal, and a hint sits right-aligned.
pub(super) fn text(label: &str, hint: &str) -> String {
    let label = label.replace('&', "&&");
    if hint.is_empty() {
        label
    } else {
        format!("{label}\t{}", hint.replace('&', "&&"))
    }
}

/// Number key for picker entry `index`: 1–9, then 0, then none.
pub(super) fn number(index: usize) -> Option<char> {
    match index {
        0..=8 => char::from_digit(index as u32 + 1, 10),
        9 => Some('0'),
        _ => None,
    }
}

impl Menu {
    /// A menu opening at `point`, sized for that monitor.
    pub fn new(point: POINT) -> Result<Self, String> {
        let handle = unsafe { CreatePopupMenu() }.map_err(|e| e.to_string())?;
        let info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_STYLE,
            // One gutter for icons and check marks, as Explorer's menus have.
            dwStyle: MNS_CHECKORBMP,
            ..Default::default()
        };
        let _ = unsafe { SetMenuInfo(handle, &info) };
        let dpi = theme::monitor_dpi(point);
        Ok(Self {
            handle,
            bitmaps: Vec::new(),
            keys: Vec::new(),
            count: 0,
            icon: unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.max(16),
            color: if theme::high_contrast() {
                COLORREF(unsafe { GetSysColor(COLOR_MENUTEXT) })
            } else {
                Palette::current().text
            },
        })
    }

    fn append(&mut self, flags: MENU_ITEM_FLAGS, id: usize, label: &str, glyph: Option<char>) {
        let label = wide(std::ffi::OsStr::new(label));
        unsafe {
            let _ = AppendMenuW(self.handle, flags, id, PCWSTR(label.as_ptr()));
            if let Some(glyph) = glyph
                && let Some(bitmap) = theme::glyph_bitmap(glyph, self.icon, self.color)
            {
                let info = MENUITEMINFOW {
                    cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_BITMAP,
                    hbmpItem: bitmap,
                    ..Default::default()
                };
                let _ = SetMenuItemInfoW(self.handle, self.count, true, &info);
                self.bitmaps.push(bitmap);
            }
        }
        self.count += 1;
    }

    /// A line describing what follows; not a command.
    pub fn caption(&mut self, glyph: char, label: &str, hint: &str) {
        self.append(MF_STRING | MF_GRAYED, 0, &text(label, hint), Some(glyph));
    }

    /// A command. `key` also runs it while the menu is open.
    pub fn item(
        &mut self,
        id: usize,
        glyph: char,
        label: &str,
        hint: &str,
        key: Option<char>,
        enabled: bool,
    ) {
        if let Some(key) = key {
            self.keys.push((key as u16, self.count));
        }
        let state = if enabled { MF_ENABLED } else { MF_GRAYED };
        self.append(MF_STRING | state, id, &text(label, hint), Some(glyph));
    }

    /// An on/off setting, shown with a check mark.
    pub fn check(&mut self, id: usize, label: &str, checked: bool) {
        let state = if checked { MF_CHECKED } else { MF_UNCHECKED };
        self.append(MF_STRING | state, id, &text(label, ""), None);
    }

    pub fn separator(&mut self) {
        self.append(MF_SEPARATOR, 0, "", None);
    }

    /// Draw command `id` bold: what a click or Ctrl+V does by default.
    pub fn default(&self, id: usize) {
        let _ = unsafe { SetMenuDefaultItem(self.handle, id as u32, 0) };
    }

    /// Show the menu and return the chosen command, or 0 when dismissed.
    pub fn show(&self, owner: HWND, point: POINT, flags: TRACK_POPUP_MENU_FLAGS) -> usize {
        theme::dark_menus(owner);
        KEYS.with(|keys| keys.replace(self.keys.clone()));
        let chosen = unsafe {
            TrackPopupMenu(
                self.handle,
                flags | TPM_RETURNCMD,
                point.x,
                point.y,
                Some(0),
                owner,
                None,
            )
        }
        .0 as usize;
        KEYS.with(|keys| keys.borrow_mut().clear());
        let _ = unsafe { PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0)) };
        chosen
    }
}

/// The owner window's answer to `WM_MENUCHAR`: a number key runs its entry.
pub(super) fn menu_char(wparam: WPARAM) -> Option<LRESULT> {
    let key = (wparam.0 & 0xffff) as u16;
    KEYS.with(|keys| {
        keys.borrow()
            .iter()
            .find(|(known, _)| *known == key)
            .map(|(_, position)| LRESULT(((MNC_EXECUTE as isize) << 16) | *position as isize))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_keep_ampersands_and_put_hints_in_the_right_column() {
        assert_eq!(text("Paste normally", ""), "Paste normally");
        assert_eq!(text("A & B", "Ctrl+V"), "A && B\tCtrl+V");
        assert_eq!(number(0), Some('1'));
        assert_eq!(number(8), Some('9'));
        assert_eq!(number(9), Some('0'));
        assert_eq!(number(10), None);
    }

    #[test]
    fn number_keys_run_their_position_including_captions_and_separators() {
        let mut menu = Menu::new(POINT::default()).unwrap();
        menu.caption(theme::glyph::PICTURE, "Image · 4 × 3", "");
        menu.separator();
        menu.item(
            1,
            theme::glyph::PASTE,
            "Paste normally",
            "1",
            number(0),
            true,
        );
        menu.separator();
        menu.item(2, theme::glyph::PICTURE, "PNG image", "2", number(1), true);
        KEYS.with(|keys| keys.replace(menu.keys.clone()));
        let run = |key: char| menu_char(WPARAM(key as usize)).map(|result| result.0);
        assert_eq!(run('1'), Some(((MNC_EXECUTE as isize) << 16) | 2));
        assert_eq!(run('2'), Some(((MNC_EXECUTE as isize) << 16) | 4));
        assert_eq!(run('3'), None);
        KEYS.with(|keys| keys.borrow_mut().clear());
        assert_eq!(menu.bitmaps.len(), 3);
    }
}
