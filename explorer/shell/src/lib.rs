//! The JustTools entry in the Windows File Explorer context menu.
//!
//! Explorer loads this library, asks it what to show for the selection, and on
//! a click it starts `just.exe context run <action>` in a new console window.
//! Everything about *what* the menu contains lives in `justtools-menu`; this
//! file only translates that model to `IExplorerCommand`.
//!
//! The library runs inside Explorer (or its COM surrogate), and the workspace
//! builds with `panic = "abort"`, so nothing here may panic: no unwrapping, no
//! indexing, and no `RefCell` borrow held across a call back into COM.

#![cfg(windows)]
#![allow(non_snake_case)]

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use justtools_menu::{self as menu, Icon, Item, Kind, Selection, Tool};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, CloseHandle, E_FAIL, E_NOTIMPL, E_POINTER,
    HINSTANCE, HMODULE, MAX_PATH, S_FALSE, S_OK,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Com::{
    CoTaskMemFree, DVASPECT_CONTENT, FORMATETC, IBindCtx, IClassFactory, IClassFactory_Impl,
    IDataObject, IServiceProvider, TYMED_HGLOBAL,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Ole::{
    CF_HDROP, IObjectWithSite, IObjectWithSite_Impl, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetCurrentProcessId,
    PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::Shell::{
    BHID_DataObject, DragQueryFileW, ECF_DEFAULT, ECF_HASSUBCOMMANDS, ECF_ISSEPARATOR, ECS_ENABLED,
    ECS_HIDDEN, HDROP, IEnumExplorerCommand, IEnumExplorerCommand_Impl, IExplorerCommand,
    IExplorerCommand_Impl, IFolderView, IShellItem, IShellItemArray, SHStrDupW, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
use windows::core::{
    BOOL, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement, w,
};

const CLASS_ID: GUID = GUID::from_u128(menu::CLASS_ID_VALUE);
/// `SID_SFolderView`, the service that yields the folder a view is showing.
const FOLDER_VIEW_SERVICE: GUID = GUID::from_u128(0xcde725b0_ccc9_4519_917e_325d72fab4ce);
/// A stat call per item is fine for a handful of unknown names and far too
/// slow for a selection of thousands, so only the first few are probed.
const FOLDER_PROBES: usize = 16;

static MODULE: AtomicUsize = AtomicUsize::new(0);
static OBJECTS: AtomicUsize = AtomicUsize::new(0);
static LAUNCHES: AtomicUsize = AtomicUsize::new(0);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn module_path() -> Option<PathBuf> {
    let module = HMODULE(MODULE.load(Ordering::Relaxed) as *mut c_void);
    let mut buffer = vec![0_u16; MAX_PATH as usize];
    loop {
        let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
        if length == 0 {
            return None;
        }
        if length < buffer.len() {
            return Some(PathBuf::from(String::from_utf16_lossy(
                buffer.get(..length)?,
            )));
        }
        if buffer.len() > 1 << 16 {
            return None;
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

fn is_directory(path: &str) -> bool {
    let attributes = unsafe { GetFileAttributesW(&HSTRING::from(path)) };
    attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
}

fn exists(path: &str) -> bool {
    unsafe { GetFileAttributesW(&HSTRING::from(path)) != INVALID_FILE_ATTRIBUTES }
}

/// Reduce the selected paths to what the menu depends on.
fn summarize(paths: &[String]) -> Selection {
    let mut selection = Selection::default();
    let mut probes = 0;
    for path in paths {
        let recognized = menu::extension(path)
            .is_some_and(|extension| Tool::ALL.iter().any(|tool| tool.accepts(&extension)));
        if !recognized && probes < FOLDER_PROBES {
            probes += 1;
            if is_directory(path) {
                selection.add_folder(exists(&format!("{path}\\.git")));
                continue;
            }
        }
        selection.add_file(path);
    }
    selection
}

/// Every selected path in one cross-process call, when Explorer can provide a
/// drop list for the selection.
unsafe fn drop_list_paths(items: &IShellItemArray) -> Vec<String> {
    let mut paths = Vec::new();
    let Ok(data) = (unsafe { items.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) })
    else {
        return paths;
    };
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = (unsafe { data.GetData(&format) }) else {
        return paths;
    };
    let drop = HDROP(unsafe { medium.u.hGlobal }.0);
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, None) } as usize;
        let mut buffer = vec![0_u16; length + 1];
        let written = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) } as usize;
        if let Some(text) = buffer.get(..written)
            && written > 0
        {
            paths.push(String::from_utf16_lossy(text));
        }
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}

unsafe fn item_path(item: &IShellItem) -> Option<String> {
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?;
    let path = unsafe { name.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(name.0 as *const c_void)) };
    path.filter(|path| !path.is_empty())
}

