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
            ]),
        ),
    )
    .expect("falha ao executar o tauri-build");
}
