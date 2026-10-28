fn main() {
    // Registra explicitamente os comandos do aplicativo: a interface só pode chamar
    // os comandos listados aqui e concedidos em `capabilities/default.json`.
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "check_git",
                "open_repository",
                "get_repository_status",
                "get_file_diff",
                "list_branches",
                "check_branch_name",
                "create_branch",
                "switch_branch",
                "prepare_commit_review",
                "create_commit",
                "get_sync_status",
                "fetch_remote",
                "pull_branch",
                "push_branch",
                "inspect_zip",
                "prepare_import_branch",
                "apply_import",
                "rollback_import",
                "create_import_commit",
                "push_import_branch",
                "find_import_operation",
                "finish_import",
            ]),
        ),
    )
    .expect("falha ao executar o tauri-build");
}