unsafe fn selected_paths(items: &IShellItemArray) -> Vec<String> {
    let paths = unsafe { drop_list_paths(items) };
    if !paths.is_empty() {
        return paths;
    }
    let count = unsafe { items.GetCount() }.unwrap_or(0);
    (0..count)
        .filter_map(|index| unsafe { items.GetItemAt(index) }.ok())
        .filter_map(|item| unsafe { item_path(&item) })
        .collect()
}

/// The folder whose empty background was right-clicked.
unsafe fn site_folder(site: &IUnknown) -> Option<String> {
    let provider: IServiceProvider = site.cast().ok()?;
    let view: IFolderView = unsafe { provider.QueryService(&FOLDER_VIEW_SERVICE) }.ok()?;
    let folder: IShellItem = unsafe { view.GetFolder() }.ok()?;
    unsafe { item_path(&folder) }
}

/// State shared by the root command and the entries it hands out.
#[derive(Default)]
struct Shared {
    /// The menu resolved for the current selection.
    items: RefCell<Vec<Item>>,
    paths: RefCell<Vec<String>>,
    site: RefCell<Option<IUnknown>>,
}

impl Shared {
    fn selection(&self, items: Option<&IShellItemArray>) -> Vec<String> {
        let mut paths = items
            .map(|items| unsafe { selected_paths(items) })
            .unwrap_or_default();
        if paths.is_empty() {
            let site = self.site.try_borrow().ok().and_then(|site| site.clone());
            if let Some(folder) = site.and_then(|site| unsafe { site_folder(&site) }) {
                paths.push(folder);
            }
        }
        paths
    }

    /// Resolve and remember the menu for a fresh selection.
    fn refresh(&self, items: Option<&IShellItemArray>) -> bool {
        let paths = self.selection(items);
        let resolved = menu::menu(&summarize(&paths));
        let visible = !resolved.is_empty();
        if let Ok(mut slot) = self.items.try_borrow_mut() {
            *slot = resolved;
        }
        if let Ok(mut slot) = self.paths.try_borrow_mut() {
            *slot = paths;
        }
        visible
    }
}

/// One menu entry. `item` is `None` for the top-level JustTools entry.
#[implement(IExplorerCommand, IObjectWithSite)]
struct Command {
    item: Option<Item>,
    shared: Rc<Shared>,
}

impl Command {
    fn create(item: Option<Item>, shared: Rc<Shared>) -> IExplorerCommand {
        OBJECTS.fetch_add(1, Ordering::Relaxed);
        Command { item, shared }.into()
    }

    /// Only the top-level entry has a flyout; its rows are the resolved menu.
    fn children(&self) -> Vec<Item> {
        if self.item.is_some() {
            return Vec::new();
        }
        self.shared
            .items
            .try_borrow()
            .map(|items| items.clone())
            .unwrap_or_default()
    }
}

impl Drop for Command {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl IExplorerCommand_Impl for Command_Impl {
    fn GetTitle(&self, _items: Ref<'_, IShellItemArray>) -> Result<PWSTR> {
        let title = self
            .item
            .as_ref()
            .map_or(menu::ROOT_TITLE, |item| item.title.as_str());
        unsafe { SHStrDupW(&HSTRING::from(title)) }
    }

    fn GetIcon(&self, _items: Ref<'_, IShellItemArray>) -> Result<PWSTR> {
        let icon = self.item.as_ref().map_or(Icon::Brand, |item| item.icon);
        if icon == Icon::None {
            return Err(E_NOTIMPL.into());
        }
        let library = module_path().ok_or(E_FAIL)?;
        let location = format!("{},-{}", library.display(), icon.resource_id());
        unsafe { SHStrDupW(&HSTRING::from(location)) }
    }

