#![cfg(windows)]

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

pub const CREATE_NO_WINDOW: u32 = 0x08000000;

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static SUBCLASS_INSTALLED: AtomicBool = AtomicBool::new(false);
static PREV_WNDPROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static EGUI_CTX_FOR_SHUTDOWN: OnceLock<eframe::egui::Context> = OnceLock::new();

pub const SHUTDOWN_EVENT_NAME: &[u8] = b"Local\\CosmoDownloader_Shutdown_Event\0";

#[repr(C)]
#[allow(non_snake_case)]
pub struct SHELLEXECUTEINFOW {
    pub cbSize: u32,
    pub fMask: u32,
    pub hwnd: *mut std::ffi::c_void,
    pub lpVerb: *const u16,
    pub lpFile: *const u16,
    pub lpParameters: *const u16,
    pub lpDirectory: *const u16,
    pub nShow: i32,
    pub hInstApp: *mut std::ffi::c_void,
    pub lpIDList: *mut std::ffi::c_void,
    pub lpClass: *const u16,
    pub hkeyClass: *mut std::ffi::c_void,
    pub dwHotKey: u32,
    pub hIconOrMonitor: *mut std::ffi::c_void,
    pub hProcess: *mut std::ffi::c_void,
}

#[link(name = "shell32")]
#[link(name = "ole32")]
extern "system" {
    pub fn CoInitializeEx(pvReserved: *mut std::ffi::c_void, dwCoInit: u32) -> i32;
    pub fn CoUninitialize();
    pub fn ShellExecuteW(
        hwnd: *mut std::ffi::c_void,
        lpOperation: *const u16,
        lpFile: *const u16,
        lpParameters: *const u16,
        lpDirectory: *const u16,
        nShowCmd: i32,
    ) -> isize;
    pub fn ShellExecuteExW(pExecInfo: *mut SHELLEXECUTEINFOW) -> i32;
    pub fn CoCreateInstance(
        rclsid: *const WinGuid,
        pUnkOuter: *mut std::ffi::c_void,
        dwClsContext: u32,
        riid: *const WinGuid,
        ppv: *mut *mut std::ffi::c_void,
    ) -> i32;
}

#[repr(C)]
pub struct WinGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub const CLSID_TASKBAR_LIST: WinGuid = WinGuid {
    data1: 0x56FDF344,
    data2: 0xFD6D,
    data3: 0x11D0,
    data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
};

pub const IID_ITASKBAR_LIST3: WinGuid = WinGuid {
    data1: 0xEA1AFB91,
    data2: 0x9E28,
    data3: 0x4B86,
    data4: [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF],
};

pub const TBPF_NOPROGRESS: u32 = 0x0;
pub const TBPF_INDETERMINATE: u32 = 0x1;
pub const TBPF_NORMAL: u32 = 0x2;
pub const TBPF_ERROR: u32 = 0x4;
pub const TBPF_PAUSED: u32 = 0x8;

