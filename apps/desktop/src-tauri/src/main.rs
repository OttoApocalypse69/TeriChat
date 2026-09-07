#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Alpha 0 desktop shell. All chat logic lives in the Vite frontend
// (apps/desktop/src); this binary hosts the webview plus the HTTP plugin
// backend (API calls bypass webview CORS through Rust).
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .run(tauri::generate_context!())
        .expect("error while running TeriChat desktop");
}
