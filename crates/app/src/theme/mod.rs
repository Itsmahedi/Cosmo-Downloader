pub mod colors;
pub mod shapes;

pub use colors::*;
pub use shapes::*;

use eframe::egui::{self, FontFamily, FontId, Stroke, TextStyle};

pub fn configure_visuals(ctx: &egui::Context, is_dark: bool) {
    let pal = colors::palette(is_dark);
    let mut visuals = if is_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.window_rounding = shapes::R;
    visuals.window_fill = pal.surface;
    visuals.window_stroke = Stroke::new(1.0_f32, pal.line);
    visuals.panel_fill = pal.surface;
    visuals.extreme_bg_color = pal.bg;
    visuals.faint_bg_color = pal.raise;
    visuals.window_shadow = egui::Shadow::NONE;
    visuals.popup_shadow = egui::Shadow::NONE;
    visuals.override_text_color = None;

    visuals.widgets.noninteractive.bg_fill = pal.surface;
    visuals.widgets.noninteractive.weak_bg_fill = pal.surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal.line);
    visuals.widgets.noninteractive.rounding = shapes::R_SM;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, pal.text);

    visuals.widgets.inactive.bg_fill = pal.raise;
    visuals.widgets.inactive.weak_bg_fill = pal.raise;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.line);
    visuals.widgets.inactive.rounding = shapes::R_SM;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, pal.text);

    visuals.widgets.hovered.bg_fill = pal.raise;
    visuals.widgets.hovered.weak_bg_fill = pal.raise;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.widgets.hovered.rounding = shapes::R_SM;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, pal.text);

    visuals.widgets.active.bg_fill = pal.accent_soft;
    visuals.widgets.active.weak_bg_fill = pal.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.widgets.active.rounding = shapes::R_SM;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, pal.accent);

    visuals.widgets.open.bg_fill = pal.raise;
    visuals.widgets.open.weak_bg_fill = pal.raise;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, pal.line);
    visuals.widgets.open.rounding = shapes::R_SM;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, pal.text);

    visuals.selection.bg_fill = pal.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0_f32, pal.accent);

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(16.0, FontFamily::Proportional)),
        (TextStyle::Name("title".into()), FontId::new(16.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
    ]
    .into();

    ctx.set_style(style);
    ctx.set_visuals(visuals);
}

pub fn configure_m3_visuals(ctx: &egui::Context) {
    configure_visuals(ctx, true);
}

pub const SCRIPT_INDIC: u8 = 1 << 0;
pub const SCRIPT_CJK: u8 = 1 << 1;
pub const SCRIPT_JAPANESE: u8 = 1 << 2;
pub const SCRIPT_KOREAN: u8 = 1 << 3;
pub const SCRIPT_EMOJI: u8 = 1 << 4;

static LOADED_SCRIPTS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static CURRENT_FONTS: std::sync::Mutex<Option<egui::FontDefinitions>> = std::sync::Mutex::new(None);

fn font_candidate_dirs() -> Vec<std::path::PathBuf> {
    let mut candidate_dirs: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(root) = std::env::var("SystemRoot") {
            candidate_dirs.push(std::path::Path::new(&root).join("Fonts"));
        }
        if let Ok(windir) = std::env::var("windir") {
            candidate_dirs.push(std::path::Path::new(&windir).join("Fonts"));
        }
        candidate_dirs.push(std::path::PathBuf::from("C:\\Windows\\Fonts"));
    }
    #[cfg(not(windows))]
    {
        candidate_dirs.push(std::path::PathBuf::from("/mnt/c/Windows/Fonts"));
        candidate_dirs.push(std::path::PathBuf::from("/usr/share/fonts"));
    }
    candidate_dirs
}

fn try_load_single_font(family_name: &str, file_candidates: &[&str], index: u32, fonts: &mut egui::FontDefinitions) -> bool {
    let candidate_dirs = font_candidate_dirs();
    for fonts_dir in &candidate_dirs {
        for filename in file_candidates {
            let font_path = fonts_dir.join(filename);
            if font_path.exists() {
                if let Ok(data) = std::fs::read(&font_path) {
                    let mut font_data = egui::FontData::from_owned(data);
                    font_data.index = index;
                    fonts.font_data.insert(
                        family_name.to_string(),
                        font_data,
                    );
                    if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                        if !prop.contains(&family_name.to_string()) {
                            prop.push(family_name.to_string());
                        }
                    }
                    if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                        if !mono.contains(&family_name.to_string()) {
                            mono.push(family_name.to_string());
                        }
                    }
                    return true;
                }
            }
        }
    }
    false
}

