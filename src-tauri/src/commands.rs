//! Comandos Tauri expostos à interface. Todos são **somente leitura** e finos:
//! validam e delegam para o crate `gitflow-git`, executando o Git fora da thread principal.

use gitflow_git::{
    self as core, DiffRequest, ErrorCode, FileDiff, Git, GitAvailability, GitError,
    RepositoryInfo, RepositoryStatus,
};

/// Executa trabalho bloqueante (processos do Git) em uma thread dedicada,
/// mantendo a interface responsiva.
async fn blocking<T, F>(job: F) -> Result<T, GitError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, GitError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(job).await {
        Ok(result) => result,
        Err(err) => Err(GitError::new(
            ErrorCode::CommandFailed,
            "A operação foi interrompida inesperadamente.",
        )
        .with_details(err.to_string())),
    }
}

/// Verifica se o Git está instalado e executável.
#[tauri::command]
pub async fn check_git() -> Result<GitAvailability, GitError> {
    blocking(|| Ok(core::check_availability())).await
}

/// Valida a pasta escolhida e identifica o repositório (nome e raiz).
#[tauri::command]
pub async fn open_repository(path: String) -> Result<RepositoryInfo, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::open_repository(&git, &path)
    })
    .await
}

/// Branch atual, arquivos alterados e estado geral do repositório.
#[tauri::command]
pub async fn get_repository_status(repo_path: String) -> Result<RepositoryStatus, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::get_status(&git, &repo_path)
    })
    .await
}

/// Diferenças de um arquivo alterado (preparado, não preparado, não rastreado ou em conflito).
#[tauri::command]
pub async fn get_file_diff(request: DiffRequest) -> Result<FileDiff, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::get_file_diff(&git, &request)
    })
    .await
}