#[repr(C)]
struct ITaskbarList3Vtbl {
    query_interface: usize,
    add_ref: unsafe extern "system" fn(this: *mut std::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(this: *mut std::ffi::c_void) -> u32,
    hr_init: unsafe extern "system" fn(this: *mut std::ffi::c_void) -> i32,
    add_tab: usize,
    delete_tab: usize,
    activate_tab: usize,
    set_active_alt: usize,
    mark_fullscreen_window: usize,
    set_progress_value: unsafe extern "system" fn(this: *mut std::ffi::c_void, hwnd: *mut std::ffi::c_void, ull_completed: u64, ull_total: u64) -> i32,
    set_progress_state: unsafe extern "system" fn(this: *mut std::ffi::c_void, hwnd: *mut std::ffi::c_void, tbp_flags: u32) -> i32,
}

pub fn update_taskbar_progress(hwnd: *mut std::ffi::c_void, flags: u32, completed: u64, total: u64) {
    if hwnd.is_null() {
        return;
    }
    unsafe {
        let _ = CoInitializeEx(std::ptr::null_mut(), 2);
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        // CLSCTX_INPROC_SERVER = 1
        if CoCreateInstance(&CLSID_TASKBAR_LIST, std::ptr::null_mut(), 1, &IID_ITASKBAR_LIST3, &mut ptr) == 0 && !ptr.is_null() {
            let vtbl = *(ptr as *mut *mut ITaskbarList3Vtbl);
            let _ = ((*vtbl).hr_init)(ptr);
            let _ = ((*vtbl).set_progress_state)(ptr, hwnd, flags);
            if total > 0 && flags != TBPF_NOPROGRESS {
                let _ = ((*vtbl).set_progress_value)(ptr, hwnd, completed, total);
            }
            let _ = ((*vtbl).release)(ptr);
        }
        CoUninitialize();
    }
}

pub fn set_taskbar_progress(flags: u32, completed: u64, total: u64) {
    let hwnd = get_process_window();
    update_taskbar_progress(hwnd, flags, completed, total);
}

extern "system" {
    pub fn CreateMutexA(lpMutexAttributes: *mut std::ffi::c_void, bInitialOwner: i32, lpName: *const u8) -> *mut std::ffi::c_void;
    pub fn CreateEventA(lpEventAttributes: *mut std::ffi::c_void, bManualReset: i32, bInitialState: i32, lpName: *const u8) -> *mut std::ffi::c_void;
    pub fn OpenEventA(dwDesiredAccess: u32, bInheritHandle: i32, lpName: *const u8) -> *mut std::ffi::c_void;
    pub fn SetEvent(hEvent: *mut std::ffi::c_void) -> i32;
    pub fn ResetEvent(hEvent: *mut std::ffi::c_void) -> i32;
    pub fn WaitForSingleObject(hHandle: *mut std::ffi::c_void, dwMilliseconds: u32) -> u32;
    pub fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    pub fn GetLastError() -> u32;
    pub fn GetCurrentProcessId() -> u32;
    pub fn GetWindowThreadProcessId(hWnd: *mut std::ffi::c_void, lpdwProcessId: *mut u32) -> u32;
    pub fn EnumWindows(lpEnumFunc: Option<unsafe extern "system" fn(*mut std::ffi::c_void, isize) -> i32>, lParam: isize) -> i32;
    pub fn GetWindowLongW(hWnd: *mut std::ffi::c_void, nIndex: i32) -> i32;
    pub fn GetWindowTextLengthW(hWnd: *mut std::ffi::c_void) -> i32;
    pub fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> *mut std::ffi::c_void;
    pub fn ShowWindow(hWnd: *mut std::ffi::c_void, nCmdShow: i32) -> i32;
    pub fn SetForegroundWindow(hWnd: *mut std::ffi::c_void) -> i32;
    pub fn BringWindowToTop(hWnd: *mut std::ffi::c_void) -> i32;
    pub fn keybd_event(bVk: u8, bScan: u8, dwFlags: u32, dwExtraInfo: usize);
    pub fn SetWindowPos(hWnd: *mut std::ffi::c_void, hWndInsertAfter: *mut std::ffi::c_void, X: i32, Y: i32, cx: i32, cy: i32, uFlags: u32) -> i32;
    pub fn GetSystemMetrics(nIndex: i32) -> i32;
    pub fn SetWindowLongPtrW(hWnd: *mut std::ffi::c_void, nIndex: i32, dwNewLong: isize) -> isize;
    pub fn CallWindowProcW(lpPrevWndFunc: isize, hWnd: *mut std::ffi::c_void, Msg: u32, wParam: usize, lParam: isize) -> isize;
    pub fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    pub fn MonitorFromPoint(pt: POINT, dwFlags: u32) -> *mut std::ffi::c_void;
    pub fn GetMonitorInfoW(hMonitor: *mut std::ffi::c_void, lpmi: *mut MONITORINFO) -> i32;
    pub fn GetWindowRect(hWnd: *mut std::ffi::c_void, lpRect: *mut RECT) -> i32;
    pub fn IsWindowVisible(hWnd: *mut std::ffi::c_void) -> i32;
    pub fn IsIconic(hWnd: *mut std::ffi::c_void) -> i32;
    pub fn GetWindowTextW(hWnd: *mut std::ffi::c_void, lpString: *mut u16, nMaxCount: i32) -> i32;
    pub fn IsZoomed(hWnd: *mut std::ffi::c_void) -> i32;
    pub fn GetWindow(hWnd: *mut std::ffi::c_void, uCmd: u32) -> *mut std::ffi::c_void;
    pub fn GetLayeredWindowAttributes(hWnd: *mut std::ffi::c_void, pcrKey: *mut u32, pbAlpha: *mut u8, pdwFlags: *mut u32) -> i32;
    pub fn GetForegroundWindow() -> *mut std::ffi::c_void;
    pub fn GetCurrentThreadId() -> u32;
    pub fn AttachThreadInput(idAttach: u32, idAttachTo: u32, fAttach: i32) -> i32;
    pub fn SetFocus(hWnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    pub fn FlashWindowEx(pfwi: *const FLASHWINFO) -> i32;
}

#[repr(C)]
#[allow(non_snake_case)]
pub struct FLASHWINFO {
    pub cbSize: u32,
    pub hwnd: *mut std::ffi::c_void,
    pub dwFlags: u32,
    pub uCount: u32,
    pub dwTimeout: u32,
}

pub const FLASHW_ALL: u32 = 0x00000003;
pub const FLASHW_TIMERNOFG: u32 = 0x0000000C;

pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOW: i32 = 5;
pub const SW_RESTORE: i32 = 9;

#[repr(C)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[allow(non_snake_case)]
pub struct MONITORINFO {
    pub cbSize: u32,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: u32,
}

unsafe extern "system" fn shutdown_wndproc(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    const WM_QUERYENDSESSION: u32 = 0x0011;
    const WM_ENDSESSION: u32 = 0x0016;
    if msg == WM_QUERYENDSESSION {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
        if let Some(ctx) = EGUI_CTX_FOR_SHUTDOWN.get() {
            ctx.request_repaint();
        }
        return 1;
    }
    if msg == WM_ENDSESSION && wparam != 0 {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
        if let Some(ctx) = EGUI_CTX_FOR_SHUTDOWN.get() {
            ctx.request_repaint();
        }
    }
    let prev = PREV_WNDPROC.load(Ordering::SeqCst);
    if prev != 0 {
        CallWindowProcW(prev, hwnd, msg, wparam, lparam)
    } else {
        0
    }
}

/// Subclasses the app's HWND once to intercept Windows RestartManager / session end messages.
pub fn init_shutdown_listener(ctx: &eframe::egui::Context) {
    let _ = EGUI_CTX_FOR_SHUTDOWN.set(ctx.clone());
    if !SUBCLASS_INSTALLED.swap(true, Ordering::SeqCst) {
        // Spawn zero-CPU waiter on named Win32 shutdown event signaled by installer or `--quit`
        std::thread::spawn(|| unsafe {
            let h = CreateEventA(std::ptr::null_mut(), 1, 0, SHUTDOWN_EVENT_NAME.as_ptr());
            if !h.is_null() {
                ResetEvent(h);
                WaitForSingleObject(h, u32::MAX);
                SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
                if let Some(c) = EGUI_CTX_FOR_SHUTDOWN.get() {
                    c.request_repaint();
                }
                CloseHandle(h);
            }
        });
    }

    if PREV_WNDPROC.load(Ordering::SeqCst) == 0 {
        let hwnd = get_process_window();
        if !hwnd.is_null() {
            unsafe {
                let prev = SetWindowLongPtrW(hwnd, -4, shutdown_wndproc as *const () as isize);
                if prev != 0 {
                    PREV_WNDPROC.store(prev, Ordering::SeqCst);
                }
            }
        }
    }
}

/// Returns true if OS RestartManager, Windows logoff/shutdown, or `--quit`/installer requested exit.
pub fn is_external_shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

/// Signals an already-running instance to gracefully save and exit. Returns true if event existed.
pub fn signal_existing_instance_shutdown() -> bool {
    unsafe {
        // EVENT_MODIFY_STATE = 0x0002
        let h = OpenEventA(0x0002, 0, SHUTDOWN_EVENT_NAME.as_ptr());
        if !h.is_null() {
            let ok = SetEvent(h);
            CloseHandle(h);
            return ok != 0;
        }
        false
    }
}

pub const HWND_TOPMOST: *mut std::ffi::c_void = (-1isize) as *mut std::ffi::c_void;
pub const HWND_NOTOPMOST: *mut std::ffi::c_void = (-2isize) as *mut std::ffi::c_void;

pub fn get_process_window() -> *mut std::ffi::c_void {
    unsafe {
        let current_pid = GetCurrentProcessId();
        struct EnumData {
            pid: u32,
            hwnd: *mut std::ffi::c_void,
        }
        let mut data = EnumData {
            pid: current_pid,
            hwnd: std::ptr::null_mut(),
        };

        unsafe extern "system" fn enum_proc(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
            let data = &mut *(lparam as *mut EnumData);
            let mut wnd_pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut wnd_pid);
            if wnd_pid == data.pid {
                let style = GetWindowLongW(hwnd, -16) as u32;
                if (style & 0x40000000) == 0 {
                    let text_len = GetWindowTextLengthW(hwnd);
                    if text_len > 0 {
                        data.hwnd = hwnd;
                        return 0;
                    } else if data.hwnd.is_null() {
                        data.hwnd = hwnd;
                    }
                }
            }
            1
        }

        EnumWindows(Some(enum_proc), &mut data as *mut _ as isize);
        if !data.hwnd.is_null() {
            return data.hwnd;
        }

        let titles = [
            "Download File Info\0",
            "New download\0",
            "Downloading - Cosmo Downloader\0",
            "Download complete\0",
            "Download Complete\0",
            "Cosmo Downloader\0",
            "CosmoDownloader\0",
        ];
        for t in titles {
            let encoded: Vec<u16> = t.encode_utf16().collect();
            let hwnd = FindWindowW(std::ptr::null(), encoded.as_ptr());
            if !hwnd.is_null() {
                return hwnd;
            }
        }

        std::ptr::null_mut()
    }
}

pub fn get_main_dashboard_window() -> *mut std::ffi::c_void {
    unsafe {
        let current_pid = GetCurrentProcessId();
        struct EnumData {
            pid: u32,
            hwnd: *mut std::ffi::c_void,
        }
        let mut data = EnumData {
            pid: current_pid,
            hwnd: std::ptr::null_mut(),
        };

        unsafe extern "system" fn enum_proc(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
            let data = &mut *(lparam as *mut EnumData);
            let mut wnd_pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut wnd_pid);
            if wnd_pid == data.pid {
                let style = GetWindowLongW(hwnd, -16) as u32;
                if (style & 0x40000000) == 0 { // not WS_CHILD
                    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    GetWindowRect(hwnd, &mut rc);
                    let w = rc.right - rc.left;
                    let h = rc.bottom - rc.top;
                    if w >= 600 && h >= 400 && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 && rc.left > -10000 && rc.top > -10000 {
                        data.hwnd = hwnd;
                        return 0; // stop
                    }
                }
            }
            1
        }

        EnumWindows(Some(enum_proc), &mut data as *mut _ as isize);
        if !data.hwnd.is_null() {
            return data.hwnd;
        }

        let title: Vec<u16> = "Cosmo Downloader\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut rc);
            if rc.left > -10000 && rc.top > -10000 {
                return hwnd;
            }
        }

