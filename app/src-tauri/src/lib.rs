//! Iron-Fleet desktop client (J.A.R.V.I.S.) — Tauri shell.
//!
//! Stage 3 of the build order: a read-only Fleet Dashboard. The client holds
//! no fleet state of its own (CLAUDE.md: "Clients are thin and
//! interchangeable... they hold no fleet state") — every command in
//! `commands.rs` reads live from the control plane on each call.

mod commands;
mod config;

use config::ControlPlaneConfig;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

pub struct AppState {
    pub http: reqwest::Client,
    pub config_path: PathBuf,
    pub config: Mutex<Option<ControlPlaneConfig>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("control-plane.json");
            let config =
                ControlPlaneConfig::from_env().or_else(|| ControlPlaneConfig::load(&config_path));
            let http = reqwest::Client::builder()
                .user_agent(concat!("iron-fleet-app/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(30))
                .build()?;
            app.manage(AppState {
                http,
                config_path,
                config: Mutex::new(config),
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
