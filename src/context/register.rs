//! Adding and removing the File Explorer entry on Windows.
//!
//! Two registrations exist, and at most one is active:
//!
//! * **Package** — a sparse package identity, which is the only way into the
//!   top-level Windows 11 menu. It needs either a signed `justtools-shell.msix`
//!   beside `just.exe` or Developer Mode for an unsigned registration.
//! * **Classic** — per-user registry verbs, which need nothing but appear
//!   under "Show more options" on Windows 11.
//!
//! Both are per-user, need no elevation, and are fully undone by `uninstall`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use justtools_menu as menu;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use super::package;
use crate::error::{ToolError, ToolResult};

const VERB: &str = "JustTools";
const CLASSES: &str = r"Software\Classes";
/// The signed package a release ships beside `just.exe`.
const SIGNED_PACKAGE: &str = "justtools-shell.msix";
/// Where an unsigned Developer Mode registration keeps its manifest.
const LAYOUT_DIRECTORY: &str = "context-menu";
const WINDOWS_11_BUILD: u32 = 22_000;
/// Where the folder that owns the registration is remembered, so another
/// copy of JustTools never refreshes or removes a menu it did not install.
const STATE_KEY: &str = r"Software\JustGains\JustTools";
const STATE_VALUE: &str = "ContextMenuDirectory";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tier {
    Package,
    Classic,
}

pub struct Status {
    /// The registered package version, when the package is present.
    pub package: Option<String>,
    pub classic: bool,
    /// The JustTools folder the active registration points at.
    pub owner: Option<PathBuf>,
    /// Whether that folder is the one this executable runs from.
    pub owned: bool,
    pub library: PathBuf,
    pub windows_11: bool,
    pub developer_mode: bool,
    pub signed_package: bool,
}

fn error(message: impl Into<String>) -> ToolError {
    ToolError::new("just", message)
}

fn class_key() -> String {
    format!(r"{CLASSES}\CLSID\{{{}}}", menu::CLASS_ID)
}

fn verb_parent(item_type: &str) -> String {
    if item_type.starts_with('.') {
        format!(r"{CLASSES}\SystemFileAssociations\{item_type}\shell")
    } else {
        format!(r"{CLASSES}\{item_type}\shell")
    }
}

fn bin_directory() -> ToolResult<PathBuf> {
    let executable = std::env::current_exe()
        .map_err(|error| self::error(format!("cannot locate just.exe: {error}")))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| error("cannot locate the JustTools folder"))
}

fn windows_build() -> u32 {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|key| key.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|build| build.parse().ok())
        .unwrap_or(0)
}

fn developer_mode() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock")
        .and_then(|key| key.get_value::<u32, _>("AllowDevelopmentWithoutDevLicense"))
        .is_ok_and(|value| value == 1)
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

/// Run a Windows PowerShell script, returning its trimmed output or its error.
///
/// Windows PowerShell 5.1 ships with every supported Windows and carries the
/// Appx cmdlets; PowerShell 7 only reaches them through a compatibility shim.
fn powershell(script: &str) -> Result<String, String> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let shell = Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let output = Command::new(shell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
        ])
        .arg(format!(
            "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; {script}"
        ))
        .output()
        .map_err(|error| format!("cannot run Windows PowerShell: {error}"))?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
    if output.status.success() {
        Ok(text(&output.stdout))
    } else {
        let message = text(&output.stderr);
        // The first line names the failure; the rest is PowerShell's position
        // and category detail.
        Err(message
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("Windows PowerShell reported a failure")
            .to_owned())
    }
}

fn package_version() -> Option<String> {
    powershell(&format!(
        "(Get-AppxPackage -Name '{}').Version",
        package::NAME
    ))
    .ok()
    .filter(|version| !version.is_empty())
}

fn classic_registered() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(class_key())
        .is_ok()
}

fn owner() -> Option<PathBuf> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(STATE_KEY)
        .and_then(|key| key.get_value::<String, _>(STATE_VALUE))
        .ok()
        .filter(|directory| !directory.is_empty())
        .map(PathBuf::from)
}