    fn GetToolTip(&self, _items: Ref<'_, IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(if self.item.is_none() {
            CLASS_ID
        } else {
            GUID::zeroed()
        })
    }

    fn GetState(&self, items: Ref<'_, IShellItemArray>, _slow: BOOL) -> Result<u32> {
        if self.item.is_some() || self.shared.refresh(items.as_ref()) {
            Ok(ECS_ENABLED.0 as u32)
        } else {
            Ok(ECS_HIDDEN.0 as u32)
        }
    }

    fn Invoke(&self, items: Ref<'_, IShellItemArray>, _context: Ref<'_, IBindCtx>) -> Result<()> {
        let Some(Item {
            kind: Kind::Action(id),
            ..
        }) = &self.item
        else {
            return Ok(());
        };
        let mut paths = self.shared.selection(items.as_ref());
        if paths.is_empty() {
            paths = self
                .shared
                .paths
                .try_borrow()
                .map(|paths| paths.clone())
                .unwrap_or_default();
        }
        if let Err(message) = launch(id, &paths) {
            unsafe {
                MessageBoxW(
                    None,
                    &HSTRING::from(message),
                    w!("JustTools"),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        let flags = match self.item.as_ref().map(|item| &item.kind) {
            None => ECF_HASSUBCOMMANDS,
            Some(Kind::Separator) => ECF_ISSEPARATOR,
            Some(Kind::Action(_)) => ECF_DEFAULT,
        };
        Ok(flags.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Ok(Commands::create(self.children(), 0, self.shared.clone()))
    }
}

impl IObjectWithSite_Impl for Command_Impl {
    fn SetSite(&self, site: Ref<'_, IUnknown>) -> Result<()> {
        if let Ok(mut slot) = self.shared.site.try_borrow_mut() {
            *slot = site.as_ref().cloned();
        }
        Ok(())
    }

    fn GetSite(&self, interface: *const GUID, result: *mut *mut c_void) -> Result<()> {
        if result.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *result = std::ptr::null_mut() };
        let site = self
            .shared
            .site
            .try_borrow()
            .ok()
            .and_then(|site| site.clone())
            .ok_or(E_FAIL)?;
        unsafe { site.query(interface, result).ok() }
    }
}

#[implement(IEnumExplorerCommand)]
struct Commands {
    items: Vec<Item>,
    next: Cell<usize>,
    shared: Rc<Shared>,
}

impl Commands {
    fn create(items: Vec<Item>, next: usize, shared: Rc<Shared>) -> IEnumExplorerCommand {
        OBJECTS.fetch_add(1, Ordering::Relaxed);
        Commands {
            items,
            next: Cell::new(next),
            shared,
        }
        .into()
    }
}

impl Drop for Commands {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl IEnumExplorerCommand_Impl for Commands_Impl {
    fn Next(
        &self,
        count: u32,
        commands: *mut Option<IExplorerCommand>,
        fetched: *mut u32,
    ) -> HRESULT {
        if commands.is_null() {
            return E_POINTER;
        }
        let mut written = 0_u32;
        while written < count {
            let Some(item) = self.items.get(self.next.get()) else {
                break;
            };
            let command = Command::create(Some(item.clone()), self.shared.clone());
            // The caller's buffer is uninitialized, so it must not be dropped.
            unsafe { commands.add(written as usize).write(Some(command)) };
            self.next.set(self.next.get() + 1);
            written += 1;
        }
        if !fetched.is_null() {
            unsafe { *fetched = written };
        }
        if written == count { S_OK } else { S_FALSE }
    }

    fn Skip(&self, count: u32) -> Result<()> {
        let next = self.next.get().saturating_add(count as usize);
        self.next.set(next.min(self.items.len()));
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        self.next.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        Ok(Commands::create(
            self.items.clone(),
            self.next.get(),
            self.shared.clone(),
        ))
    }
}

/// Start `just.exe context run <id>` in its own console window.
///
/// The selection travels in a temporary list file: a command line holds
/// about 32,000 characters and a selection can hold far more than that.
fn launch(id: &str, paths: &[String]) -> std::result::Result<(), String> {
    let library = module_path().ok_or("JustTools could not locate its own files.")?;
    let executable = library.parent().unwrap_or(Path::new(".")).join("just.exe");
    if !executable.is_file() {
        return Err(format!(
            "JustTools is not installed beside its context menu.\n\nMissing: {}\n\nRun “just install” and then “just context install”.",
            executable.display()
        ));
    }
    let list = std::env::temp_dir().join(format!(
        "justtools-menu-{}-{}.txt",
        unsafe { GetCurrentProcessId() },
        LAUNCHES.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&list, paths.join("\n"))
        .map_err(|error| format!("JustTools could not pass on the selection: {error}"))?;
    let mut command = wide(&format!(
        "\"{}\" context run {id} --list \"{}\"",
        executable.display(),
        list.display()
    ));
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    let started = unsafe {
        CreateProcessW(
            &HSTRING::from(executable.as_os_str()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR::null(),
            &startup,
            &mut process,
        )
    };
    match started {
        Ok(()) => {
            unsafe {
                CloseHandle(process.hThread).ok();
                CloseHandle(process.hProcess).ok();
            }
            Ok(())
        }
        Err(error) => {
            std::fs::remove_file(&list).ok();
            Err(format!("JustTools could not start: {}", error.message()))
        }
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        interface: *const GUID,
        result: *mut *mut c_void,
    ) -> Result<()> {
        if result.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *result = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let command = Command::create(None, Rc::new(Shared::default()));
        unsafe { command.query(interface, result).ok() }
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            OBJECTS.fetch_add(1, Ordering::Relaxed);
        } else {
            OBJECTS.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(instance: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(instance.0 as usize, Ordering::Relaxed);
    }
    true.into()
}

/// # Safety
/// Called by COM with valid pointers for the class, interface, and result.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    class: *const GUID,
    interface: *const GUID,
    result: *mut *mut c_void,
) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    unsafe { *result = std::ptr::null_mut() };
    if class.is_null() || interface.is_null() || unsafe { *class } != CLASS_ID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    unsafe { factory.query(interface, result) }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::Relaxed) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
