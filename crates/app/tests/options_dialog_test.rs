use eframe::egui::{self, Context};
use rustget_app::{
    dialogs::settings::{
        is_proxy_valid, render_options_window_content, OptionsDialogOutcome, OptionsDialogState,
        OptionsTab, SpeedUnit,
    },
    theme,
};

#[test]
fn test_options_tab_switching_and_retention() {
    let mut state = OptionsDialogState::default();
    assert_eq!(state.active_tab, OptionsTab::General);

    state.active_tab = OptionsTab::Downloads;
    assert_eq!(state.active_tab, OptionsTab::Downloads);

    state.active_tab = OptionsTab::Notifications;
    assert_eq!(state.active_tab, OptionsTab::Notifications);

    state.active_tab = OptionsTab::Network;
    assert_eq!(state.active_tab, OptionsTab::Network);
}

#[test]
fn test_options_general_4_options_dependencies() {
    let mut state = OptionsDialogState::default();
    assert_eq!(state.startup, false);
    assert_eq!(state.minstart, false);
    assert_eq!(state.tray, true);
    assert_eq!(state.mintray, true);

    // If startup is false, minstart is not enabled
    assert!(!state.is_start_minimized_enabled());
    assert!(state.is_minimize_to_tray_enabled());

    // If startup is true and tray is true, minstart is enabled
    state.startup = true;
    assert!(state.is_start_minimized_enabled());

    // If tray is turned off, neither minstart nor mintray are enabled
    state.tray = false;
    assert!(!state.is_start_minimized_enabled());
    assert!(!state.is_minimize_to_tray_enabled());
}

#[test]
fn test_options_proxy_validation() {
    assert!(is_proxy_valid(""));
    assert!(is_proxy_valid("   "));
    assert!(is_proxy_valid("http://127.0.0.1:8080"));
    assert!(is_proxy_valid("https://proxy.example.com:443"));
    assert!(is_proxy_valid("socks5://10.0.0.1:1080"));
    assert!(is_proxy_valid("socks5h://127.0.0.1:9050"));

    // Invalid proxies
    assert!(!is_proxy_valid("ftp://127.0.0.1:21"));
    assert!(!is_proxy_valid("http://"));
    assert!(!is_proxy_valid("https://proxy with spaces:8080"));
    assert!(!is_proxy_valid("http://localhost:invalidport"));
    assert!(!is_proxy_valid("http://localhost:0"));
}

#[test]
fn test_options_reset_to_defaults_preserves_active_tab() {
    let mut state = OptionsDialogState::default();
    state.active_tab = OptionsTab::Downloads;
    state.threads = 16;
    state.speed = 500;
    state.unit = SpeedUnit::Mbps;
    state.folder = "C:\\CustomPath".to_string();

    state.reset_to_defaults();

    // Default values restored
    assert_eq!(state.threads, 8);
    assert_eq!(state.speed, 0);
    assert_eq!(state.folder, "");
    // Active tab preserved
    assert_eq!(state.active_tab, OptionsTab::Downloads);
}

#[test]
fn test_options_render_all_tabs_without_panic() {
    let ctx = Context::default();
    let pal = theme::palette(true);

    for tab in [
        OptionsTab::General,
        OptionsTab::Downloads,
        OptionsTab::Notifications,
        OptionsTab::Network,
    ] {
        let mut state = OptionsDialogState::default();
        state.active_tab = tab;

        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let outcome = render_options_window_content(ui, &pal, None, &mut state);
                assert_eq!(outcome, OptionsDialogOutcome::None);
            });
        });
    }
}
