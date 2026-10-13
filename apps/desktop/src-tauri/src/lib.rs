//! The GPU Share desktop app: Share mode (host) and Use mode (client) in one window. The Rust
//! side owns everything real; the webview only reads two views (`host-view`, `client-view`)
//! and calls the commands in `commands.rs`.

mod commands;
mod state;
mod tray;

use std::time::Duration;

use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();
    // One copy at a time (two would fight over the ports); a second launch shows the first.
    // Development and the e2e script run several on purpose, each with its own data folder.
    if std::env::var_os("GPU_SHARE_DATA_DIR").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_window(app)));
    }
    builder
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(if cfg!(debug_assertions) { log::LevelFilter::Info } else { log::LevelFilter::Warn })
                .build(),
        )
        .setup(|app| {
            let state = state::AppState::init(app.handle())?;
            app.manage(state.clone());
            let tray = tray::create(app.handle(), state.host.is_some())?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // push views to the webview whenever they change
                let (mut last_host, mut last_client) = (String::new(), String::new());
                let mut last_pending = false;
                loop {
                    let view = state.host.as_ref().map(|h| h.core.view());
                    if let Some(v) = &view {
                        tray.update(v.sharing, &v.state);
                        // a friend is asking to connect: bring the window forward so the owner sees the code
                        let pending = v.pending.is_some();
                        if pending && !last_pending {
                            tray::show_window(&handle);
                        }
                        last_pending = pending;
                    }
                    let h = serde_json::to_string(&view).unwrap_or_default();
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
        .on_window_event(|window, event| {
            // On Windows the app lives in the tray (spec §6.1): closing the window hides it, and
            // sharing carries on. The Linux build is a client, so closing it quits.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if cfg!(windows) && window.app_handle().state::<std::sync::Arc<state::AppState>>().host.is_some() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::host_view,
            commands::host_set_sharing,
            commands::host_update_settings,
            commands::host_welcome_done,
            commands::host_model_toggle,
            commands::host_model_add,
            commands::host_model_remove,
            commands::host_model_set_default,
            commands::host_model_add_link,
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
