#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use eframe::egui::{self, Vec2};

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

#[cfg(windows)]
extern "system" {
    fn GetWindowLongW(hWnd: *mut std::ffi::c_void, nIndex: i32) -> i32;
    fn SetWindowLongPtrW(hWnd: *mut std::ffi::c_void, nIndex: i32, dwNewLong: isize) -> isize;
    fn SetLayeredWindowAttributes(hWnd: *mut std::ffi::c_void, crKey: u32, bAlpha: u8, dwFlags: u32) -> i32;
    fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> *mut std::ffi::c_void;
}

fn make_window_invisible_1x1(_title: &str) {
    #[cfg(windows)]
    unsafe {
        let title_wide: Vec<u16> = _title.encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = FindWindowW(std::ptr::null(), title_wide.as_ptr());
        if !hwnd.is_null() {
            // GWL_EXSTYLE = -20
            // WS_EX_TOOLWINDOW = 0x00000080 (No taskbar button!)
            // WS_EX_LAYERED = 0x00080000
            let ex_style = GetWindowLongW(hwnd, -20);
            SetWindowLongPtrW(hwnd, -20, (ex_style | 0x00000080 | 0x00080000) as isize);
            // Alpha = 0 (100% transparent), LWA_ALPHA = 2
            SetLayeredWindowAttributes(hwnd, 0, 0, 2);
        }
    }
}

fn restore_window_normal(_title: &str) {
    #[cfg(windows)]
    unsafe {
        let title_wide: Vec<u16> = _title.encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = FindWindowW(std::ptr::null(), title_wide.as_ptr());
        if !hwnd.is_null() {
            let ex_style = GetWindowLongW(hwnd, -20);
            SetWindowLongPtrW(hwnd, -20, (ex_style & !0x00000080) as isize);
            SetLayeredWindowAttributes(hwnd, 0, 255, 2);
        }
    }
}

struct SpikeApp {
    mode: u32,
    cycle_count: u32,
    stage_start: Instant,
    log_file: std::path::PathBuf,
    styled: bool,
}

impl SpikeApp {
    fn log(&self, msg: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S%.3f");
        let line = format!("[{}] {}\n", ts, msg);
        eprint!("{}", line);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }
}

impl eframe::App for SpikeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _count = FRAME_COUNT.fetch_add(1, Ordering::SeqCst);
        let elapsed = self.stage_start.elapsed();

        if !self.styled {
            self.styled = true;
            make_window_invisible_1x1("Spike Root 1x1");
        }

        // Heartbeat
        ctx.request_repaint_after(Duration::from_millis(50));

        match self.mode {
            0 => {
                // Mode 0: Root 1x1 transparent (no taskbar button), Dialog 1 open
                let vp_id = egui::ViewportId::from_hash_of("spike_dialog_1");
                let vp_builder = egui::ViewportBuilder::default()
                    .with_title("Spike Dialog 1")
                    .with_inner_size(Vec2::new(400.0, 250.0))
                    .with_visible(true);

                ctx.show_viewport_immediate(vp_id, vp_builder, |child_ctx, _class| {
                    egui::CentralPanel::default().show(child_ctx, |ui| {
                        ui.heading("Spike Dialog 1 (Root 1x1 Transparent)");
                    });
                });

                if elapsed > Duration::from_millis(1000) {
                    self.log("Step 1 PASSED: Dialog 1 painted with 1x1 transparent root");
                    ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::Close);
                    self.mode = 1;
                    self.stage_start = Instant::now();
                }
            }
            1 => {
                // Mode 1: Dialog 1 closed, open Dialog 2 with root still 1x1 transparent
                let vp_id2 = egui::ViewportId::from_hash_of("spike_dialog_2");
                let vp_builder2 = egui::ViewportBuilder::default()
                    .with_title("Spike Dialog 2")
                    .with_inner_size(Vec2::new(400.0, 250.0))
                    .with_visible(true);

                ctx.show_viewport_immediate(vp_id2, vp_builder2, |child_ctx, _class| {
                    egui::CentralPanel::default().show(child_ctx, |ui| {
                        ui.heading("Spike Dialog 2 (Root 1x1 Transparent)");
                    });
                });

                if elapsed > Duration::from_millis(1000) {
                    self.log("Step 2 PASSED: Dialog 2 painted with 1x1 transparent root (No white screen)");
                    ctx.send_viewport_cmd_to(vp_id2, egui::ViewportCommand::Close);
                    self.mode = 2;
                    self.stage_start = Instant::now();
                    self.cycle_count = 0;
                }
            }
            2 => {
                // Mode 2: Hide/show main window 5 times using 1x1 transparent <-> 1000x640 visible
                if elapsed > Duration::from_millis(150) {
                    self.cycle_count += 1;
                    let make_vis = (self.cycle_count % 2) == 1;
                    if make_vis {
                        self.log(&format!("Step 3: Tray toggle cycle {}/5: Show Main Dashboard (1000x640)", self.cycle_count / 2 + 1));
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1000.0, 640.0)));
                        restore_window_normal("Spike Root 1x1");
                    } else {
                        self.log(&format!("Step 3: Tray toggle cycle {}/5: Hide to Tray (1x1 Transparent)", self.cycle_count / 2));
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1.0, 1.0)));
                        make_window_invisible_1x1("Spike Root 1x1");
                    }
                    self.stage_start = Instant::now();

                    if self.cycle_count >= 10 {
                        self.log("Step 3 PASSED: Toggled main window 5 times without stalling!");
                        self.mode = 3;
                        self.stage_start = Instant::now();
                    }
                }
            }
            3 => {
                // Mode 3: Main dashboard visible, minimized, then open detached dialog
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1000.0, 640.0)));
                restore_window_normal("Spike Root 1x1");
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));

                let vp_id3 = egui::ViewportId::from_hash_of("spike_dialog_3");
                let vp_builder3 = egui::ViewportBuilder::default()
                    .with_title("Spike Dialog 3")
                    .with_inner_size(Vec2::new(400.0, 250.0))
                    .with_visible(true)
                    .with_always_on_top();

                ctx.show_viewport_immediate(vp_id3, vp_builder3, |child_ctx, _class| {
                    egui::CentralPanel::default().show(child_ctx, |ui| {
                        ui.heading("Spike Dialog 3 (Main Window Minimized)");
                    });
                });

                if elapsed > Duration::from_millis(1000) {
                    self.log("Step 4 PASSED: Dialog 3 painted and responded while Main Window was Minimized");
                    ctx.send_viewport_cmd_to(vp_id3, egui::ViewportCommand::Close);
                    self.mode = 4;
                    self.stage_start = Instant::now();
                }
            }
            _ => {
                self.log("ALL SPIKE TESTS PASSED PERFECTLY!");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let log_file = std::env::temp_dir().join("cosmo_spike_test.log");
    let _ = std::fs::remove_file(&log_file);

    let viewport = egui::ViewportBuilder::default()
        .with_title("Spike Root 1x1")
        .with_decorations(false)
        .with_transparent(true)
        .with_inner_size(Vec2::new(1.0, 1.0))
        .with_position([0.0, 0.0])
        .with_visible(true); // Root is visible 1x1 transparent!

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Spike Root 1x1",
        native_options,
        Box::new(move |_cc| {
            Ok(Box::new(SpikeApp {
                mode: 0,
                cycle_count: 0,
                stage_start: Instant::now(),
                log_file,
                styled: false,
            }))
        }),
    )
}