        std::ptr::null_mut()
    }
}

pub fn get_dialog_center_pos(main_window_visible: bool, width: f32, height: f32) -> (f32, f32) {
    unsafe {
        if main_window_visible {
            let hwnd = get_main_dashboard_window();
            if !hwnd.is_null() && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
                let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                if GetWindowRect(hwnd, &mut rc) != 0 {
                    let w = rc.right - rc.left;
                    let h = rc.bottom - rc.top;
                    if rc.left > -10000 && rc.top > -10000 && w >= 400 && h >= 300 {
                        let cx = rc.left + w / 2;
                        let cy = rc.top + h / 2;
                        let x = cx - (width as i32 / 2);
                        let y = cy - (height as i32 / 2);
                        return (x as f32, y as f32);
                    }
                }
            }
        }

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        const MONITOR_DEFAULTTONEAREST: u32 = 2;
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if !hmon.is_null() {
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                rcWork: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                dwFlags: 0,
            };
            if GetMonitorInfoW(hmon, &mut mi) != 0 {
                let cx = mi.rcWork.left + (mi.rcWork.right - mi.rcWork.left) / 2;
                let cy = mi.rcWork.top + (mi.rcWork.bottom - mi.rcWork.top) / 2;
                let x = cx - (width as i32 / 2);
                let y = cy - (height as i32 / 2);
                return (x as f32, y as f32);
            }
        }

        (100.0, 100.0)
    }
}

