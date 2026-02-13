//! Tauri desktop application entry point.
//!
//! Wires Tauri commands, managed state, and the React frontend.

mod commands;
mod events;
mod state;

use tracing_subscriber::EnvFilter;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_call,
            commands::end_call,
            commands::get_peers,
            commands::get_stats,
            commands::set_muted,
            commands::list_audio_devices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    run();
}
