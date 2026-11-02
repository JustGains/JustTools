//! The Windows look JustPaste follows: light or dark apps, the accent color,
//! Fluent icon glyphs, crisp per-monitor scaling, and dark native menus.
//! Every query falls back to the light theme instead of failing.
use super::*;
use windows::Win32::{
    Graphics::{Dwm::*, Gdi::*},
    System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
    UI::HiDpi::*,
};
use winreg::{RegKey, enums::*};

/// Icon glyphs shared by Segoe Fluent Icons (Windows 11) and Segoe MDL2
/// Assets (Windows 10).
pub(super) mod glyph {
    pub const PASTE: char = '\u{E77F}';
    pub const COPY: char = '\u{E8C8}';
    pub const CUT: char = '\u{E8C6}';
    pub const DOWNLOAD: char = '\u{E896}';
    pub const LINK: char = '\u{E71B}';
    pub const GLOBE: char = '\u{E774}';
    pub const DOCUMENT: char = '\u{E8A5}';
    pub const LIST: char = '\u{E8FD}';
    pub const CODE: char = '\u{E943}';
    pub const PICTURE: char = '\u{E8B9}';
    pub const FONT: char = '\u{E8D2}';
    pub const AUDIO: char = '\u{E8D6}';
    pub const VIDEO: char = '\u{E714}';
    pub const FOLDER: char = '\u{E8B7}';
    pub const FOLDER_OPEN: char = '\u{E838}';
    pub const EDIT: char = '\u{E70F}';
    pub const BOLT: char = '\u{E945}';
    pub const CHAT: char = '\u{E8F2}';
    pub const SETTINGS: char = '\u{E713}';
    pub const PAUSE: char = '\u{E769}';
    pub const PLAY: char = '\u{E768}';
    pub const CANCEL: char = '\u{E711}';
    pub const POWER: char = '\u{E7E8}';
    pub const CLOSE: char = '\u{E8BB}';
    pub const DONE: char = '\u{E930}';
    pub const WARNING: char = '\u{E7BA}';
    pub const UNKNOWN: char = '\u{E9CE}';
}

pub(super) const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    COLORREF(red as u32 | (green as u32) << 8 | (blue as u32) << 16)
}

/// Colors for JustPaste's own surfaces, matching Windows 11 flyouts.
#[derive(Clone, Copy, Debug)]
pub(super) struct Palette {
    pub background: COLORREF,
    pub hover: COLORREF,
    pub pressed: COLORREF,
    /// A button's face and outline.
    pub control: COLORREF,
    pub stroke: COLORREF,
    pub text: COLORREF,
    pub secondary: COLORREF,
    pub track: COLORREF,
    pub accent: COLORREF,
    pub success: COLORREF,
    pub error: COLORREF,
}

impl Palette {
    pub fn current() -> Self {
        if high_contrast() {
            let system = |index| COLORREF(unsafe { GetSysColor(index) });
            return Self {
                background: system(COLOR_WINDOW),
                hover: system(COLOR_HIGHLIGHT),
                pressed: system(COLOR_HIGHLIGHT),
                control: system(COLOR_BTNFACE),
                stroke: system(COLOR_WINDOWTEXT),
                text: system(COLOR_WINDOWTEXT),
                secondary: system(COLOR_WINDOWTEXT),
                track: system(COLOR_GRAYTEXT),
                accent: system(COLOR_HIGHLIGHT),
                success: system(COLOR_WINDOWTEXT),
                error: system(COLOR_WINDOWTEXT),
            };
        }
        let dark = dark();
        if dark {
            Self {
                background: rgb(0x2c, 0x2c, 0x2c),
                hover: rgb(0x3a, 0x3a, 0x3a),
                pressed: rgb(0x34, 0x34, 0x34),
                control: rgb(0x37, 0x37, 0x37),
                stroke: rgb(0x45, 0x45, 0x45),
                text: rgb(0xff, 0xff, 0xff),
                secondary: rgb(0xc5, 0xc5, 0xc5),
                track: rgb(0x5c, 0x5c, 0x5c),
                accent: accent(true),
                success: rgb(0x6c, 0xcb, 0x5f),
                error: rgb(0xff, 0x99, 0xa4),
            }
        } else {
            Self {
                background: rgb(0xf9, 0xf9, 0xf9),
                hover: rgb(0xec, 0xec, 0xec),
                pressed: rgb(0xe3, 0xe3, 0xe3),
                control: rgb(0xff, 0xff, 0xff),
                stroke: rgb(0xe0, 0xe0, 0xe0),
                text: rgb(0x1a, 0x1a, 0x1a),
                secondary: rgb(0x5f, 0x5f, 0x5f),
                track: rgb(0xd1, 0xd1, 0xd1),
                accent: accent(false),
                success: rgb(0x0f, 0x7b, 0x0f),
                error: rgb(0xc4, 0x2b, 0x1c),
            }
        }
    }
}

