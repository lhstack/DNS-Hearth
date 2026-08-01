mod application;
mod business;
mod dns;
mod infrastructure;

use application::commands::{
    apply_home_configuration, enforce_system_dns, get_network_services, get_system_dns_config,
    invoke_api, update_system_dns_config,
};
use application::runtime::AppState;
use infrastructure::privileged_udp::PrivilegedUdpDaemonConfig;
use tauri::Manager;

fn main() -> anyhow::Result<()> {
    if let Some(forwarder) = PrivilegedUdpDaemonConfig::from_process_arguments()? {
        return forwarder.run();
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            },
        ))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = tauri::async_runtime::block_on(AppState::initialize(&data_dir))?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            invoke_api,
            get_system_dns_config,
            get_network_services,
            update_system_dns_config,
            enforce_system_dns,
            apply_home_configuration
        ])
        .run(tauri::generate_context!())
        .map_err(Into::into)
}
