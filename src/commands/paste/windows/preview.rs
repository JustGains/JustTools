//! Screenshots of every JustPaste surface in light and dark, for review.
//! Run with `cargo test preview -- --ignored --test-threads=1`; images land
//! in `JUSTPASTE_PREVIEW_DIR` (the temp folder by default). These open real
//! menus and cards on screen for about a second each.
use super::theme::{self, PREVIEW};
use super::*;
use std::path::Path;
use std::time::Duration;
use windows::Win32::Graphics::Gdi::*;

fn folder() -> PathBuf {
    let folder = std::env::var_os("JUSTPASTE_PREVIEW_DIR").map_or_else(
        || std::env::temp_dir().join("justpaste-preview"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Save the screen inside `area` as a PNG.
fn capture(area: RECT, path: &Path) {
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    let mut pixels = vec![0_u8; (width * height * 4) as usize];
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap.into());
        BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            area.left,
            area.top,
            SRCCOPY,
        )
        .unwrap();
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        GetDIBits(
            memory,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
    }
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    image::RgbaImage::from_raw(width as u32, height as u32, pixels)
        .unwrap()
        .save(path)
        .unwrap();
}

fn grow(area: RECT, by: i32) -> RECT {
    RECT {
        left: area.left - by,
        top: area.top - by,
        right: area.right + by,
        bottom: area.bottom + by,
    }
}

/// Open `menu` at `point`, photograph it, and close it again.
fn shoot_menu(menu: &popup::Menu, point: POINT, name: &str) {
    let path = folder().join(name);
    unsafe {
        let owner = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("JustPaste preview"),
            WS_POPUP,
            point.x,
            point.y,
            0,
            0,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let target = owner.0 as usize;
        let photographer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            let menu = FindWindowW(w!("#32768"), None).expect("menu window");
            let mut area = RECT::default();
            GetWindowRect(menu, &mut area).unwrap();
            capture(grow(area, 14), &path);
            let _ = PostMessageW(
                Some(HWND(target as *mut _)),
                WM_CANCELMODE,
                WPARAM(0),
                LPARAM(0),
            );
        });
        let _ = SetForegroundWindow(owner);
        menu.show(owner, point, TPM_LEFTALIGN | TPM_TOPALIGN);
        photographer.join().unwrap();
        let _ = DestroyWindow(owner);
    }
}

fn spot() -> POINT {
    let area = theme::work_area(POINT { x: 0, y: 0 });
    POINT {
        x: area.left + 160,
        y: area.top + 120,
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(width, height)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

#[test]
#[ignore = "opens the picker and tray menus on screen and saves screenshots"]
fn preview_menus_in_light_and_dark() {
    theme::per_monitor_dpi();
    let settings = Settings {
        ai: Some(super::super::ai::Config::default()),
        custom: super::super::ai::custom_actions(
            "Summarize: Summarize it in 3 bullets
",
        ),
        ..Default::default()
    };
    let snapshots = [
        (
            "image",
            Snapshot::Image(png(1920, 1080), image::ImageFormat::Png),
            OnPaste::Act(Action::As(actions::Format::Png)),
        ),
        (
            "text",
            Snapshot::Html {
                html: "<p>Meeting notes</p>".into(),
                text: "Meeting notes: ship the new picker on Friday and send the launch post"
                    .into(),
            },
            OnPaste::Act(Action::Native),
        ),
        (
            "links",
            Snapshot::Text("https://www.youtube.com/watch?v=abc123".into()),
            OnPaste::Act(Action::Smart),
        ),
    ];
    for (mode, theme_name) in [(1, "light"), (2, "dark")] {
        PREVIEW.store(mode, Ordering::SeqCst);
        for (name, snapshot, default) in &snapshots {
            let entries = actions::entries(snapshot, &settings, true);
            let mut menu = popup::Menu::new(spot()).unwrap();
            picker::build(&mut menu, snapshot, &entries, true, default);
            shoot_menu(&menu, spot(), &format!("picker-{name}-{theme_name}.png"));
        }
        if let Ok(mut last) = LAST_FOLDER.lock() {
            *last = Some(PathBuf::from("C:\\"));
        }
        let menu = tray_menu(spot()).unwrap();
        shoot_menu(&menu, spot(), &format!("tray-{theme_name}.png"));
    }
    PREVIEW.store(0, Ordering::SeqCst);
}

/// Photograph the card in each state it can show.
#[test]
#[ignore = "opens the progress card on screen and saves screenshots"]
fn preview_progress_card_in_light_and_dark() {
    theme::per_monitor_dpi();
    for (mode, theme_name) in [(1, "light"), (2, "dark")] {
        PREVIEW.store(mode, Ordering::SeqCst);
        for (state, finish) in [
            ("working", None),
            ("downloading", None),
            (
                "saved",
                Some(Ok(
                    "saved C:\\Users\\Ada\\Downloads\\Launch video.mp4".to_owned()
                )),
            ),
            (
                "failed",
                Some(Err(
                    "yt-dlp could not download https://example.com/watch: this video is private"
                        .to_owned(),
                )),
            ),
        ] {
            let operation = crate::progress::Operation::begin();
            if state == "downloading" {
                operation.reporter.report(
                    "Downloading media…",
                    "Launch video.mp4 · 48 MB of 120 MB",
                    Some(40),
                );
            } else {
                operation.reporter.report("Saving your paste…", "", None);
            }
            let card = progress_window::start(
                operation.reporter.clone(),
                PathBuf::from("C:\\Users\\Ada\\Downloads"),
                true,
            );
            std::thread::sleep(Duration::from_millis(1300));
            if let Some(result) = &finish {
                operation.finish(result);
                std::thread::sleep(Duration::from_millis(250));
            }
            let window = unsafe { FindWindowW(w!("JustTools.SmartPaste.Progress.v2"), None) }
                .expect("card window");
            let mut area = RECT::default();
            unsafe { GetWindowRect(window, &mut area) }.unwrap();
            capture(
                grow(area, 18),
                &folder().join(format!("card-{state}-{theme_name}.png")),
            );
            if finish.is_none() {
                operation.finish(&Ok("done".into()));
            }
            unsafe {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            drop(operation);
            card.wait();
        }
    }
    PREVIEW.store(0, Ordering::SeqCst);
}
