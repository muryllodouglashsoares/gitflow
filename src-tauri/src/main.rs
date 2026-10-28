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
            commands::get_sync_status,
            commands::fetch_remote,
            commands::pull_branch,
            commands::push_branch,
            commands::inspect_zip,
            commands::prepare_import_branch,
            commands::apply_import,
            commands::rollback_import,
            commands::create_import_commit,
            commands::push_import_branch,
            commands::find_import_operation,
            commands::finish_import,
        ])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o GitFlow Desktop");
}