pub fn get_dialog_window() -> *mut std::ffi::c_void {
    unsafe {
        let current_pid = GetCurrentProcessId();
        struct EnumData {
            pid: u32,
            hwnd: *mut std::ffi::c_void,
        }
        let mut data = EnumData {
            pid: current_pid,
            hwnd: std::ptr::null_mut(),
        };

        unsafe extern "system" fn enum_proc(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
            let data = &mut *(lparam as *mut EnumData);
            let mut wnd_pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut wnd_pid);
            if wnd_pid == data.pid {
                let style = GetWindowLongW(hwnd, -16) as u32;
                if (style & 0x40000000) == 0 {
                    let mut buf = [0u16; 256];
                    let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
                    if len > 0 {
                        let title = String::from_utf16_lossy(&buf[..len as usize]);
                        if title.contains("Download File Info") || title.contains("New download") {
                            data.hwnd = hwnd;
                            return 0;
                        }
                    }
                }
            }
            1
        }

        EnumWindows(Some(enum_proc), &mut data as *mut _ as isize);
        if !data.hwnd.is_null() {
            return data.hwnd;
        }

        get_process_window()
    }
}

pub fn show_download_popup_window() {
    let hwnd = get_dialog_window();
    if !hwnd.is_null() {
        unsafe {
            let style = GetWindowLongW(hwnd, -16) as u32;
            let new_style = (style & !0x00C00000) | 0x00040000;
            SetWindowLongPtrW(hwnd, -16, new_style as isize);

            // Step 2: Show dialog with WindowState = Normal, ShowActivated = true.
            // If minimized, SW_RESTORE first.
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            ShowWindow(hwnd, SW_SHOWNORMAL);
            ShowWindow(hwnd, SW_SHOW);

            // Break Windows foreground lock via keyboard event pulse (VK_MENU)
            keybd_event(0x12, 0, 0, 0); // VK_MENU down
            keybd_event(0x12, 0, 0x0002, 0); // VK_MENU up

            // Step 3: Topmost and BringWindowToTop so it is guaranteed in front of browser/active app
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0020 | 0x0040);
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);

            // AttachThreadInput fallback if foreground activation is constrained by OS
            let fg_wnd = GetForegroundWindow();
            if !fg_wnd.is_null() && fg_wnd != hwnd {
                let mut fg_pid = 0u32;
                let fg_thread = GetWindowThreadProcessId(fg_wnd, &mut fg_pid);
                let cur_thread = GetCurrentThreadId();
                if fg_thread != 0 && fg_thread != cur_thread {
                    AttachThreadInput(cur_thread, fg_thread, 1);
                    BringWindowToTop(hwnd);
                    SetForegroundWindow(hwnd);
                    SetFocus(hwnd);
                    AttachThreadInput(cur_thread, fg_thread, 0);
                }
            }

            let fwi = FLASHWINFO {
                cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                hwnd,
                dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
                uCount: 4,
                dwTimeout: 0,
            };
            FlashWindowEx(&fwi);
        }
    }
}

