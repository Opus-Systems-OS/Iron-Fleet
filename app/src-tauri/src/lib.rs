//! Iron-Fleet desktop client (J.A.R.V.I.S.) — Tauri shell.
//!
//! Stage 3 of the build order: a read-only Fleet Dashboard. The client holds
//! no fleet state of its own (CLAUDE.md: "Clients are thin and
//! interchangeable... they hold no fleet state") — every command in
//! `commands.rs` reads live from the control plane on each call.

mod commands;
mod config;
mod speech;
mod stream;

use config::ControlPlaneConfig;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

pub struct AppState {
    pub http: reqwest::Client,
    /// For the session event stream only: no total timeout, or every stream
    /// would be cut off at 30s. Connect timeout still applies.
    pub stream_http: reqwest::Client,
    pub config_path: PathBuf,
    pub config: Mutex<Option<ControlPlaneConfig>>,
    /// Live watches by slot (`"fleet"`: the selected row; `"voice"`: the
    /// Jarvis conversation). Re-watching a slot aborts its previous task.
    pub watch: Mutex<HashMap<String, commands::Watch>>,
    /// Speech input (macOS: the speech thread; elsewhere: a stub).
    pub speech: speech::SpeechHandle,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("control-plane.json");
            let config =
                ControlPlaneConfig::from_env().or_else(|| ControlPlaneConfig::load(&config_path));
            const UA: &str = concat!("iron-fleet-app/", env!("CARGO_PKG_VERSION"));
            let http = reqwest::Client::builder()
                .user_agent(UA)
                .timeout(Duration::from_secs(30))
                .build()?;
            let stream_http = reqwest::Client::builder()
                .user_agent(UA)
                .connect_timeout(Duration::from_secs(10))
                .build()?;
            app.manage(AppState {
                http,
                stream_http,
                config_path,
                config: Mutex::new(config),
                watch: Mutex::new(HashMap::new()),
                speech: speech::SpeechHandle::new(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connection_status,
            commands::set_connection,
            commands::list_agents,
            commands::list_sessions,
            commands::get_session,
            commands::create_session,
            commands::send_session_event,
            commands::interrupt_session,
            commands::get_usage,
            commands::get_inference_models,
            commands::watch_session,
            commands::unwatch_session,
            speech::speech_support,
            speech::speech_start,
            speech::speech_stop,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
