//! Windows-specific bits: hidden child processes, killing a whole process
//! tree, Explorer, keeping the PC awake, taskbar progress and window focus.

use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicIsize, Ordering};

/// The main window, once eframe has created it.
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn set_main_window(hwnd: isize) {
    MAIN_HWND.store(hwnd, Ordering::SeqCst);
}

pub fn main_window() -> isize {
    MAIN_HWND.load(Ordering::SeqCst)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;

    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
        TerminateJobObject,
    };
    use windows::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
    use windows::Win32::UI::Shell::{
        ITaskbarList3, ShellExecuteW, TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
        TaskbarList,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx, IsIconic, MB_ICONASTERISK,
        SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
    };
    use windows::core::PCWSTR;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn hide_window(cmd: &mut Command) {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    /// A Windows job object. Every process inside it (yt-dlp and the ffmpeg
    /// it starts) dies together on `kill`, or when Snag itself exits.
    pub struct ProcGroup(isize);

    impl ProcGroup {
        pub fn new() -> Option<ProcGroup> {
            unsafe {
                let h = CreateJobObjectW(None, PCWSTR::null()).ok()?;
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let _ = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                Some(ProcGroup(h.0 as isize))
            }
        }

        pub fn assign(&self, child: &Child) -> bool {
            unsafe { AssignProcessToJobObject(HANDLE(self.0 as _), HANDLE(child.as_raw_handle() as _)).is_ok() }
        }

        pub fn kill(&self) {
            unsafe {
                let _ = TerminateJobObject(HANDLE(self.0 as _), 1);
            }
        }
    }

    impl Drop for ProcGroup {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(HANDLE(self.0 as _));
            }
        }
    }

    pub fn shell_open(target: &OsStr) -> bool {
        let file = wide(target);
        let verb = wide(OsStr::new("open"));
        let r = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        r.0 as isize > 32
    }

    pub fn reveal(path: &Path) {
        if path.is_file() {
            // explorer wants exactly: /select,"C:\path\file.mp4"
            let _ = Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
        } else {
            shell_open(path.as_os_str());
        }
    }

    pub fn hide_path(path: &Path) {
        use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
        let w = wide(path.as_os_str());
        unsafe {
            let _ = SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_ATTRIBUTE_HIDDEN);
        }
    }

    pub fn keep_awake(on: bool) {
        unsafe {
            if on {
                SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
            } else {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        }
    }

    pub fn free_space(path: &Path) -> Option<u64> {
        let mut p = path;
        while !p.exists() {
            p = p.parent()?;
        }
        let w = wide(p.as_os_str());
        let mut free = 0u64;
        unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), None, None).ok()? };
        Some(free)
    }

    pub fn clipboard_seq() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    pub fn allow_foreground(pid: u32) {
        unsafe {
            let _ = AllowSetForegroundWindow(pid);
        }
    }

    pub fn bring_to_front() {
        let h = main_window();
        if h == 0 {
            return;
        }
        unsafe {
            let hwnd = HWND(h as _);
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }

    /// Flashes the taskbar button (until the window is focused) and plays the
    /// system "asterisk" sound.
    pub fn notify_attention(sound: bool) {
        let h = main_window();
        unsafe {
            if h != 0 {
                let info = FLASHWINFO {
                    cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                    hwnd: HWND(h as _),
                    dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
                    uCount: 3,
                    dwTimeout: 0,
                };
                let _ = FlashWindowEx(&info);
            }
            if sound {
                let _ = MessageBeep(MB_ICONASTERISK);
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    pub enum Bar {
        Off,
        Normal(f32),
        Busy,
        Paused(f32),
        Error(f32),
    }

    /// The progress shown on Snag's taskbar button. Lives on one thread.
    pub struct Taskbar {
        list: Option<ITaskbarList3>,
        last: Option<Bar>,
    }

    impl Taskbar {
        pub fn new() -> Taskbar {
            let list = unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                    .ok()
                    .filter(|l| l.HrInit().is_ok())
            };
            Taskbar { list, last: None }
        }

        pub fn set(&mut self, bar: Bar) {
            let h = main_window();
            let Some(list) = &self.list else { return };
            if h == 0 || self.last == Some(bar) {
                return;
            }
            let hwnd = HWND(h as _);
            let value = |f: f32| ((f.clamp(0.0, 1.0) * 1000.0) as u64, 1000u64);
            unsafe {
                let _ = match bar {
                    Bar::Off => list.SetProgressState(hwnd, TBPF_NOPROGRESS),
                    Bar::Busy => list.SetProgressState(hwnd, TBPF_INDETERMINATE),
                    Bar::Normal(f) => {
                        let _ = list.SetProgressState(hwnd, TBPF_NORMAL);
                        let (a, b) = value(f);
                        list.SetProgressValue(hwnd, a, b)
                    }
                    Bar::Paused(f) => {
                        let _ = list.SetProgressState(hwnd, TBPF_PAUSED);
                        let (a, b) = value(f);
                        list.SetProgressValue(hwnd, a, b)
                    }
                    Bar::Error(f) => {
                        let _ = list.SetProgressState(hwnd, TBPF_ERROR);
                        let (a, b) = value(f.max(0.05));
                        list.SetProgressValue(hwnd, a, b)
                    }
                };
            }
            self.last = Some(bar);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    //! Enough for the app to build elsewhere; Snag targets Windows.
    use super::*;
    use std::ffi::OsStr;

    pub fn hide_window(_cmd: &mut Command) {}

    pub struct ProcGroup(std::sync::Mutex<Option<u32>>);

    impl ProcGroup {
        pub fn new() -> Option<ProcGroup> {
            Some(ProcGroup(std::sync::Mutex::new(None)))
        }
        pub fn assign(&self, child: &Child) -> bool {
            *self.0.lock().unwrap() = Some(child.id());
            true
        }
        pub fn kill(&self) {
            if let Some(pid) = *self.0.lock().unwrap() {
                let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
            }
        }
    }

    pub fn shell_open(target: &OsStr) -> bool {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        Command::new(opener).arg(target).spawn().is_ok()
    }

    pub fn reveal(path: &Path) {
        let dir = if path.is_file() { path.parent().unwrap_or(path) } else { path };
        shell_open(dir.as_os_str());
    }

    pub fn keep_awake(_on: bool) {}
    pub fn hide_path(_path: &Path) {}
    pub fn free_space(_path: &Path) -> Option<u64> {
        None
    }
    pub fn clipboard_seq() -> u32 {
        0
    }
    pub fn allow_foreground(_pid: u32) {}
    pub fn bring_to_front() {}
    pub fn notify_attention(_sound: bool) {}

    #[derive(Clone, Copy, PartialEq)]
    pub enum Bar {
        Off,
        Normal(f32),
        Busy,
        Paused(f32),
        Error(f32),
    }

    pub struct Taskbar;
    impl Taskbar {
        pub fn new() -> Taskbar {
            Taskbar
        }
        pub fn set(&mut self, _bar: Bar) {}
    }
}

pub use imp::*;

pub fn open_path(path: &Path) -> bool {
    shell_open(path.as_os_str())
}

pub fn open_url(url: &str) -> bool {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    shell_open(std::ffi::OsStr::new(url))
}
