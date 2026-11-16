mod backup_commands;
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

/// Debug builds only: `POCKET_LEDGER_DATA_DIR` keeps the database and config of a
/// development run inside the project (e.g. its ignored `tmp/`) instead of the
/// user's app data directory. Release builds always use the app data directory.
fn data_dir_override() -> Option<std::path::PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os("POCKET_LEDGER_DATA_DIR")
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
    } else {
        None
    }
}

pub fn run() {
    tauri::Builder::default()
        // Used from Rust only, to open the bank authorization URL in the system browser.
        .plugin(tauri_plugin_opener::init())
        // Used from Rust only (backup Save/Open dialogs). No dialog permission is granted
        // to the WebView in capabilities/.
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = data_dir_override().map_or_else(|| app.path().app_data_dir(), Ok)?;
            let state = state::AppState::init(&data_dir).map_err(|e| e.to_string())?;
            app.manage(state);
            scheduler::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            backup_commands::app_mode,
            backup_commands::backup_status,
            backup_commands::backup_check_passphrase,
            backup_commands::backup_create,
            backup_commands::backup_pick_file,
            backup_commands::backup_restore,
            backup_commands::recovery_unlock,
            backup_commands::recovery_restore,
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
