#![cfg(not(windows))]

use std::path::Path;

pub fn show_download_popup_window() {}
pub fn show_download_window_hwnd(_hwnd: *mut std::ffi::c_void) {}
pub fn minimize_dialog_window() {}
pub fn grant_foreground_permission() {}
pub fn drag_window_under_cursor() {}
pub fn drag_window_by_title(_title: &str) {}
pub fn show_main_window() {}
pub fn make_popup_topmost() {}
pub fn remove_popup_topmost() {}
pub fn hide_main_window() {}
pub fn trim_working_set() {}
pub fn make_window_invisible_toolwindow(_title: &str) {}
pub fn restore_window_normal(_title: &str) {}
pub fn flash_window_hwnd(_hwnd: *mut std::ffi::c_void) {}
pub fn flash_dialog_window() {}
pub fn get_dialog_center_pos(_main_window_visible: bool, _width: f32, _height: f32) -> (f32, f32) {
    (100.0, 100.0)
}

pub const TBPF_NOPROGRESS: u32 = 0x0;
pub const TBPF_INDETERMINATE: u32 = 0x1;
pub const TBPF_NORMAL: u32 = 0x2;
pub const TBPF_ERROR: u32 = 0x4;
pub const TBPF_PAUSED: u32 = 0x8;

pub fn update_taskbar_progress(_hwnd: *mut std::ffi::c_void, _flags: u32, _completed: u64, _total: u64) {}

pub fn set_taskbar_progress(_flags: u32, _completed: u64, _total: u64) {}

pub fn remove_window_from_taskbar(_hwnd: *mut std::ffi::c_void) {}

pub fn add_window_to_taskbar(_hwnd: *mut std::ffi::c_void) {}

pub fn shell_open_file(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

pub fn shell_open_with(path: &str) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

pub fn shell_open_folder_and_select(path: &str) {
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
    }
}

pub fn pick_save_file_dialog(
    title: &str,
    start_dir: &Path,
    file_name: &str,
) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .set_title(title)
        .set_directory(start_dir)
        .set_file_name(file_name)
        .save_file()
}

pub fn pick_folder_dialog(
    title: &str,
    start_dir: Option<&Path>,
) -> Option<std::path::PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_folder()
}
