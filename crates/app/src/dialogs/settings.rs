use std::path::Path;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2, RichText};
use crate::theme::{shapes, ThemeColors};
use crate::icons;
use crate::platform;

// ============================================================================
// Section 7 Options Window Implementation (Cosmo Downloader Specification)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsTab {
    General,
    Downloads,
    Notifications,
    Network,
}

impl OptionsTab {
    pub fn label(&self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Downloads => "Downloads",
            Self::Notifications => "Notifications",
            Self::Network => "Network",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedUnit {
    Kbps,
    Mbps,
}

impl SpeedUnit {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Kbps => "KB/s",
            Self::Mbps => "MB/s",
        }
    }
}

#[derive(Clone, Debug)]
pub struct OptionsDialogState {
    pub active_tab: OptionsTab,
    pub startup: bool,
    pub minstart: bool,
    pub tray: bool,
    pub mintray: bool,
    pub threads: usize,
    pub speed: u64,
    pub unit: SpeedUnit,
    pub folder: String,
    pub clip: bool,
    pub notif: bool,
    pub cwin: bool,
    pub openmain: bool,
    pub proxy: String,
}

impl Default for OptionsDialogState {
    fn default() -> Self {
        Self {
            active_tab: OptionsTab::General,
            startup: false,
            minstart: false,
            tray: true,
            mintray: true,
            threads: 8,
            speed: 0,
            unit: SpeedUnit::Mbps,
            folder: String::new(),
            clip: false,
            notif: true,
            cwin: true,
            openmain: false,
            proxy: String::new(),
        }
    }
}

impl OptionsDialogState {
    pub fn reset_to_defaults(&mut self) {
        let cur_tab = self.active_tab;
        *self = Self::default();
        self.active_tab = cur_tab;
    }

    pub fn is_proxy_valid(&self) -> bool {
        is_proxy_valid(&self.proxy)
    }

    pub fn is_start_minimized_enabled(&self) -> bool {
        self.startup && self.tray
    }

    pub fn is_minimize_to_tray_enabled(&self) -> bool {
        self.tray
    }
}

pub fn is_proxy_valid(proxy: &str) -> bool {
    let s = proxy.trim();
    if s.is_empty() {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("http://") {
        r
    } else if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("socks5://") {
        r
    } else if let Some(r) = lower.strip_prefix("socks5h://") {
        r
    } else {
        return false;
    };

    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.contains(' ') || rest.contains('/') {
        return false;
    }

    if let Some((_host, port_str)) = rest.split_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            port > 0
        } else {
            false
        }
    } else {
        !rest.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsDialogOutcome {
    None,
    Save,
    Cancel,
}

// ----------------------------------------------------------------------------
// Backward-compatible outcome and state for tests
// ----------------------------------------------------------------------------

pub struct SettingsDialogOutcome {
    pub save_and_close: bool,
}

pub struct SettingsViewState<'a> {
    pub max_concurrent: &'a mut usize,
    pub global_speed_kb: &'a mut u64,
    pub default_connections: &'a mut usize,
    pub default_save_dir: &'a mut String,
    pub proxy_url: &'a mut String,
    pub clipboard_monitor: &'a mut bool,
    pub notifications_enabled: &'a mut bool,
    pub open_main_on_download_start: &'a mut bool,
    pub dont_show_complete_dialog: &'a mut bool,
    pub minimize_to_tray: &'a mut bool,
    pub autostart: &'a mut bool,
    pub start_minimized: &'a mut bool,
}

