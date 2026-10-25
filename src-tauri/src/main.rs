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
            commands::list_branches,
            commands::check_branch_name,
            commands::create_branch,
            commands::switch_branch,
            commands::prepare_commit_review,
            commands::create_commit,
        ])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o GitFlow Desktop");
}