pub fn show_main_window() {
    let hwnd = get_process_window();
    if !hwnd.is_null() {
        unsafe {
            let style = GetWindowLongW(hwnd, -16) as u32;
            let new_style = (style & !0x00C00000) | 0x00040000;
            SetWindowLongPtrW(hwnd, -16, new_style as isize);
            ShowWindow(hwnd, 9);
            ShowWindow(hwnd, 5);
            SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0020 | 0x0040);
            SetForegroundWindow(hwnd);
        }
    }
}

pub fn make_popup_topmost() {
    let hwnd = get_process_window();
    if !hwnd.is_null() {
        unsafe {
            let style = GetWindowLongW(hwnd, -16) as u32;
            let new_style = (style & !0x00C00000) | 0x00040000;
            SetWindowLongPtrW(hwnd, -16, new_style as isize);
            ShowWindow(hwnd, 9);
            ShowWindow(hwnd, 5);
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0020 | 0x0040);
            SetForegroundWindow(hwnd);
        }
    }
}

pub fn remove_popup_topmost() {
    let hwnd = get_process_window();
    if !hwnd.is_null() {
        unsafe {
            let style = GetWindowLongW(hwnd, -16) as u32;
            let new_style = (style & !0x00C00000) | 0x00040000;
            SetWindowLongPtrW(hwnd, -16, new_style as isize);
            SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0020 | 0x0040);
        }
    }
}