pub fn render_settings_dialog(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    state: &mut SettingsViewState<'_>,
) -> SettingsDialogOutcome {
    let mut outcome = SettingsDialogOutcome { save_and_close: false };

    let mut opt_state = OptionsDialogState {
        active_tab: OptionsTab::General,
        startup: *state.autostart,
        minstart: *state.start_minimized,
        tray: true,
        mintray: *state.minimize_to_tray,
        threads: *state.default_connections,
        speed: if *state.global_speed_kb % 1024 == 0 && *state.global_speed_kb > 0 {
            *state.global_speed_kb / 1024
        } else {
            *state.global_speed_kb
        },
        unit: if *state.global_speed_kb % 1024 == 0 && *state.global_speed_kb > 0 {
            SpeedUnit::Mbps
        } else {
            SpeedUnit::Kbps
        },
        folder: state.default_save_dir.clone(),
        clip: *state.clipboard_monitor,
        notif: *state.notifications_enabled,
        cwin: !*state.dont_show_complete_dialog,
        openmain: *state.open_main_on_download_start,
        proxy: state.proxy_url.clone(),
    };

    let opt_outcome = render_options_window_content(ui, pal, None, &mut opt_state);
    if opt_outcome == OptionsDialogOutcome::Save {
        *state.autostart = opt_state.startup;
        *state.start_minimized = opt_state.minstart;
        *state.minimize_to_tray = opt_state.mintray;
        *state.default_connections = opt_state.threads;
        *state.global_speed_kb = match opt_state.unit {
            SpeedUnit::Mbps => opt_state.speed * 1024,
            SpeedUnit::Kbps => opt_state.speed,
        };
        *state.default_save_dir = opt_state.folder;
        *state.clipboard_monitor = opt_state.clip;
        *state.notifications_enabled = opt_state.notif;
        *state.dont_show_complete_dialog = !opt_state.cwin;
        *state.open_main_on_download_start = opt_state.openmain;
        *state.proxy_url = opt_state.proxy;
        outcome.save_and_close = true;
    }

    outcome
}

// ----------------------------------------------------------------------------
// Color and Animation Helpers
// ----------------------------------------------------------------------------

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_color(c1: Color32, c2: Color32, t: f32) -> Color32 {
    let r = lerp_f32(c1.r() as f32, c2.r() as f32, t).round() as u8;
    let g = lerp_f32(c1.g() as f32, c2.g() as f32, t).round() as u8;
    let b = lerp_f32(c1.b() as f32, c2.b() as f32, t).round() as u8;
    let a = lerp_f32(c1.a() as f32, c2.a() as f32, t).round() as u8;
    Color32::from_rgba_premultiplied(r, g, b, a)
}

pub fn warn_color(pal: &ThemeColors) -> Color32 {
    if pal.is_dark {
        Color32::from_rgb(0xE2, 0xB0, 0x4A) // #e2b04a
    } else {
        Color32::from_rgb(0x9A, 0x67, 0x00) // #9a6700
    }
}

pub fn warn_bg_color(pal: &ThemeColors) -> Color32 {
    let w = warn_color(pal);
    Color32::from_rgba_premultiplied(
        (w.r() as f32 * 0.12).round() as u8,
        (w.g() as f32 * 0.12).round() as u8,
        (w.b() as f32 * 0.12).round() as u8,
        31,
    )
}

// ----------------------------------------------------------------------------
// Custom Toggle Switch Widget (.opsw in HTML)
// ----------------------------------------------------------------------------

pub fn toggle_switch(
    ui: &mut egui::Ui,
    value: &mut bool,
    enabled: bool,
    pal: &ThemeColors,
) -> egui::Response {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if !enabled {
        response = response.on_hover_cursor(egui::CursorIcon::NotAllowed);
    } else {
        response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }

    if enabled && response.clicked() {
        *value = !*value;
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, *value && enabled);

        let bg_color = if !enabled {
            pal.line.gamma_multiply(0.4)
        } else {
            let active_bg = pal.accent;
            let inactive_bg = pal.line;
            lerp_color(inactive_bg, active_bg, t)
        };

        ui.painter().rect_filled(rect, 10.0, bg_color);

        // Knob: 14px diameter (radius 7.0), moves between x=10 and x=26
        let knob_x = lerp_f32(rect.left() + 10.0, rect.right() - 10.0, t);
        let knob_center = Pos2::new(knob_x, rect.center().y);

        let knob_color = if !enabled {
            pal.muted.gamma_multiply(0.4)
        } else {
            let active_knob = pal.on_accent;
            let inactive_knob = pal.muted;
            lerp_color(inactive_knob, active_knob, t)
        };

        ui.painter().circle_filled(knob_center, 7.0, knob_color);
    }

    response
}

// ----------------------------------------------------------------------------
// Setting Row Layout Helper (.opr in HTML)
// ----------------------------------------------------------------------------

