//! Tauri commands: the actions in `HostBackend` / `ClientBackend` (src/lib/types.ts).
//! Views are pushed as events; these only change things.

use std::sync::Arc;

use host::runtime::HostRuntime;
use tauri::State;

use crate::state::AppState;

type S<'a> = State<'a, Arc<AppState>>;
type R<T = ()> = Result<T, String>;

fn host(s: &AppState) -> R<&Arc<HostRuntime>> {
    s.host.as_ref().ok_or_else(|| "Sharing isn't available on this computer.".to_string())
}

// ------------------------------------------------------------------ host

#[tauri::command]
pub fn host_view(s: S) -> Option<host::core::HostView> {
    s.host.as_ref().map(|h| h.core.view())
}

#[tauri::command]
pub async fn host_set_sharing(s: S<'_>, on: bool) -> R {
    let h = host(&s)?;
    h.core.set_sharing(on);
    h.tick().await; // apply now rather than on the next tick
    Ok(())
}

/// Merges `patch` (a partial `HostSettings`) into the settings.
#[tauri::command]
pub fn host_update_settings(s: S, patch: serde_json::Value) -> R {
    let h = host(&s)?;
    let mut current = serde_json::to_value(h.core.settings()).map_err(|e| e.to_string())?;
    let (Some(cur), Some(p)) = (current.as_object_mut(), patch.as_object()) else {
        return Err("bad settings".into());
    };
    for (k, v) in p {
        if k != "sharing" {
            cur.insert(k.clone(), v.clone());
        }
    }
    let next: host::settings::HostSettings = serde_json::from_value(current).map_err(|e| e.to_string())?;
    h.core.update_settings(|st| {
        let sharing = st.sharing; // only the switch changes this
        *st = next;
        st.sharing = sharing;
    });
    Ok(())
}

#[tauri::command]
pub fn host_welcome_done(s: S) -> R {
    host(&s)?.core.update_settings(|st| st.welcomed = true);
    Ok(())
}

#[tauri::command]
pub fn host_model_toggle(s: S, id: String) -> R {
    host(&s)?.core.update_settings(|st| {
        if let Some(i) = st.disabled.iter().position(|d| *d == id) {
            st.disabled.remove(i);
        } else {
            st.disabled.push(id);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn host_model_add(s: S, id: String) -> R {
    let _ = id;
    host(&s)?.core.toast("Not yet", "Downloading models arrives with the model library.");
    Ok(())
}

#[tauri::command]
pub fn host_model_remove(s: S, id: String) -> R {
    let _ = id;
    host(&s)?.core.toast("Not yet", "Removing models arrives with the model library.");
    Ok(())
}

#[tauri::command]
pub fn host_unpair(s: S, id: String) -> R {
    let h = host(&s)?;
    h.core.store.unpair(&id).map_err(|e| e.to_string())?;
    h.core.drop_device(&id);
    Ok(())
}

#[tauri::command]
pub fn host_pair_allow(s: S) -> R {
    host(&s)?;
    Ok(())
}

#[tauri::command]
pub fn host_pair_deny(s: S) -> R {
    host(&s)?;
    Ok(())
}

#[tauri::command]
pub async fn app_quit(app: tauri::AppHandle, s: S<'_>) -> R {
    if let Some(h) = &s.host {
        h.core.set_sharing(false);
        h.shutdown().await;
    }
    app.exit(0);
    Ok(())
}

// ------------------------------------------------------------------ client

#[tauri::command]
pub fn client_view(s: S) -> client::runtime::ClientView {
    s.client.view()
}

#[tauri::command]
pub async fn client_ask_connect(s: S<'_>, host_id: String) -> R {
    let _ = (s, host_id);
    Err("Pairing arrives in a later build.".into())
}

/// Resolves to an error message, or null when the host was found.
#[tauri::command]
pub async fn client_add_by_ip(s: S<'_>, address: String) -> R<Option<String>> {
    match s.client.add_by_ip(&address).await {
        Ok(_) => Ok(None),
        Err(e) => Ok(Some(e)),
    }
}

#[tauri::command]
pub fn client_rescan(s: S) {
    s.discovery.rescan();
}

#[tauri::command]
pub fn client_open_chat(s: S, host_id: String) {
    s.client.open_chat(&host_id);
}

#[tauri::command]
pub fn client_send(s: S, text: String) {
    s.client.send(&text);
}

#[tauri::command]
pub fn client_stop(s: S) {
    s.client.stop();
}

#[tauri::command]
pub fn client_set_model(s: S, id: String) {
    s.client.set_model(&id);
}

#[tauri::command]
pub fn client_forget(s: S, host_id: String) {
    s.client.forget(&host_id);
}