/// Configures initial base UI font definitions (Segoe UI, Segoe UI Symbol).
/// Fallback scripts (Nirmala UI, Microsoft YaHei UI, Yu Gothic UI, Malgun Gothic, Segoe UI Emoji)
/// are loaded lazily on demand when glyphs from those scripts are shown, minimizing memory.
pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Base startup fonts: Segoe UI + Segoe UI Symbol + Unicode/Fallback scripts
    let base_specs: &[(&str, &[&str], u32)] = &[
        ("Segoe UI", &["segoeui.ttf"], 0),
        ("Segoe UI Symbol", &["seguisym.ttf"], 0),
        ("Segoe UI Historic", &["seguihis.ttf"], 0),
        ("Nirmala UI", &["Nirmala.ttc", "NirmalaB.ttc", "NirmalaS.ttc", "nirmala.ttf", "vrinda.ttf", "vrindab.ttf", "shonar.ttf", "shonarb.ttf"], 0),
        ("Vrinda", &["vrinda.ttf", "vrindab.ttf"], 0),
        ("Shonar Bangla", &["shonar.ttf", "shonarb.ttf"], 0),
        ("Microsoft YaHei UI", &["msyh.ttc", "msyhbd.ttc", "msyhl.ttc", "msyh.ttf"], 0),
        ("Yu Gothic UI", &["YuGothR.ttc", "YuGothM.ttc", "yugoth.ttc"], 0),
        ("Malgun Gothic", &["malgun.ttf", "malgunbd.ttf"], 0),
        ("Segoe UI Emoji", &["seguiemj.ttf"], 0),
    ];

    for (family_name, file_candidates, idx) in base_specs {
        try_load_single_font(family_name, file_candidates, *idx, &mut fonts);
    }

    if let Ok(mut lock) = CURRENT_FONTS.lock() {
        *lock = Some(fonts.clone());
    }
    LOADED_SCRIPTS.store(0, std::sync::atomic::Ordering::Relaxed);

    ctx.set_fonts(fonts);
}

/// Lazily loads fallback fonts on demand only when glyphs from those scripts are encountered.
pub fn ensure_fonts_for_text(ctx: &egui::Context, text: &str) {
    let mut needed = 0u8;
    for c in text.chars() {
        let u = c as u32;
        if (0x0900..=0x0DFF).contains(&u) {
            needed |= SCRIPT_INDIC;
        } else if (0x4E00..=0x9FFF).contains(&u) || (0x3400..=0x4DBF).contains(&u) {
            needed |= SCRIPT_CJK;
        } else if (0x3040..=0x30FF).contains(&u) || (0x31F0..=0x31FF).contains(&u) {
            needed |= SCRIPT_JAPANESE;
        } else if (0xAC00..=0xD7AF).contains(&u) || (0x1100..=0x11FF).contains(&u) || (0x3130..=0x318F).contains(&u) {
            needed |= SCRIPT_KOREAN;
        } else if (0x1F300..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u) {
            needed |= SCRIPT_EMOJI;
        }
    }

    if needed == 0 {
        return;
    }

    let loaded = LOADED_SCRIPTS.load(std::sync::atomic::Ordering::Relaxed);
    if (needed & !loaded) == 0 {
        return;
    }

    let mut lock = match CURRENT_FONTS.lock() {
        Ok(l) => l,
        Err(_) => return,
    };

    let mut fonts = lock.take().unwrap_or_else(egui::FontDefinitions::default);
    let mut new_loaded = loaded;

    if (needed & SCRIPT_INDIC) != 0 && (loaded & SCRIPT_INDIC) == 0 {
        if try_load_single_font("Nirmala UI", &["Nirmala.ttc", "NirmalaB.ttc", "NirmalaS.ttc"], 0, &mut fonts) {
            new_loaded |= SCRIPT_INDIC;
        }
    }
    if (needed & SCRIPT_CJK) != 0 && (loaded & SCRIPT_CJK) == 0 {
        if try_load_single_font("Microsoft YaHei UI", &["msyh.ttc", "msyhbd.ttc", "msyhl.ttc"], 0, &mut fonts) {
            new_loaded |= SCRIPT_CJK;
        }
    }
    if (needed & SCRIPT_JAPANESE) != 0 && (loaded & SCRIPT_JAPANESE) == 0 {
        if try_load_single_font("Yu Gothic UI", &["YuGothR.ttc", "YuGothM.ttc", "yugoth.ttc"], 0, &mut fonts) {
            new_loaded |= SCRIPT_JAPANESE;
        }
    }
    if (needed & SCRIPT_KOREAN) != 0 && (loaded & SCRIPT_KOREAN) == 0 {
        if try_load_single_font("Malgun Gothic", &["malgun.ttf", "malgunbd.ttf"], 0, &mut fonts) {
            new_loaded |= SCRIPT_KOREAN;
        }
    }
    if (needed & SCRIPT_EMOJI) != 0 && (loaded & SCRIPT_EMOJI) == 0 {
        if try_load_single_font("Segoe UI Emoji", &["seguiemj.ttf"], 0, &mut fonts) {
            new_loaded |= SCRIPT_EMOJI;
        }
    }

    LOADED_SCRIPTS.store(new_loaded, std::sync::atomic::Ordering::Relaxed);
    ctx.set_fonts(fonts.clone());
    *lock = Some(fonts);
}