fn render_setting_row<F>(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    label: &str,
    description: &str,
    off: bool,
    hint: Option<&str>,
    add_divider: bool,
    control_width: f32,
    render_control: F,
) where
    F: FnOnce(&mut egui::Ui, &ThemeColors),
{
    let warn_c = warn_color(pal);
    let title_color = if off { pal.text.gamma_multiply(0.45) } else { pal.text };
    let desc_color = if off { pal.muted.gamma_multiply(0.45) } else { pal.muted };

    ui.horizontal(|ui| {
        let total_avail = ui.available_width();
        let text_width = (total_avail - control_width - 16.0).max(120.0);

        ui.allocate_ui_with_layout(
            Vec2::new(text_width, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.label(RichText::new(label).strong().size(13.0).color(title_color));
                ui.label(RichText::new(description).size(11.5).color(desc_color));
                if off {
                    if let Some(h) = hint {
                        ui.label(RichText::new(h).size(11.0).color(warn_c));
                    }
                }
            },
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            render_control(ui, pal);
        });
    });

    if add_divider {
        ui.add_space(5.0);
        let (line_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().rect_filled(line_rect, 0.0, pal.line);
        ui.add_space(5.0);
    }
}

// ----------------------------------------------------------------------------
// Main Render Function for Options Window
// ----------------------------------------------------------------------------

