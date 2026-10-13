//! The GPU Share desktop app: Share mode (host) and Use mode (client) in one window. The Rust
//! side owns everything real; the webview only reads two views (`host-view`, `client-view`)
//! and calls the commands in `commands.rs`.

mod commands;
mod state;

use std::time::Duration;

use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(if cfg!(debug_assertions) { log::LevelFilter::Info } else { log::LevelFilter::Warn })
                .build(),
        )
        .setup(|app| {
            let state = state::AppState::init(app.handle())?;
            app.manage(state.clone());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // push views to the webview whenever they change
                let (mut last_host, mut last_client) = (String::new(), String::new());
                loop {
                    let h = serde_json::to_string(&state.host.as_ref().map(|h| h.core.view())).unwrap_or_default();
                    if h != last_host {
                        let _ = handle.emit("host-view", &h);
                        last_host = h;
                    }
                    let c = serde_json::to_string(&state.client.view()).unwrap_or_default();
                    if c != last_client {
                        let _ = handle.emit("client-view", &c);
                        last_client = c;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::host_view,
            commands::host_set_sharing,
            commands::host_update_settings,
            commands::host_welcome_done,
            commands::host_model_toggle,
            commands::host_model_add,
            commands::host_model_remove,
            commands::host_unpair,
            commands::host_pair_allow,
            commands::host_pair_deny,
            commands::app_quit,
            commands::client_view,
            commands::client_ask_connect,
            commands::client_add_by_ip,
            commands::client_rescan,
            commands::client_open_chat,
            commands::client_send,
            commands::client_retry,
            commands::client_stop,
            commands::client_set_model,
            commands::client_forget,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
