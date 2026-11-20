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

    /// Sleep (not hibernate), like Start → Power → Sleep.
    pub fn sleep_pc() {
        unsafe {
            let _ = windows::Win32::System::Power::SetSuspendState(false, false, false);
        }
    }

    pub fn shut_down() {
        let mut cmd = Command::new("shutdown");
        cmd.args(["/s", "/t", "0"]);
        hide_window(&mut cmd);
        let _ = cmd.spawn();
    }

    const PROTOCOL_KEY: &str = r"Software\Classes\snag";

    fn reg_set(subkey: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
        let key = wide(OsStr::new(subkey));
        let name_w = name.map(|n| wide(OsStr::new(n)));
        let data = wide(OsStr::new(value));
        let err = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                name_w.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
                REG_SZ.0,
                Some(data.as_ptr() as *const core::ffi::c_void),
                (data.len() * 2) as u32,
            )
        };
        if err.0 == 0 { Ok(()) } else { Err(format!("Windows refused (error {})", err.0)) }
    }

    /// Makes `snag:<link>` addresses (the browser button) open this Snagger.exe.
    /// Per-user (HKEY_CURRENT_USER); no admin rights needed.
    pub fn register_protocol(exe: &Path) -> Result<(), String> {
        let exe = exe.display().to_string();
        reg_set(PROTOCOL_KEY, None, "URL:Snagger")?;
        reg_set(PROTOCOL_KEY, Some("URL Protocol"), "")?;
        reg_set(&format!(r"{PROTOCOL_KEY}\DefaultIcon"), None, &format!("\"{exe}\",0"))?;
        reg_set(&format!(r"{PROTOCOL_KEY}\shell\open\command"), None, &format!("\"{exe}\" \"%1\""))
    }

    pub fn unregister_protocol() {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
        let key = wide(OsStr::new(PROTOCOL_KEY));
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()));
        }
    }

    /// The command `snag:` links run, if any.
    pub fn protocol_command() -> Option<String> {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
        let key = wide(OsStr::new(&format!(r"{PROTOCOL_KEY}\shell\open\command")));
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let err = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR::null(),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                Some(&mut len),
            )
        };
        if err.0 != 0 {
            return None;
        }
        let chars = (len as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..chars.min(buf.len())]))
    }

    pub fn hide_path(path: &Path) {
        use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
        let w = wide(path.as_os_str());
        unsafe {
            let _ = SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_ATTRIBUTE_HIDDEN);
        }
    }

    /// True when Windows "Animation effects" (Show animations) is off.
    pub fn reduced_motion() -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        let mut on = windows::core::BOOL(1);
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some(&mut on as *mut _ as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        ok.is_ok() && !on.as_bool()
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
            if !windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
            }
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }

    /// Hides the window (Snagger keeps running in the tray).
    pub fn hide_main_window() {
        let h = main_window();
        if h != 0 {
            unsafe {
                let _ = ShowWindow(HWND(h as _), windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            }
        }
    }

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_NAME: &str = "Snagger";

    /// Starts Snagger (in the tray) when you sign in to Windows, or stops it.
    pub fn set_run_at_startup(on: bool, exe: &Path) -> Result<(), String> {
        if on {
            reg_set(RUN_KEY, Some(RUN_NAME), &format!("\"{}\" --background", exe.display()))
        } else {
            use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyValueW};
            let key = wide(OsStr::new(RUN_KEY));
            let name = wide(OsStr::new(RUN_NAME));
            unsafe {
                let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()));
            }
            Ok(())
        }
    }

    /// Registers Snagger's name and icon for Windows notifications (per user).
    pub fn register_notifications(aumid: &str, icon: &Path) -> Result<(), String> {
        let key = format!(r"Software\Classes\AppUserModelId\{aumid}");
        reg_set(&key, Some("DisplayName"), "Snagger")?;
        reg_set(&key, Some("IconUri"), &icon.display().to_string())
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

    /// The progress shown on Snagger's taskbar button. Lives on one thread.
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
    //! Enough for the app to build elsewhere; Snagger targets Windows.
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

    pub fn reduced_motion() -> bool {
        false
    }
    pub fn keep_awake(_on: bool) {}
    pub fn hide_path(_path: &Path) {}
    pub fn sleep_pc() {}
    pub fn shut_down() {}
    pub fn register_protocol(_exe: &Path) -> Result<(), String> {
        Err("Only on Windows".into())
    }
    pub fn unregister_protocol() {}
    pub fn protocol_command() -> Option<String> {
        None
    }
    pub fn free_space(_path: &Path) -> Option<u64> {
        None
    }
    pub fn clipboard_seq() -> u32 {
        0
    }
    pub fn allow_foreground(_pid: u32) {}
    pub fn bring_to_front() {}
    pub fn hide_main_window() {}
    pub fn set_run_at_startup(_on: bool, _exe: &Path) -> Result<(), String> {
        Err("Only on Windows".into())
    }
    pub fn register_notifications(_aumid: &str, _icon: &Path) -> Result<(), String> {
        Err("Only on Windows".into())
    }
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
