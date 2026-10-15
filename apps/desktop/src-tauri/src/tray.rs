//! The tray icon (`P7-T5`, spec §6.10): native on Windows, AppIndicator on Ubuntu. Its menu has
//! the one switch that matters, a way back to the window, and Quit.

use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;

pub struct Tray {
    toggle: Option<MenuItem<Wry>>,
    tray: tauri::tray::TrayIcon<Wry>,
    last: Mutex<(bool, String)>,
}

pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn create(app: &AppHandle, can_share: bool) -> tauri::Result<Arc<Tray>> {
    let toggle = can_share.then(|| MenuItem::with_id(app, "toggle", "Start sharing", true, None::<&str>)).transpose()?;
    let open = MenuItem::with_id(app, "open", "Open GPU Share", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit GPU Share", true, None::<&str>)?;
    let menu = match &toggle {
        Some(t) => Menu::with_items(app, &[t, &open, &quit])?,
        None => Menu::with_items(app, &[&open, &quit])?,
    };
    let tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("GPU Share")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "toggle" => {
                let state = app.state::<Arc<AppState>>().inner().clone();
                if let Some(h) = state.host.clone() {
                    tauri::async_runtime::spawn(async move {
                        h.core.set_sharing(!h.core.sharing());
                        h.tick().await;
                    });
                }
            }
            "open" => show_window(app),
            "quit" => {
                let app = app.clone();
                let state = app.state::<Arc<AppState>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(h) = &state.host {
                        h.core.set_sharing(false);
                        h.shutdown().await;
                    }
                    app.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(Arc::new(Tray { toggle, tray, last: Mutex::new((false, String::new())) }))
}

impl Tray {
    /// Keeps the menu's switch label and the tooltip in step with the host.
    pub fn update(&self, sharing: bool, state: &str) {
        let mut last = self.last.lock().unwrap();
        if *last == (sharing, state.to_string()) {
            return;
        }
        *last = (sharing, state.to_string());
        if let Some(t) = &self.toggle {
            let _ = t.set_text(if sharing { "Stop sharing" } else { "Start sharing" });
        }
        let label = match state {
            "available" => "Ready",
            "busy" => "In use",
            "paused" => "Paused",
            _ => "Off",
        };
        let _ = self.tray.set_tooltip(Some(format!("GPU Share: {label}")));
    }
}
