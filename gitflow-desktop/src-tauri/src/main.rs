// Evita abrir uma janela de console extra no Windows em builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::check_git,
            commands::open_repository,
            commands::get_repository_status,
            commands::get_file_diff,
        ])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o GitFlow Desktop");
}
