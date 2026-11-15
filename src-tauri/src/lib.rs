mod bank_commands;
mod bank_config;
mod errors;
mod scheduler;
mod state;

use serde::Serialize;
use tauri::Manager;

/// Basic application metadata exposed to the UI.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

fn build_app_info() -> AppInfo {
    AppInfo {
        name: "Pocket Ledger".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
fn app_info() -> AppInfo {
    build_app_info()
}

pub fn run() {
    tauri::Builder::default()
        // Used from Rust only, to open the bank authorization URL in the system browser.
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = state::AppState::init(&data_dir).map_err(|e| e.to_string())?;
            app.manage(state);
            scheduler::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            bank_commands::bank_overview,
            bank_commands::bank_connect,
            bank_commands::bank_sync_now,
            bank_commands::bank_disconnect,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pocket Ledger");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_reports_name_and_version() {
        let info = build_app_info();
        assert_eq!(info.name, "Pocket Ledger");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }
}