fn remember_owner(bin: &Path) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(STATE_KEY)?;
    key.set_value(STATE_VALUE, &bin.to_string_lossy().as_ref())
}

fn forget_owner() -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(STATE_KEY, winreg::enums::KEY_SET_VALUE) {
        ignore_missing(key.delete_value(STATE_VALUE))?;
    }
    Ok(())
}

/// Whether the registration, if any, belongs to this copy of JustTools.
/// `None` means this user has no JustTools menu at all. Unlike [`status`],
/// this reads only the registry, so it is cheap enough for every install.
pub fn ownership() -> ToolResult<Option<bool>> {
    let bin = bin_directory()?;
    Ok(owner().map(|owner| crate::common::same_path(&owner, &bin)))
}

pub fn library_present() -> bool {
    bin_directory().is_ok_and(|bin| bin.join(menu::SHELL_LIBRARY).is_file())
}

pub fn status() -> ToolResult<Status> {
    let bin = bin_directory()?;
    let owner = owner();
    Ok(Status {
        package: package_version(),
        classic: classic_registered(),
        owned: owner
            .as_deref()
            .is_some_and(|owner| crate::common::same_path(owner, &bin)),
        owner,
        library: bin.join(menu::SHELL_LIBRARY),
        windows_11: windows_build() >= WINDOWS_11_BUILD,
        developer_mode: developer_mode(),
        signed_package: bin.join(SIGNED_PACKAGE).is_file(),
    })
}

fn notify_shell() {
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    // Explorer caches verbs per file type until it is told they changed.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

fn remove_package() -> Result<(), String> {
    powershell(&format!(
        "Get-AppxPackage -Name '{}' | Remove-AppxPackage",
        package::NAME
    ))
    .map(drop)
}

fn register_package(bin: &Path) -> Result<(), String> {
    // A signed package and a Developer Mode registration cannot replace each
    // other in place, so any earlier registration is removed first.
    remove_package()?;
    let signed = bin.join(SIGNED_PACKAGE);
    if signed.is_file() {
        return powershell(&format!(
            "Add-AppxPackage -Path {} -ExternalLocation {}",
            quote(&signed),
            quote(bin)
        ))
        .map(drop);
    }
    let layout = bin.join(LAYOUT_DIRECTORY);
    package::write_layout(&layout, package::DEFAULT_PUBLISHER)
        .map_err(|error| error.message().to_owned())?;
    powershell(&format!(
        "Add-AppxPackage -Register {} -ExternalLocation {}",
        quote(&layout.join(package::MANIFEST)),
        quote(bin)
    ))
    .map(drop)
}

fn register_classic(library: &Path) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let class_id = format!("{{{}}}", menu::CLASS_ID);
    let (class, _) = hkcu.create_subkey(class_key())?;
    class.set_value("", &"JustTools context menu")?;
    let (server, _) = class.create_subkey("InprocServer32")?;
    server.set_value("", &library.to_string_lossy().as_ref())?;
    server.set_value("ThreadingModel", &"Apartment")?;
    let icon = format!("{},-{}", library.display(), menu::Icon::Brand.resource_id());
    for item_type in package::item_types() {
        let (verb, _) = hkcu.create_subkey(format!(r"{}\{VERB}", verb_parent(&item_type)))?;
        verb.set_value("", &menu::ROOT_TITLE)?;
        verb.set_value("MUIVerb", &menu::ROOT_TITLE)?;
        verb.set_value("Icon", &icon)?;
        verb.set_value("ExplorerCommandHandler", &class_id)?;
    }
    Ok(())
}