pub fn hide_main_window() {
    let hwnd = get_process_window();
    if !hwnd.is_null() {
        unsafe {
            ShowWindow(hwnd, 0);
        }
    }
}

pub fn shell_open_file(path: &str) {
    let target = path.to_string();
    std::thread::spawn(move || {
        let p = Path::new(&target);
        let actual_target = if !p.exists() {
            let mut resolved = target.clone();
            if let Some(parent) = p.parent() {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(entries) = std::fs::read_dir(parent) {
                        for entry in entries.flatten() {
                            let ep = entry.path();
                            if ep.is_file() {
                                if let Some(name) = ep.file_name().and_then(|n| n.to_str()) {
                                    if name.starts_with(stem) && !name.ends_with(".part") && !name.ends_with(".ytdl") {
                                        resolved = ep.to_string_lossy().to_string();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            resolved
        } else {
            target
        };

        let wide: Vec<u16> = actual_target.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let res = ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            );
            if (res as isize) <= 32 {
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", &actual_target])
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn();
            }
        }
    });
}

pub fn shell_open_with(path: &str) {
    let target = path.to_string();
    std::thread::spawn(move || {
        let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
        let openas_verb: Vec<u16> = "openas\0".encode_utf16().collect();
        unsafe {
            let _ = CoInitializeEx(std::ptr::null_mut(), 2);
            let mut exec_info: SHELLEXECUTEINFOW = std::mem::zeroed();
            exec_info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            exec_info.fMask = 0x0000000C;
            exec_info.hwnd = get_process_window();
            exec_info.lpVerb = openas_verb.as_ptr();
            exec_info.lpFile = wide.as_ptr();
            exec_info.nShow = 1;

            let success = ShellExecuteExW(&mut exec_info);
            if success == 0 {
                let _ = std::process::Command::new("rundll32.exe")
                    .arg("shell32.dll,OpenAs_RunDLL")
                    .arg(&target)
                    .spawn();
            }
            CoUninitialize();
        }
    });
}

pub fn shell_open_folder_and_select(path: &str) {
    let target = path.to_string();
    std::thread::spawn(move || {
        let p = Path::new(&target);
        if p.is_dir() {
            let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                let res = ShellExecuteW(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                );
                if (res as isize) > 32 {
                    return;
                }
            }
            let _ = std::process::Command::new("explorer").arg(&target).spawn();
            return;
        }

        if p.is_file() {
            let win_path = target.replace('/', "\\");
            let mut cmd = std::process::Command::new("explorer");
            cmd.raw_arg(format!(r#"/select,"{}""#, win_path));
            if cmd.spawn().is_ok() {
                return;
            }
        }

        if let Some(parent) = p.parent() {
            if parent.exists() && parent.is_dir() {
                let parent_str = parent.to_string_lossy().to_string();
                let wide: Vec<u16> = parent_str.encode_utf16().chain(std::iter::once(0)).collect();
                unsafe {
                    let res = ShellExecuteW(
                        std::ptr::null_mut(),
                        std::ptr::null(),
                        wide.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        1,
                    );
                    if (res as isize) > 32 {
                        return;
                    }
                }
                let _ = std::process::Command::new("explorer").arg(&parent_str).spawn();
            }
        }
    });
}
