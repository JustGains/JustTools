use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[cfg(not(windows))]
use std::fs::{self, OpenOptions};
#[cfg(not(windows))]
use std::io::{ErrorKind, Write};

#[cfg(not(windows))]
use directories::BaseDirs;

use crate::error::{ToolError, ToolResult};

#[cfg(any(not(windows), test))]
const BLOCK_BEGIN: &str = "# >>> JustTools PATH >>>";
#[cfg(any(not(windows), test))]
const BLOCK_END: &str = "# <<< JustTools PATH <<<";

fn key(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = resolved
        .to_string_lossy()
        .trim_matches('"')
        .trim_end_matches(['/', '\\'])
        .replace('\\', "/");
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

pub fn contains(directory: &Path) -> bool {
    let target = key(directory);
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|entry| key(&entry) == target)
}

#[cfg(windows)]
fn contains_path_separator(value: &OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .any(|character| character == u16::from(b';'))
}

#[cfg(not(windows))]
fn contains_path_separator(value: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().contains(&b':')
}

fn validate_directory(directory: &Path) -> ToolResult {
    if directory.as_os_str().is_empty() {
        return Err(ToolError::new(
            "just",
            "cannot add an empty directory to PATH",
        ));
    }
    if contains_path_separator(directory.as_os_str()) {
        let separator = if cfg!(windows) { ';' } else { ':' };
        return Err(ToolError::new(
            "just",
            format!(
                "cannot add {} to PATH because it contains the PATH separator `{separator}`",
                directory.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn add_platform(directory: &Path) -> ToolResult {
    use std::io::ErrorKind;
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::{FromRegValue, ToRegValue};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let environment = hkcu
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|error| {
            ToolError::new("just", format!("cannot open user environment: {error}"))
        })?;
    let (current, value_type) = match environment.get_raw_value("Path") {
        Ok(raw) => {
            let value_type = match raw.vtype {
                REG_EXPAND_SZ => REG_EXPAND_SZ,
                _ => REG_SZ,
            };
            let current = String::from_reg_value(&raw).map_err(|error| {
                ToolError::new("just", format!("cannot read user PATH: {error}"))
            })?;
            (current, value_type)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => (String::new(), REG_EXPAND_SZ),
        Err(error) => {
            return Err(ToolError::new(
                "just",
                format!("cannot read user PATH: {error}"),
            ));
        }
    };
    let already_present =
        std::env::split_paths(OsStr::new(&current)).any(|entry| key(&entry) == key(directory));
    if already_present {
        println!("just: already on user PATH: {}", directory.display());
        return Ok(());
    }
    let mut updated = current.trim_end_matches(';').to_owned();
    if !updated.is_empty() {
        updated.push(';');
    }
    updated.push_str(&directory.to_string_lossy());
    let mut raw = updated.to_reg_value();
    // Retain REG_EXPAND_SZ so existing %VARIABLE% entries keep expanding.
    raw.vtype = value_type;
    environment
        .set_raw_value("Path", &raw)
        .map_err(|error| ToolError::new("just", format!("cannot update user PATH: {error}")))?;

    let message: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result = 0usize;
    // Notify desktop processes so terminals opened after this point inherit the change.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            message.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            3_000,
            &mut result,
        );
    }
    println!("just: added to user PATH: {}", directory.display());
    println!("just: open a new terminal to use every just* command.");
    Ok(())
}

#[cfg(any(not(windows), test))]
fn shell_quote(value: &Path) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(any(not(windows), test))]
fn unix_profile(home: &Path, shell_name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    match shell_name {
        "fish" => home.join(".config/fish/config.fish"),
        "zsh" => home.join(".zprofile"),
        "bash" => {
            let bash_profile = home.join(".bash_profile");
            if exists(&bash_profile) {
                return bash_profile;
            }
            let bash_login = home.join(".bash_login");
            if exists(&bash_login) {
                return bash_login;
            }
            home.join(".profile")
        }
        _ => home.join(".profile"),
    }
}

#[cfg(any(not(windows), test))]
fn managed_profile_contents(existing: &str, line: &str) -> Result<Option<String>, String> {
    let replacement = format!("{BLOCK_BEGIN}\n{line}\n{BLOCK_END}");
    if let Some(begin) = existing.find(BLOCK_BEGIN) {
        let after_begin = begin + BLOCK_BEGIN.len();
        let relative_end = existing[after_begin..]
            .find(BLOCK_END)
            .ok_or_else(|| "found an incomplete JustTools PATH block".to_owned())?;
        let end = after_begin + relative_end + BLOCK_END.len();
        let mut updated = String::with_capacity(existing.len() + replacement.len());
        updated.push_str(&existing[..begin]);
        updated.push_str(&replacement);
        updated.push_str(&existing[end..]);
        return Ok((updated != existing).then_some(updated));
    }
    if existing.contains(BLOCK_END) {
        return Err("found an incomplete JustTools PATH block".to_owned());
    }
    // Recognize the one-line format used by older JustTools releases without
    // adding a second entry. New writes always use the bounded block above.
    if existing
        .lines()
        .any(|existing_line| existing_line.trim() == line)
    {
        return Ok(None);
    }

    let mut updated = existing.to_owned();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(&replacement);
    updated.push('\n');
    Ok(Some(updated))
}

#[cfg(not(windows))]
fn add_platform(directory: &Path) -> ToolResult {
    let base =
        BaseDirs::new().ok_or_else(|| ToolError::new("just", "cannot locate home directory"))?;
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell_name = Path::new(&shell)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("");
    let profile = unix_profile(base.home_dir(), shell_name, Path::exists);
    let line = if shell_name == "fish" {
        format!("fish_add_path --global {}", shell_quote(directory))
    } else {
        format!("export PATH={}:$PATH", shell_quote(directory))
    };
    let existing = match fs::read_to_string(&profile) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(ToolError::new(
                "just",
                format!("cannot read {}: {error}", profile.display()),
            ));
        }
    };
    let Some(updated) = managed_profile_contents(&existing, &line).map_err(|error| {
        ToolError::new(
            "just",
            format!("cannot edit {}: {error}", profile.display()),
        )
    })?
    else {
        println!("just: already configured in {}", profile.display());
        return Ok(());
    };
    if let Some(parent) = profile.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            ToolError::new(
                "just",
                format!("cannot create {}: {error}", parent.display()),
            )
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&profile)
        .map_err(|error| {
            ToolError::new(
                "just",
                format!("cannot edit {}: {error}", profile.display()),
            )
        })?;
    file.write_all(updated.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            ToolError::new(
                "just",
                format!("cannot update {}: {error}", profile.display()),
            )
        })?;
    println!(
        "just: added {} to PATH in {}",
        directory.display(),
        profile.display()
    );
    println!("just: open a new terminal to use every just* command.");
    Ok(())
}

