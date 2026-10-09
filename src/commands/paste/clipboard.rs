//! Bounded clipboard snapshots. Never changes the clipboard or holds it while working.
use std::path::PathBuf;
#[cfg(any(not(windows), test))]
mod unix;

#[derive(Debug)]
pub enum Snapshot {
    Text(String),
    Image(Vec<u8>, image::ImageFormat),
    Files { paths: Vec<PathBuf>, cut: bool },
    Unsupported,
}

#[cfg(windows)]
pub fn read() -> Result<Snapshot, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::UI::Shell::DragQueryFileW;
    struct Clipboard;
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
    unsafe fn bytes(format: u32) -> Option<Vec<u8>> {
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return None;
        }
        let size = unsafe { GlobalSize(handle) };
        if size == 0 || size > 256 * 1024 * 1024 {
            return None;
        }
        let pointer = unsafe { GlobalLock(handle) } as *const u8;
        if pointer.is_null() {
            return None;
        }
        let result = unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec();
        unsafe {
            GlobalUnlock(handle);
        }
        Some(result)
    }
    fn format(name: &str) -> u32 {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe { RegisterClipboardFormatW(name.as_ptr()) }
    }
    let mut opened = false;
    for _ in 0..10 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !opened {
        return Err("the clipboard is busy; try again".into());
    }
    let _guard = Clipboard;
    unsafe {
        if IsClipboardFormatAvailable(15) != 0 {
            let drop = GetClipboardData(15);
            if drop.is_null() {
                return Err("cannot read copied files".into());
            }
            let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            if count > 100_000 {
                return Err("too many copied files".into());
            }
            let mut paths = Vec::new();
            for index in 0..count {
                let length = DragQueryFileW(drop, index, std::ptr::null_mut(), 0);
                if length == 0 || length > 32768 {
                    return Err("invalid copied path".into());
                }
                let mut buffer = vec![0; length as usize + 1];
                let written = DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32);
                paths.push(PathBuf::from(std::ffi::OsString::from_wide(
                    &buffer[..written as usize],
                )));
            }
            let cut = bytes(format("Preferred DropEffect"))
                .and_then(|b| {
                    b.get(..4)
                        .and_then(|b| b.try_into().ok())
                        .map(u32::from_le_bytes)
                })
                .is_some_and(|effect| effect & 2 != 0);
            return Ok(Snapshot::Files { paths, cut });
        }
        // Outlook attachments and other virtual file transfers belong to Explorer.
        if IsClipboardFormatAvailable(format("FileGroupDescriptorW")) != 0
            || IsClipboardFormatAvailable(format("FileGroupDescriptor")) != 0
        {
            return Ok(Snapshot::Unsupported);
        }
        for name in ["PNG", "image/png"] {
            if let Some(png) = bytes(format(name)) {
                return Ok(Snapshot::Image(png, image::ImageFormat::Png));
            }
        }
        for dib_format in [17, 8] {
            if let Some(dib) = bytes(dib_format) {
                return Ok(Snapshot::Image(dib_to_bmp(&dib)?, image::ImageFormat::Bmp));
            }
        }
        if let Some(raw) = bytes(13) {
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .take_while(|unit| *unit != 0)
                .collect();
            return Ok(Snapshot::Text(String::from_utf16_lossy(&units)));
        }
    }
    Ok(Snapshot::Unsupported)
}

#[cfg(not(windows))]
pub fn read() -> Result<Snapshot, String> {
    unix::read()
}

#[cfg(any(windows, test))]
fn dib_to_bmp(dib: &[u8]) -> Result<Vec<u8>, String> {
    let u32_at = |offset| {
        dib.get(offset..offset + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| "truncated clipboard bitmap".to_owned())
    };
    let header = u32_at(0)? as usize;
    if !matches!(header, 40 | 52 | 56 | 108 | 124) || dib.len() < header {
        return Err("unsupported clipboard bitmap header".into());
    }
    let depth = u16::from_le_bytes([dib[14], dib[15]]);
    let compression = u32_at(16)?;
    let colors = u32_at(32)? as usize;
    let palette = if colors > 0 {
        colors
    } else if depth <= 8 {
        1_usize << depth
    } else {
        0
    };
    let masks = if header == 40 {
        match compression {
            3 => 12,
            6 => 16,
            _ => 0,
        }
    } else {
        0
    };
    let offset = header
        .checked_add(masks)
        .and_then(|n| palette.checked_mul(4).and_then(|p| n.checked_add(p)))
        .filter(|offset| *offset <= dib.len())
        .ok_or("invalid clipboard bitmap layout")?;
    let mut bmp = Vec::with_capacity(dib.len() + 14);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((dib.len() + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((offset + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_bottom_up_dib_and_rejects_truncated_header() {
        let mut dib = vec![0; 44];
        dib[..4].copy_from_slice(&40_u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1_i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1_i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24_u16.to_le_bytes());
        dib[40..43].copy_from_slice(&[30, 20, 10]);
        let decoded = image::load_from_memory(&dib_to_bmp(&dib).unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert!(dib_to_bmp(&[0; 4]).is_err());
    }
}
