use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::io::{self, BufWriter, Write};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ::windows::Win32::Foundation::{
    CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, E_UNEXPECTED, LPARAM, POINT, WPARAM,
};
use ::windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
    CoRegisterClassObject, CoRevokeClassObject, CoTaskMemFree, CoUninitialize, IClassFactory,
    IClassFactory_Impl, REGCLS_SINGLEUSE,
};
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::Shell::{
    IExecuteCommand, IExecuteCommand_Impl, IObjectWithSelection, IObjectWithSelection_Impl,
    IShellItemArray, SIGDN_FILESYSPATH,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MSG,
    MessageBoxW, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer, TranslateMessage, WM_APP,
    WM_TIMER,
};
use ::windows::core::{
    BOOL, Error, GUID, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement, w,
};

use crate::i18n::{AppKey, Message};

use super::{SELECTION_PREFIX, SELECTION_SUFFIX, selection_path, wide};

mod registry;

pub use registry::{menu_registered, refresh_menu_language, register_menu, remove_menu};

const RUN_SELECTION: u32 = WM_APP + 2;

pub(super) struct ComApartment;

impl ComApartment {
    pub(super) fn enter() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()? };
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct Server {
    thread: u32,
    idle_from: Cell<Instant>,
    activated: Cell<bool>,
    live_commands: Cell<u32>,
    locks: Cell<u32>,
    pending: RefCell<VecDeque<Pending>>,
}

struct Pending {
    items: IShellItemArray,
    no_ui: bool,
}

#[implement(IClassFactory, Agile = false)]
struct Factory(Rc<Server>);

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        iid: *const GUID,
        output: *mut *mut c_void,
    ) -> Result<()> {
        check_out_args(iid, output)?;
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        self.0.activated.set(true);
        self.0.live_commands.set(self.0.live_commands.get() + 1);
        let command: IExecuteCommand = ExecuteCommand {
            server: self.0.clone(),
            items: RefCell::new(None),
            scheduled: Cell::new(false),
            no_ui: Cell::new(false),
        }
        .into();
        // QueryInterface adds the reference handed back to the COM caller.
        unsafe { command.query(iid, output).ok() }
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        let old = self.0.locks.get();
        if lock.as_bool() {
            self.0.locks.set(old + 1);
        } else if old > 0 {
            self.0.locks.set(old - 1);
            self.0.idle_from.set(Instant::now());
        }
        Ok(())
    }
}

// Keep the shell selection and its proxies on the server's COM apartment.
#[implement(IExecuteCommand, IObjectWithSelection, Agile = false)]
struct ExecuteCommand {
    server: Rc<Server>,
    items: RefCell<Option<IShellItemArray>>,
    scheduled: Cell<bool>,
    no_ui: Cell<bool>,
}

impl IExecuteCommand_Impl for ExecuteCommand_Impl {
    fn SetKeyState(&self, _state: u32) -> Result<()> {
        Ok(())
    }

    fn SetParameters(&self, _parameters: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn SetPosition(&self, _point: &POINT) -> Result<()> {
        Ok(())
    }

    fn SetShowWindow(&self, _show: i32) -> Result<()> {
        Ok(())
    }

    fn SetNoShowUI(&self, disabled: BOOL) -> Result<()> {
        self.no_ui.set(disabled.as_bool());
        Ok(())
    }

    fn SetDirectory(&self, _directory: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn Execute(&self) -> Result<()> {
        if self.scheduled.get() {
            return Err(E_UNEXPECTED.into());
        }
        let items = self.items.borrow().clone().ok_or(E_UNEXPECTED)?;
        // Return from Explorer's COM call before resolving paths and starting the app.
        unsafe { PostThreadMessageW(self.server.thread, RUN_SELECTION, WPARAM(0), LPARAM(0))? };
        self.server.pending.borrow_mut().push_back(Pending {
            items,
            no_ui: self.no_ui.get(),
        });
        self.scheduled.set(true);
        Ok(())
    }
}

impl IObjectWithSelection_Impl for ExecuteCommand_Impl {
    fn SetSelection(&self, items: Ref<'_, IShellItemArray>) -> Result<()> {
        if self.scheduled.get() {
            return Err(E_UNEXPECTED.into());
        }
        self.items.replace(items.cloned());
        Ok(())
    }

    fn GetSelection(&self, iid: *const GUID, output: *mut *mut c_void) -> Result<()> {
        check_out_args(iid, output)?;
        let items = self.items.borrow().clone().ok_or(E_FAIL)?;
        unsafe { items.query(iid, output).ok() }
    }
}

impl Drop for ExecuteCommand {
    fn drop(&mut self) {
        self.server
            .live_commands
            .set(self.server.live_commands.get() - 1);
        self.server.idle_from.set(Instant::now());
    }
}

fn check_out_args(iid: *const GUID, output: *mut *mut c_void) -> Result<()> {
    if output.is_null() {
        return Err(E_POINTER.into());
    }
    // Out storage is supplied by the COM caller and must be null on failure.
    unsafe { output.write(std::ptr::null_mut()) };
    if iid.is_null() {
        return Err(E_POINTER.into());
    }
    Ok(())
}

struct Timer(usize);

impl Drop for Timer {
    fn drop(&mut self) {
        let _ = unsafe { KillTimer(None, self.0) };
    }
}

struct Registration(u32);

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = unsafe { CoRevokeClassObject(self.0) };
    }
}

