#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Alpha 0 desktop shell. All chat logic lives in the Vite frontend
// (apps/desktop/src); this binary only hosts the webview.
fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running TeriChat desktop");
}
