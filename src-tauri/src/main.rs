mod application;
mod business;
mod dns;
mod infrastructure;

use application::commands::{
    apply_home_configuration, clear_network_service_dns, enforce_system_dns, get_network_services,
    get_system_dns_config, invoke_api, update_system_dns_config,
};
use application::runtime::AppState;
use infrastructure::privileged_udp::PrivilegedUdpDaemonConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};

const TRAY_SHOW_ID: &str = "show";
const TRAY_QUIT_ID: &str = "quit";

struct DesktopLifecycle {
    quit_requested: AtomicBool,
    cleanup_in_progress: AtomicBool,
}

impl DesktopLifecycle {
    fn new() -> Self {
        Self {
            quit_requested: AtomicBool::new(false),
            cleanup_in_progress: AtomicBool::new(false),
        }
    }
}

fn main() -> anyhow::Result<()> {
    if let Some(forwarder) = PrivilegedUdpDaemonConfig::from_process_arguments()? {
        return forwarder.run();
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| show_main_window(app),
        ))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = tauri::async_runtime::block_on(AppState::initialize(&data_dir))?;
            app.manage(state);
            app.manage(DesktopLifecycle::new());
            create_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let lifecycle = window.state::<DesktopLifecycle>();
                if !lifecycle.quit_requested.load(Ordering::Acquire) {
                    api.prevent_close();
                    if let Err(error) = window.hide() {
                        tracing::error!("Failed to hide main window: {}", error);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            invoke_api,
            get_system_dns_config,
            get_network_services,
            update_system_dns_config,
            enforce_system_dns,
            clear_network_service_dns,
            apply_home_configuration
        ])
        .build(tauri::generate_context!())?;

    app.run(|app, event| {
        match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen {
                has_visible_windows: _,
                ..
            } => {
                // macOS sends Reopen when the Dock icon is activated after the
                // close button hid the main window. A hidden window is not
                // visible, so always restore it explicitly.
                show_main_window(app);
            }
            tauri::RunEvent::ExitRequested { api, .. } => {
                let lifecycle = app.state::<DesktopLifecycle>();
                if lifecycle.quit_requested.load(Ordering::Acquire) {
                    return;
                }
                api.prevent_exit();
                request_clean_exit(app);
            }
            _ => {}
        }
    });
    Ok(())
}

fn create_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, TRAY_SHOW_ID, "显示 DNS Hearth", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "退出并清空 DNS", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("DNS Hearth")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_SHOW_ID => show_main_window(app),
            TRAY_QUIT_ID => request_clean_exit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.show() {
            tracing::error!("Failed to show main window: {}", error);
            return;
        }
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn request_clean_exit(app: &AppHandle) {
    let lifecycle = app.state::<DesktopLifecycle>();
    if lifecycle.quit_requested.load(Ordering::Acquire)
        || lifecycle.cleanup_in_progress.swap(true, Ordering::AcqRel)
    {
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = app
            .state::<AppState>()
            .system_dns
            .clear_configured_services_for_exit()
            .await;
        let lifecycle = app.state::<DesktopLifecycle>();
        lifecycle
            .cleanup_in_progress
            .store(false, Ordering::Release);
        match result {
            Ok(()) => {
                lifecycle.quit_requested.store(true, Ordering::Release);
                app.exit(0);
            }
            Err(error) => {
                tracing::error!("Failed to clear system DNS before exit: {}", error);
                show_main_window(&app);
                let _ = app.emit("dns-exit-cleanup-failed", error.to_string());
            }
        }
    });
}
