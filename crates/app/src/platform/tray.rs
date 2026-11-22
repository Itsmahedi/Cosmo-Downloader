use eframe::egui;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tray_icon::{
    menu::{Menu, MenuItem},
    TrayIcon, TrayIconBuilder,
};

thread_local! {
    static ACTIVE_TRAY_ICON: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

pub fn destroy_system_tray() {
    ACTIVE_TRAY_ICON.with(|t| {
        *t.borrow_mut() = None;
    });
}

pub fn has_system_tray() -> bool {
    ACTIVE_TRAY_ICON.with(|t| t.borrow().is_some())
}

static TRAY_TX: Mutex<Option<std::sync::mpsc::Sender<String>>> = Mutex::new(None);
static TRAY_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);
static HANDLER_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn build_system_tray(
    tray_tx: &std::sync::mpsc::Sender<String>,
    egui_ctx: egui::Context,
    icon_rgba: &[u8],
    app_title: &str,
) -> bool {
    #[cfg(target_os = "linux")]
    {
        if gtk::init().is_err() {
            return false;
        }
    }

    destroy_system_tray();

    if let Ok(mut tx_guard) = TRAY_TX.lock() {
        *tx_guard = Some(tray_tx.clone());
    }
    if let Ok(mut ctx_guard) = TRAY_CTX.lock() {
        *ctx_guard = Some(egui_ctx.clone());
    }

    let tray_menu = Menu::new();
    let open_item = MenuItem::with_id("tray_open", "Open", true, None);
    let pause_all_item = MenuItem::with_id("tray_pause_all", "Pause all", true, None);
    let resume_all_item = MenuItem::with_id("tray_resume_all", "Resume all", true, None);
    let exit_item = MenuItem::with_id("tray_exit", "Exit", true, None);
    let _ = tray_menu.append(&open_item);
    let _ = tray_menu.append(&pause_all_item);
    let _ = tray_menu.append(&resume_all_item);
    let _ = tray_menu.append(&exit_item);

    if !HANDLER_INITIALIZED.swap(true, Ordering::SeqCst) {
        tray_icon::menu::MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
            let id = event.id.as_ref();
            if id == "tray_open" || id.contains("open") {
                #[cfg(windows)]
                {
                    crate::platform::windows::grant_foreground_permission();
                    crate::platform::show_main_window();
                }
                if let Ok(guard) = TRAY_TX.lock() {
                    if let Some(ref tx) = *guard {
                        let _ = tx.send("tray_menu_open".to_string());
                    }
                }
                if let Ok(guard) = TRAY_CTX.lock() {
                    if let Some(ref ctx) = *guard {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                        ctx.request_repaint();
                    }
                }
            } else if id == "tray_pause_all" || id.contains("pause") {
                if let Ok(guard) = TRAY_TX.lock() {
                    if let Some(ref tx) = *guard {
                        let _ = tx.send("pause_all".to_string());
                    }
                }
                if let Some(q) = crate::app::get_global_queue() {
                    let q = std::sync::Arc::clone(&q);
                    tokio::spawn(async move {
                        let active = q.db().list_downloads(None).await.unwrap_or_default();
                        for d in active {
                            if d.status == "downloading" || d.status == "queued" {
                                let _ = q.pause(&d.id).await;
                            }
                        }
                    });
                }
                if let Ok(guard) = TRAY_CTX.lock() {
                    if let Some(ref ctx) = *guard {
                        ctx.request_repaint();
                    }
                }
            } else if id == "tray_resume_all" || id.contains("resume") {
                if let Ok(guard) = TRAY_TX.lock() {
                    if let Some(ref tx) = *guard {
                        let _ = tx.send("resume_all".to_string());
                    }
                }
                if let Some(q) = crate::app::get_global_queue() {
                    let q = std::sync::Arc::clone(&q);
                    tokio::spawn(async move {
                        let active = q.db().list_downloads(None).await.unwrap_or_default();
                        for d in active {
                            if d.status == "paused" || d.status == "failed" {
                                let _ = q.resume(&d.id).await;
                            }
                        }
                    });
                }
                if let Ok(guard) = TRAY_CTX.lock() {
                    if let Some(ref ctx) = *guard {
                        ctx.request_repaint();
                    }
                }
            } else if id == "tray_exit" || id.contains("exit") {
                crate::platform::log_daemon("Tray menu exit clicked -> direct graceful exit");
                crate::app::direct_graceful_shutdown();
            }
        }));

        tray_icon::TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| {
            match event {
                tray_icon::TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. }
                | tray_icon::TrayIconEvent::DoubleClick { button: tray_icon::MouseButton::Left, .. } => {
                    #[cfg(windows)]
                    {
                        crate::platform::windows::grant_foreground_permission();
                        crate::platform::show_main_window();
                    }
                    if let Ok(guard) = TRAY_TX.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send("tray_click".to_string());
                        }
                    }
                    if let Ok(guard) = TRAY_CTX.lock() {
                        if let Some(ref ctx) = *guard {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                            ctx.request_repaint();
                        }
                    }
                }
                _ => {}
            }
        }));
    }

    let tray_icon_img = tray_icon::Icon::from_rgba(icon_rgba.to_vec(), 32, 32).ok();
    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip(app_title);
    if let Some(ref icon) = tray_icon_img {
        builder = builder.with_icon(icon.clone());
    }
    let tray = builder.build().ok();
    let success = tray.is_some();
    ACTIVE_TRAY_ICON.with(|holder| {
        *holder.borrow_mut() = tray;
    });
    success
}