/// Previews pin a theme without touching Windows settings: 0 follows
/// Windows, 1 is light, 2 is dark.
#[cfg(test)]
pub(super) static PREVIEW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether Windows shows apps in dark mode (never under high contrast).
pub(super) fn dark() -> bool {
    #[cfg(test)]
    match PREVIEW.load(Ordering::SeqCst) {
        1 => return false,
        2 => return true,
        _ => {}
    }
    !high_contrast()
        && RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|key| key.get_value::<u32, _>("AppsUseLightTheme"))
            .is_ok_and(|light| light == 0)
}

pub(super) fn high_contrast() -> bool {
    let mut contrast = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok()
        && (contrast.dwFlags & HCF_HIGHCONTRASTON).0 != 0
}

/// The accent shade Windows itself uses on this background: a lighter one
/// on dark surfaces, a deeper one on light surfaces. JustPaste violet when
/// the palette is unavailable.
fn accent(dark: bool) -> COLORREF {
    let palette = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
        .and_then(|key| key.get_raw_value("AccentPalette"))
        .map(|value| value.bytes);
    match palette {
        Ok(bytes) if bytes.len() >= 32 => {
            let at = if dark { 4 } else { 16 };
            rgb(bytes[at], bytes[at + 1], bytes[at + 2])
        }
        _ if dark => rgb(0xa8, 0x9b, 0xff),
        _ => rgb(0x5b, 0x45, 0xe0),
    }
}

/// Render pixels at the monitor's real resolution, so menus and cards stay
/// sharp at 125–200% scaling instead of being stretched by Windows.
pub(super) fn per_monitor_dpi() {
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_err() {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        }
    }
}

/// Scale this thread's new windows by the system DPI until dropped, then
/// return to the thread's previous scaling (the picker needs per-monitor).
pub(super) struct SystemDpi(DPI_AWARENESS_CONTEXT);

impl SystemDpi {
    pub fn begin() -> Self {
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE) })
    }
}

impl Drop for SystemDpi {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
}

pub(super) fn monitor_dpi(point: POINT) -> u32 {
    let (mut x, mut y) = (0, 0);
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y)
    }
    .map_or(96, |()| x.max(96))
}

/// The work area (the screen minus the taskbar) of the monitor at `point`.
pub(super) fn work_area(point: POINT) -> RECT {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(monitor, &mut info);
    }
    info.rcWork
}

/// Windows 11 rounded corners; ignored by Windows 10.
pub(super) fn round_corners(window: HWND) {
    let preference = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

fn installed(face: &str) -> bool {
    unsafe {
        let font = font(16, 400, face, DEFAULT_QUALITY);
        let dc = CreateCompatibleDC(None);
        let previous = SelectObject(dc, font.into());
        let mut name = [0_u16; 32];
        let length = GetTextFaceW(dc, Some(&mut name)).max(0) as usize;
        SelectObject(dc, previous);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(font.into());
        String::from_utf16_lossy(&name[..length.saturating_sub(1).min(32)])
            .trim_end_matches('\0')
            .eq_ignore_ascii_case(face)
    }
}

fn font(pixels: i32, weight: i32, face: &str, quality: FONT_QUALITY) -> HFONT {
    let mut info = LOGFONTW {
        lfHeight: -pixels,
        lfWeight: weight,
        lfQuality: quality,
        ..Default::default()
    };
    fill(&mut info.lfFaceName, face);
    unsafe { CreateFontIndirectW(&info) }
}

/// Segoe UI Variable on Windows 11, Segoe UI before it.
pub(super) fn ui_font(pixels: i32, weight: i32) -> HFONT {
    static FACE: OnceLock<&str> = OnceLock::new();
    let face = FACE.get_or_init(|| {
        if installed("Segoe UI Variable Text") {
            "Segoe UI Variable Text"
        } else {
            "Segoe UI"
        }
    });
    font(pixels, weight, face, CLEARTYPE_QUALITY)
}

/// Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10.
pub(super) fn icon_font(pixels: i32) -> HFONT {
    static FACE: OnceLock<&str> = OnceLock::new();
    let face = FACE.get_or_init(|| {
        if installed("Segoe Fluent Icons") {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        }
    });
    font(pixels, 400, face, ANTIALIASED_QUALITY)
}

/// `glyph` in `color` as a premultiplied 32-bit bitmap, the form Windows
/// blends into themed menus. The caller deletes it.
pub(super) fn glyph_bitmap(glyph: char, size: i32, color: COLORREF) -> Option<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let dc = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(dc);
            return None;
        };
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize);
        pixels.fill(0);
        let previous = SelectObject(dc, bitmap.into());
        let font = icon_font(size);
        let previous_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, rgb(0xff, 0xff, 0xff));
        let mut area = RECT {
            right: size,
            bottom: size,
            ..Default::default()
        };
        let mut text: Vec<u16> = glyph.encode_utf16(&mut [0; 2]).to_vec();
        DrawTextW(
            dc,
            &mut text,
            &mut area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        let _ = GdiFlush();
        let [red, green, blue] = [
            color.0 & 0xff,
            (color.0 >> 8) & 0xff,
            (color.0 >> 16) & 0xff,
        ];
        for pixel in pixels.iter_mut() {
            // White-on-black coverage becomes alpha for the wanted color.
            let alpha = (*pixel >> 8) & 0xff;
            let scale = |channel: u32| channel * alpha / 255;
            *pixel = alpha << 24 | scale(red) << 16 | scale(green) << 8 | scale(blue);
        }
        SelectObject(dc, previous_font);
        SelectObject(dc, previous);
        let _ = DeleteObject(font.into());
        let _ = DeleteDC(dc);
        Some(bitmap)
    }
}

