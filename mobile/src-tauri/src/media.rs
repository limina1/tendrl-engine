//! Narration media plugin: the SPA mirrors its player state into Android's
//! media stack (foreground service + MediaSession + notification) so audio
//! keeps playing with the screen off and the lock screen gets transport
//! controls. Kotlin side: `gen/android/.../MediaPlugin.kt`, `MediaService.kt`,
//! `MediaBridge.kt`. The return path (system buttons → page) is a
//! `tendrl:media` CustomEvent evaluated straight into the WebView, so no
//! Tauri event plumbing is needed here.

use serde::Serialize;
use serde_json::Value;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[cfg(target_os = "android")]
struct Media<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StateArgs {
    playing: bool,
    title: String,
    subtitle: String,
    album: String,
    position: f64,
    duration: f64,
    rate: f64,
    has_next: bool,
    has_prev: bool,
}

#[derive(Debug, Serialize)]
struct EmptyArgs {}

#[cfg(target_os = "android")]
fn forward<R: Runtime, T: Serialize>(
    app: &tauri::AppHandle<R>,
    command: &str,
    args: T,
) -> Result<Value, String> {
    use tauri::Manager;
    let handle = app.state::<Media<R>>();
    handle
        .0
        .run_mobile_plugin::<Value>(command, args)
        .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "android"))]
fn unsupported() -> Result<Value, String> {
    Err("background media is Android-only".into())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn set_state<R: Runtime>(
    app: tauri::AppHandle<R>,
    playing: bool,
    title: String,
    subtitle: Option<String>,
    album: Option<String>,
    position: Option<f64>,
    duration: Option<f64>,
    rate: Option<f64>,
    has_next: Option<bool>,
    has_prev: Option<bool>,
) -> Result<Value, String> {
    let args = StateArgs {
        playing,
        title,
        subtitle: subtitle.unwrap_or_default(),
        album: album.unwrap_or_default(),
        position: position.unwrap_or(0.0),
        duration: duration.unwrap_or(0.0),
        rate: rate.unwrap_or(1.0),
        has_next: has_next.unwrap_or(false),
        has_prev: has_prev.unwrap_or(false),
    };
    #[cfg(target_os = "android")]
    return forward(&app, "setState", args);
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, args);
        unsupported()
    }
}

#[tauri::command]
async fn stop<R: Runtime>(app: tauri::AppHandle<R>) -> Result<Value, String> {
    #[cfg(target_os = "android")]
    return forward(&app, "stop", EmptyArgs {});
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        unsupported()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("media")
        .invoke_handler(tauri::generate_handler![set_state, stop])
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    _api.register_android_plugin("io.github.limina1.tendrl", "MediaPlugin")?;
                _app.manage(Media(handle));
            }
            Ok(())
        })
        .build()
}