fn ignore_missing(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Remove one verb and any parent keys it was the only reason for.
fn remove_verb(hkcu: &RegKey, parent: &str) -> std::io::Result<()> {
    ignore_missing(hkcu.delete_subkey_all(format!(r"{parent}\{VERB}")))?;
    // Only keys left with nothing in them go, so other software's verbs and
    // values stay exactly where they were.
    let unused = |key: &str| {
        hkcu.open_subkey(key)
            .and_then(|key| key.query_info())
            .is_ok_and(|info| info.sub_keys == 0 && info.values == 0)
    };
    let mut key = parent;
    while key.len() > CLASSES.len() && unused(key) && hkcu.delete_subkey(key).is_ok() {
        match key.rsplit_once('\\') {
            Some((up, _)) if !up.ends_with("SystemFileAssociations") => key = up,
            _ => break,
        }
    }
    Ok(())
}

fn remove_classic() -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut parents: Vec<String> = package::item_types()
        .iter()
        .map(|item_type| verb_parent(item_type))
        .collect();
    // An older build may have registered types this one no longer lists.
    let associations = format!(r"{CLASSES}\SystemFileAssociations");
    if let Ok(key) = hkcu.open_subkey(&associations) {
        parents.extend(
            key.enum_keys()
                .filter_map(Result::ok)
                .map(|name| format!(r"{associations}\{name}\shell")),
        );
    }
    parents.sort_unstable();
    parents.dedup();
    let class_id = format!("{{{}}}", menu::CLASS_ID);
    for parent in parents {
        let ours = hkcu
            .open_subkey(format!(r"{parent}\{VERB}"))
            .and_then(|verb| verb.get_value::<String, _>("ExplorerCommandHandler"))
            .is_ok_and(|handler| handler.eq_ignore_ascii_case(&class_id));
        if ours {
            remove_verb(&hkcu, &parent)?;
        }
    }
    ignore_missing(hkcu.delete_subkey_all(class_key()))
}

/// Register the menu and report which tier ended up active, with the reason
/// the package tier was not used when it was not.
pub fn install(classic_only: bool) -> ToolResult<(Tier, Option<String>)> {
    let bin = bin_directory()?;
    let library = bin.join(menu::SHELL_LIBRARY);
    if !library.is_file() {
        return Err(error(format!(
            "{} is not beside just.exe in {}; it ships in the Windows release archive and is copied by `just install`",
            menu::SHELL_LIBRARY,
            bin.display()
        )));
    }
    let registry =
        |error: std::io::Error| self::error(format!("cannot update the registry: {error}"));
    let reason = if classic_only {
        None
    } else if windows_build() < WINDOWS_11_BUILD {
        Some("this is Windows 10, which has a single context menu".to_owned())
    } else if !bin.join(SIGNED_PACKAGE).is_file() && !developer_mode() {
        Some(format!(
            "the top-level Windows 11 menu needs a signed {SIGNED_PACKAGE} beside just.exe or Developer Mode"
        ))
    } else {
        match register_package(&bin) {
            Ok(()) => {
                // Windows also lists packaged verbs under "Show more
                // options", so the classic verbs would only duplicate them.
                remove_classic().map_err(registry)?;
                remember_owner(&bin).map_err(registry)?;
                notify_shell();
                return Ok((Tier::Package, None));
            }
            Err(message) => Some(format!("Windows declined the package: {message}")),
        }
    };
    if classic_only {
        remove_package().map_err(error)?;
    }
    remove_classic().map_err(registry)?;
    register_classic(&library).map_err(registry)?;
    remember_owner(&bin).map_err(registry)?;
    notify_shell();
    Ok((Tier::Classic, reason))
}

pub fn uninstall() -> ToolResult {
    let bin = bin_directory()?;
    remove_package().map_err(|message| error(format!("cannot remove the package: {message}")))?;
    remove_classic()
        .and_then(|()| forget_owner())
        .map_err(|error| self::error(format!("cannot update the registry: {error}")))?;
    let layout = bin.join(LAYOUT_DIRECTORY);
    if layout.join(package::MANIFEST).is_file() {
        std::fs::remove_dir_all(&layout)
            .map_err(|error| self::error(format!("cannot remove {}: {error}", layout.display())))?;
    }
    notify_shell();
    Ok(())
}