/// Draw `glyph` centered in `area` of `dc`, which already holds an icon font.
pub(super) fn draw_glyph(dc: HDC, glyph: char, area: RECT, color: COLORREF) {
    let mut area = area;
    let mut text: Vec<u16> = glyph.encode_utf16(&mut [0; 2]).to_vec();
    unsafe {
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            &mut text,
            &mut area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    }
}

/// Let native menus follow Windows' dark app mode. The switch lives in
/// uxtheme's unnamed exports, present since Windows 10 1903 and used by
/// Explorer itself; older builds keep light menus.
pub(super) fn dark_menus(owner: HWND) {
    type AppMode = unsafe extern "system" fn(i32) -> i32;
    type WindowMode = unsafe extern "system" fn(HWND, windows::core::BOOL) -> windows::core::BOOL;
    type Flush = unsafe extern "system" fn();
    static EXPORTS: OnceLock<Option<(usize, usize, usize)>> = OnceLock::new();
    let exports = EXPORTS.get_or_init(|| unsafe {
        let build = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            .and_then(|key| key.get_value::<String, _>("CurrentBuildNumber"))
            .ok()
            .and_then(|build| build.parse::<u32>().ok())?;
        if build < 18362 {
            return None;
        }
        let theme = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let export = |ordinal: usize| {
            GetProcAddress(theme, windows::core::PCSTR(ordinal as *const u8))
                .map(|function| function as usize)
        };
        Some((export(133)?, export(135)?, export(136)?))
    });
    // 1 follows Windows: dark menus only while apps are dark.
    #[cfg(not(test))]
    let mode = 1;
    #[cfg(test)]
    let mode = match PREVIEW.load(Ordering::SeqCst) {
        1 => 3,
        2 => 2,
        _ => 1,
    };
    if let Some((window, app, flush)) = *exports {
        unsafe {
            std::mem::transmute::<usize, AppMode>(app)(mode);
            let _ = std::mem::transmute::<usize, WindowMode>(window)(owner, true.into());
            std::mem::transmute::<usize, Flush>(flush)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_pack_red_green_blue_as_windows_expects() {
        assert_eq!(rgb(0x12, 0x34, 0x56).0, 0x0056_3412);
    }

    #[test]
    fn every_glyph_renders_with_visible_pixels() {
        for glyph in [
            glyph::PASTE,
            glyph::DOWNLOAD,
            glyph::PICTURE,
            glyph::CUT,
            glyph::SETTINGS,
            glyph::DONE,
            glyph::WARNING,
        ] {
            let bitmap = glyph_bitmap(glyph, 16, rgb(0, 0, 0)).expect("bitmap");
            let mut pixels = [0_u32; 256];
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 16,
                    biHeight: -16,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            unsafe {
                let dc = CreateCompatibleDC(None);
                GetDIBits(
                    dc,
                    bitmap,
                    0,
                    16,
                    Some(pixels.as_mut_ptr().cast()),
                    &info as *const _ as *mut _,
                    DIB_RGB_COLORS,
                );
                let _ = DeleteDC(dc);
                let _ = DeleteObject(bitmap.into());
            }
            assert!(
                pixels.iter().filter(|pixel| *pixel >> 24 > 128).count() > 4,
                "glyph {:X} is missing",
                glyph as u32
            );
        }
    }
}