pub fn render_options_window_content(
    ui: &mut egui::Ui,
    pal: &ThemeColors,
    logo_tex: Option<&egui::TextureHandle>,
    state: &mut OptionsDialogState,
) -> OptionsDialogOutcome {
    let mut outcome = OptionsDialogOutcome::None;

    // Handle Escape key
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return OptionsDialogOutcome::Cancel;
    }

    let total_width = 620.0;
    let total_height = 490.0;

    ui.allocate_ui_with_layout(
        Vec2::new(total_width, total_height),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            // ----------------------------------------------------------------
            // 1. Title Bar (Height: 36px)
            // ----------------------------------------------------------------
            let bar_rect = ui.allocate_space(Vec2::new(total_width, 36.0)).1;
            let top_rounding = eframe::egui::Rounding { nw: shapes::R.nw, ne: shapes::R.ne, sw: 0.0, se: 0.0 };
            ui.painter().rect_filled(bar_rect, top_rounding, pal.raise);
            ui.painter().rect_filled(
                Rect::from_min_max(Pos2::new(bar_rect.left(), bar_rect.bottom() - 1.0), bar_rect.max),
                0.0,
                pal.line,
            );

            // Drag area
            let wc_width = 46.0;
            let drag_rect = Rect::from_min_max(
                bar_rect.min,
                Pos2::new(bar_rect.right() - wc_width, bar_rect.bottom()),
            );
            let drag_resp = ui.interact(drag_rect, ui.id().with("options_title_drag"), egui::Sense::click_and_drag());
            if drag_resp.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                #[cfg(windows)]
                crate::platform::drag_window_under_cursor();
            }

            // Logo (18x18)
            let mid_y = bar_rect.center().y;
            let logo_rect = Rect::from_center_size(Pos2::new(bar_rect.left() + 20.0, mid_y), Vec2::splat(18.0));
            if let Some(tex) = logo_tex {
                ui.painter().image(
                    tex.id(),
                    logo_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                icons::draw_logo(ui.painter(), logo_rect, pal.accent);
            }

            // Title
            ui.painter().text(
                Pos2::new(bar_rect.left() + 38.0, mid_y),
                egui::Align2::LEFT_CENTER,
                "Options",
                egui::FontId::proportional(13.0),
                pal.text,
            );

            // Close button [X]
            let btn_rect = Rect::from_min_size(Pos2::new(bar_rect.right() - wc_width, bar_rect.top()), Vec2::new(46.0, 36.0));
            let resp = ui.interact(btn_rect, ui.id().with("options_close_btn"), egui::Sense::click());
            if resp.hovered() {
                let btn_rounding = eframe::egui::Rounding { nw: 0.0, ne: shapes::R.ne, sw: 0.0, se: 0.0 };
                ui.painter().rect_filled(btn_rect, btn_rounding, pal.danger);
                icons::draw_close(ui.painter(), Rect::from_center_size(btn_rect.center(), Vec2::splat(14.0)), Color32::WHITE);
            } else {
                icons::draw_close(ui.painter(), Rect::from_center_size(btn_rect.center(), Vec2::splat(14.0)), pal.muted);
            }
            if resp.clicked() {
                outcome = OptionsDialogOutcome::Cancel;
            }

            // ----------------------------------------------------------------
            // 2. Middle Content Area (.opmid: Nav + Main Page) (Height: 402px)
            // ----------------------------------------------------------------
            let mid_height = total_height - 36.0 - 52.0;
            let nav_width = 150.0;
            let main_width = total_width - nav_width;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;

                // --- Left Navigation Rail (150px) ---
                let (nav_rect, _) = ui.allocate_exact_size(Vec2::new(nav_width, mid_height), egui::Sense::hover());
                ui.painter().rect_filled(nav_rect, 0.0, pal.bg);
                ui.painter().rect_filled(
                    Rect::from_min_max(Pos2::new(nav_rect.right() - 1.0, nav_rect.top()), nav_rect.max),
                    0.0,
                    pal.line,
                );

                let nav_ui_rect = nav_rect.shrink2(Vec2::new(8.0, 8.0));
                ui.allocate_new_ui(
                    egui::UiBuilder::new()
                        .max_rect(nav_ui_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(0.0, 3.0);

                    for tab in [
                        OptionsTab::General,
                        OptionsTab::Downloads,
                        OptionsTab::Notifications,
                        OptionsTab::Network,
                    ] {
                        let is_active = state.active_tab == tab;
                        let label = tab.label();
                        let (btn_rect, btn_resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 32.0),
                            egui::Sense::click(),
                        );
                        let btn_resp = btn_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if btn_resp.clicked() {
                            state.active_tab = tab;
                        }

                        let hovered = btn_resp.hovered();
                        let bg_color = if is_active {
                            pal.accent_soft
                        } else if hovered {
                            pal.raise
                        } else {
                            Color32::TRANSPARENT
                        };

                        if bg_color != Color32::TRANSPARENT {
                            ui.painter().rect_filled(btn_rect, shapes::R_SM, bg_color);
                        }

                        let text_color = if is_active {
                            pal.accent
                        } else if hovered {
                            pal.text
                        } else {
                            pal.muted
                        };

                        ui.painter().text(
                            Pos2::new(btn_rect.left() + 12.0, btn_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.5),
                            text_color,
                        );
                    }
                });

                // Version text at bottom of nav
                let ver_text = format!("Cosmo Downloader v{}", env!("CARGO_PKG_VERSION"));
                ui.painter().text(
                    Pos2::new(nav_rect.left() + 12.0, nav_rect.bottom() - 14.0),
                    egui::Align2::LEFT_BOTTOM,
                    ver_text,
                    egui::FontId::proportional(11.0),
                    pal.muted,
                );

                // --- Right Main Content Area (.opmain) ---
                let (main_rect, _) = ui.allocate_exact_size(Vec2::new(main_width, mid_height), egui::Sense::hover());
                ui.painter().rect_filled(main_rect, 0.0, pal.surface);

                let main_padding = Vec2::new(22.0, 16.0);
                let main_content_rect = main_rect.shrink2(main_padding);

                ui.allocate_new_ui(
                    egui::UiBuilder::new()
                        .max_rect(main_content_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(state.active_tab.label())
                                        .strong()
                                        .size(16.0)
                                        .color(pal.text),
                                );
                                ui.add_space(10.0);

                                match state.active_tab {
                                    OptionsTab::General => {
                                        // Row 1: Startup
                                        render_setting_row(
                                            ui,
                                            pal,
                                            "Launch Cosmo on system startup",
                                            "Start when you sign in to Windows.",
                                            false,
                                            None,
                                            true,
                                            36.0,
                                            |ui, pal| {
                                                toggle_switch(ui, &mut state.startup, true, pal);
                                            },
                                        );

                                        // Row 2: Start minimized in tray
                                        let start_ok = state.startup && state.tray;
                                        let hint = if !state.startup {
                                            Some("Needs “Launch Cosmo on system startup”.")
                                        } else if !state.tray {
                                            Some("Needs the tray icon.")
                                        } else {
                                            None
                                        };
                                        render_setting_row(
                                            ui,
                                            pal,
                                            "Start minimized in tray",
                                            "Open without showing the main window.",
                                            !start_ok,
                                            hint,
                                            true,
                                            36.0,
                                            |ui, pal| {
                                                let mut val = state.minstart && start_ok;
                                                if toggle_switch(ui, &mut val, start_ok, pal).changed() && start_ok {
                                                    state.minstart = val;
                                                }
                                            },
                                        );

                                        // Row 3: Show icon in system tray
                                        render_setting_row(
                                            ui,
                                            pal,
                                            "Show icon in system tray",
                                            "Keep an icon near the clock.",
                                            false,
                                            None,
                                            true,
                                            36.0,
                                            |ui, pal| {
                                                toggle_switch(ui, &mut state.tray, true, pal);
                                            },
                                        );

                                        // Row 4: Minimize to tray on close
                                        let tray_ok = state.tray;
                                        let mintray_hint = if !tray_ok {
                                            Some("Needs the tray icon, so you can always get back.")
                                        } else {
                                            None
                                        };
                                        render_setting_row(
                                            ui,
                                            pal,
                                            "Minimize to tray on close",
                                            "Closing the window keeps downloads running.",
                                            !tray_ok,
                                            mintray_hint,
                                            false,
                                            36.0,
                                            |ui, pal| {
                                                let mut val = state.mintray && tray_ok;
                                                if toggle_switch(ui, &mut val, tray_ok, pal).changed() && tray_ok {
                                                    state.mintray = val;
                                                }
                                            },
                                        );
                                    }

                        OptionsTab::Downloads => {
                            // Row 1: Connections per download
                            render_setting_row(
                                ui,
                                pal,
                                "Connections per download",
                                "More is not always faster. 8 works well for most.",
                                false,
                                None,
                                true,
                                175.0,
                                |ui, pal| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("{}", state.threads))
                                                .strong()
                                                .size(13.0)
                                                .color(pal.text),
                                        );
                                        ui.add_space(4.0);
                                        ui.add(
                                            egui::Slider::new(&mut state.threads, 1..=16)
                                                .show_value(false),
                                        );
                                    });
                                },
                            );

                            // Row 2: Speed limit
                            render_setting_row(
                                ui,
                                pal,
                                "Speed limit",
                                "0 means unlimited.",
                                false,
                                None,
                                true,
                                175.0,
                                |ui, _pal| {
                                    ui.horizontal(|ui| {
                                        egui::ComboBox::from_id_salt("opt_speed_unit")
                                            .selected_text(state.unit.label())
                                            .width(72.0)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(&mut state.unit, SpeedUnit::Kbps, "KB/s");
                                                ui.selectable_value(&mut state.unit, SpeedUnit::Mbps, "MB/s");
                                            });
                                        ui.add_space(4.0);
                                        ui.add(
                                            egui::DragValue::new(&mut state.speed)
                                                .speed(1.0)
                                                .range(0..=1_000_000),
                                        );
                                    });
                                },
                            );

                            // Row 3: Default save folder
                            render_setting_row(
                                ui,
                                pal,
                                "Default save folder",
                                "Empty uses a folder per category.",
                                false,
                                None,
                                true,
                                230.0,
                                |ui, _pal| {
                                    ui.horizontal(|ui| {
                                        if ui.button("Browse").clicked() {
                                            let start_dir = if !state.folder.trim().is_empty() && Path::new(state.folder.trim()).exists() {
                                                Some(Path::new(state.folder.trim()))
                                            } else {
                                                None
                                            };
                                            if let Some(dir) = platform::pick_folder_dialog("Select Default Download Folder", start_dir) {
                                                state.folder = dir.to_string_lossy().to_string();
                                            }
                                        }
                                        ui.add_space(4.0);
                                        ui.add(
                                            egui::TextEdit::singleline(&mut state.folder)
                                                .hint_text("Category default")
                                                .desired_width(150.0),
                                        );
                                    });
                                },
                            );

                            // Row 4: Catch links from clipboard
                            render_setting_row(
                                ui,
                                pal,
                                "Catch links from clipboard",
                                "Offer to download when you copy a link.",
                                false,
                                None,
                                false,
                                36.0,
                                |ui, pal| {
                                    toggle_switch(ui, &mut state.clip, true, pal);
                                },
                            );
                        }

                        OptionsTab::Notifications => {
                            // Row 1: Desktop notifications
                            render_setting_row(
                                ui,
                                pal,
                                "Desktop notifications",
                                "Show a message when a download finishes or fails.",
                                false,
                                None,
                                true,
                                36.0,
                                |ui, pal| {
                                    toggle_switch(ui, &mut state.notif, true, pal);
                                },
                            );

                            // Row 2: Show download window
                            render_setting_row(
                                ui,
                                pal,
                                "Show the download window",
                                "Keep the progress and complete window open.",
                                false,
                                None,
                                true,
                                36.0,
                                |ui, pal| {
                                    toggle_switch(ui, &mut state.cwin, true, pal);
                                },
                            );

                            // Row 3: Open main window when download starts
                            render_setting_row(
                                ui,
                                pal,
                                "Open the main window when a download starts",
                                "Off keeps only the download window.",
                                false,
                                None,
                                false,
                                36.0,
                                |ui, pal| {
                                    toggle_switch(ui, &mut state.openmain, true, pal);
                                },
                            );

                            // Warning callout if both notif and cwin are off
                            if !state.notif && !state.cwin {
                                ui.add_space(14.0);
                                let warn_bg = warn_bg_color(pal);
                                let warn_c = warn_color(pal);

                                let (warn_rect, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 34.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(warn_rect, shapes::R_SM, warn_bg);
                                ui.painter().text(
                                    Pos2::new(warn_rect.left() + 12.0, warn_rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    "With both turned off, you won’t see anything when a download finishes.",
                                    egui::FontId::proportional(12.0),
                                    warn_c,
                                );
                            }
                        }

                        OptionsTab::Network => {
                            let is_valid = state.is_proxy_valid();

                            render_setting_row(
                                ui,
                                pal,
                                "Proxy",
                                "http://, https:// or socks5://  ·  empty = direct connection",
                                false,
                                None,
                                false,
                                220.0,
                                |ui, pal| {
                                    let text_edit = egui::TextEdit::singleline(&mut state.proxy)
                                        .hint_text("Direct connection")
                                        .desired_width(210.0);
                                    if !is_valid {
                                        ui.scope(|ui| {
                                            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.danger);
                                            ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.5_f32, pal.danger);
                                            ui.visuals_mut().widgets.active.bg_stroke = Stroke::new(1.5_f32, pal.danger);
                                            ui.add(text_edit);
                                        });
                                    } else {
                                        ui.add(text_edit);
                                    }
                                },
                            );

                            if !is_valid {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("Enter a valid address, for example http://127.0.0.1:8080")
                                        .size(12.0)
                                        .color(pal.danger),
                                );
                            }
                        }
                    }
                            });
                });
            });

            // ----------------------------------------------------------------
            // 3. Footer (.opft) (Height: 52px)
            // ----------------------------------------------------------------
            let (ft_rect, _) = ui.allocate_exact_size(Vec2::new(total_width, 52.0), egui::Sense::hover());
            let bot_rounding = eframe::egui::Rounding { nw: 0.0, ne: 0.0, sw: shapes::R.sw, se: shapes::R.se };
            ui.painter().rect_filled(ft_rect, bot_rounding, pal.surface);
            ui.painter().rect_filled(
                Rect::from_min_max(ft_rect.min, Pos2::new(ft_rect.right(), ft_rect.top() + 1.0)),
                0.0,
                pal.line,
            );

            let ft_ui_rect = ft_rect.shrink2(Vec2::new(16.0, 10.0));
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(ft_ui_rect), |ui| {
                ui.horizontal(|ui| {
                    // Left: Reset to defaults
                    let reset_btn = egui::Button::new(
                        RichText::new("Reset to defaults")
                            .size(12.5)
                            .color(pal.muted),
                    )
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(0.0, 30.0));

                    if ui.add(reset_btn).clicked() {
                        state.reset_to_defaults();
                    }

                    // Right: Cancel and Save
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let is_valid = state.is_proxy_valid();

                        // Save button
                        let save_btn = egui::Button::new(
                            RichText::new("Save")
                                .size(12.5)
                                .color(if is_valid { pal.on_accent } else { pal.on_accent.gamma_multiply(0.5) }),
                        )
                        .fill(if is_valid { pal.accent } else { pal.accent.gamma_multiply(0.4) })
                        .min_size(Vec2::new(76.0, 30.0))
                        .rounding(shapes::R_SM);

                        if ui.add_enabled(is_valid, save_btn).clicked() {
                            outcome = OptionsDialogOutcome::Save;
                        }

                        ui.add_space(8.0);

                        // Cancel button
                        let cancel_btn = egui::Button::new(
                            RichText::new("Cancel")
                                .size(12.5)
                                .color(pal.text),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0_f32, pal.line))
                        .min_size(Vec2::new(72.0, 30.0))
                        .rounding(shapes::R_SM);

                        if ui.add(cancel_btn).clicked() {
                            outcome = OptionsDialogOutcome::Cancel;
                        }
                    });
                });
            });
        },
    );

    outcome
}
