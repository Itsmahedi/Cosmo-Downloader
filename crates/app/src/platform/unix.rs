#![cfg(not(windows))]

use std::path::Path;

pub fn show_download_popup_window() {}
pub fn show_main_window() {}
pub fn make_popup_topmost() {}
pub fn remove_popup_topmost() {}
pub fn hide_main_window() {}
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