pub fn add(directory: &Path) -> ToolResult {
    validate_directory(directory)?;
    if contains(directory) {
        println!("just: already on PATH: {}", directory.display());
        return Ok(());
    }
    add_platform(directory)
}

pub fn current_bin_directory() -> ToolResult<PathBuf> {
    let executable = std::env::current_exe()
        .map_err(|error| ToolError::new("just", format!("cannot locate executable: {error}")))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| ToolError::new("just", "cannot locate executable directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_keys_ignore_trailing_separators() {
        assert_eq!(
            key(Path::new("some/folder/")),
            key(Path::new("some/folder"))
        );
    }

    #[test]
    fn shell_paths_are_single_quoted() {
        assert_eq!(shell_quote(Path::new("a b")), "'a b'");
    }

    #[test]
    fn path_separator_is_rejected() {
        let value = if cfg!(windows) {
            Path::new(r"C:\one;C:\two")
        } else {
            Path::new("/one:/two")
        };
        assert!(validate_directory(value).is_err());
    }

    #[test]
    fn bash_prefers_existing_login_profiles() {
        let home = Path::new("/home/test");
        assert_eq!(
            unix_profile(home, "bash", |path| path.ends_with(".bash_profile")),
            home.join(".bash_profile")
        );
        assert_eq!(
            unix_profile(home, "bash", |path| path.ends_with(".bash_login")),
            home.join(".bash_login")
        );
        assert_eq!(unix_profile(home, "bash", |_| false), home.join(".profile"));
    }

    #[test]
    fn managed_block_is_idempotent_and_updatable() {
        let first = managed_profile_contents("existing\n", "export PATH='/one':$PATH")
            .unwrap()
            .unwrap();
        assert!(
            managed_profile_contents(&first, "export PATH='/one':$PATH")
                .unwrap()
                .is_none()
        );
        let updated = managed_profile_contents(&first, "export PATH='/two':$PATH")
            .unwrap()
            .unwrap();
        assert!(updated.contains("'/two'"));
        assert!(!updated.contains("'/one'"));
        assert_eq!(updated.matches(BLOCK_BEGIN).count(), 1);
    }

    #[test]
    fn incomplete_managed_block_is_rejected() {
        assert!(managed_profile_contents(BLOCK_BEGIN, "export PATH='/one':$PATH").is_err());
        assert!(
            managed_profile_contents(
                &format!("{BLOCK_BEGIN}\nexport PATH='/one':$PATH"),
                "export PATH='/one':$PATH"
            )
            .is_err()
        );
        assert!(managed_profile_contents(BLOCK_END, "export PATH='/one':$PATH").is_err());
    }
}
