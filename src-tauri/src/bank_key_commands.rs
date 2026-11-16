//! Tauri commands for the Enable Banking private key location (Ajustes > Conexión
//! bancaria). The UI only ever receives the file name and whether it exists: never the
//! full path, never the content. The file is not opened here; it is read only when a
//! request is signed. The native dialog is driven from Rust (no dialog permission in the
//! WebView).

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::bank_config::{key_status, update_key_path, validate_key_path, KeyStatus};
use crate::errors::{CommandError, ErrorCode};
use crate::state::AppState;

fn pick_dialog(app: &AppHandle) -> Option<PathBuf> {
    app.dialog()
        .file()
        .add_filter("Clave privada (.pem)", &["pem"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
}

#[tauri::command]
pub fn bank_key_status(state: State<'_, AppState>) -> KeyStatus {
    key_status(&state.data_dir)
}

/// Lets the owner choose the `.pem` and stores its path in `enable-banking.json`.
/// Cancelling the dialog returns the current status unchanged.
#[tauri::command]
pub async fn bank_key_pick(app: AppHandle) -> Result<KeyStatus, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Some(path) = pick_dialog(&app) {
            validate_key_path(&path)?;
            update_key_path(&state.data_dir, Some(&path))?;
            log::info!("bank key path updated");
        }
        Ok(key_status(&state.data_dir))
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal))?
}

#[tauri::command]
pub fn bank_key_clear(state: State<'_, AppState>) -> Result<KeyStatus, CommandError> {
    update_key_path(&state.data_dir, None)?;
    log::info!("bank key path cleared");
    Ok(key_status(&state.data_dir))
}
