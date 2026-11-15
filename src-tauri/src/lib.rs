use serde::Serialize;

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
        .invoke_handler(tauri::generate_handler![app_info])
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