pub fn run_explorer_server() -> i32 {
    i32::from(serve(&registry::CLASS).is_err())
}

fn serve(clsid: &GUID) -> Result<()> {
    let _apartment = ComApartment::enter()?;
    let server = Rc::new(Server {
        thread: unsafe { GetCurrentThreadId() },
        idle_from: Cell::new(Instant::now()),
        activated: Cell::new(false),
        live_commands: Cell::new(0),
        locks: Cell::new(0),
        pending: RefCell::new(VecDeque::new()),
    });
    let mut message = MSG::default();
    // Force creation of the STA message queue before exposing the class.
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
    let timer = unsafe { SetTimer(None, 0, 1000, None) };
    if timer == 0 {
        return Err(Error::from_win32());
    }
    let _timer = Timer(timer);
    let factory: IClassFactory = Factory(server.clone()).into();
    let _registration = Registration(unsafe {
        CoRegisterClassObject(clsid, &factory, CLSCTX_LOCAL_SERVER, REGCLS_SINGLEUSE)?
    });

    loop {
        match unsafe { GetMessageW(&mut message, None, 0, 0).0 } {
            -1 => return Err(Error::from_win32()),
            0 => return Ok(()),
            _ => {}
        }
        match (message.hwnd.0.is_null(), message.message) {
            (true, RUN_SELECTION) => {
                let pending = server.pending.borrow_mut().pop_front();
                if let Some(pending) = pending {
                    let result = launch_selection(&pending.items);
                    if let Err(error) = result
                        && !pending.no_ui
                    {
                        show_error(error);
                    }
                    server.idle_from.set(Instant::now());
                }
            }
            (true, WM_TIMER) if message.wParam.0 == timer => {
                let idle_limit = if server.activated.get() { 5 } else { 30 };
                if server.live_commands.get() == 0
                    && server.locks.get() == 0
                    && server.pending.borrow().is_empty()
                    && server.idle_from.get().elapsed() >= Duration::from_secs(idle_limit)
                {
                    return Ok(());
                }
            }
            _ => unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
}

fn show_error(error: io::Error) {
    let description = Message::from(AppKey::ExplorerLaunchFailed)
        .arg(Message::from_error(error))
        .to_string();
    if let Ok(message) = wide(description.as_ref()) {
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(message.as_ptr()),
                w!("NestedUnpack"),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }
}

struct ShellString(PWSTR);

impl Drop for ShellString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
        }
    }
}

fn write_selection(output: &mut impl Write, items: &IShellItemArray) -> io::Result<()> {
    let count = unsafe { items.GetCount() }.map_err(io::Error::other)?;
    if count == 0 {
        return Err(Message::from(AppKey::NoUsableSelection).io(io::ErrorKind::InvalidInput));
    }
    for index in 0..count {
        let item = unsafe { items.GetItemAt(index) }.map_err(io::Error::other)?;
        let name = ShellString(
            unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(io::Error::other)?,
        );
        if name.0.is_null() {
            return Err(
                Message::from(AppKey::InvalidExplorerFilePath).io(io::ErrorKind::InvalidData)
            );
        }
        let path = selection_path(unsafe { name.0.as_wide() });
        let name = path
            .to_str()
            .filter(|name| path.is_absolute() && !name.contains(['\r', '\n']));
        let name = name.ok_or_else(|| {
            Message::from(AppKey::InvalidExplorerFilePath).io(io::ErrorKind::InvalidData)
        })?;
        writeln!(output, "{name}")?;
    }
    Ok(())
}

fn launch_selection(items: &IShellItemArray) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut manifest = tempfile::Builder::new()
        .prefix(SELECTION_PREFIX)
        .suffix(SELECTION_SUFFIX)
        .tempfile_in(std::env::temp_dir())?;
    {
        let mut output = BufWriter::new(manifest.as_file_mut());
        write_selection(&mut output, items)?;
        output.flush()?;
    }
    let manifest = manifest
        .into_temp_path()
        .keep()
        .map_err(|error| error.error)?;
    match Command::new(exe)
        .arg("--nested-selection-file")
        .arg(&manifest)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(manifest);
            Err(error)
        }
    }
}
