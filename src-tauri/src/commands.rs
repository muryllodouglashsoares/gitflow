//! Comandos Tauri expostos à interface. São finos: validam e delegam para o crate `gitflow-git`,
//! executando o Git fora da thread principal.
//!
//! Leitura: `check_git`, `open_repository`, `get_repository_status`, `get_file_diff`,
//! `list_branches`, `check_branch_name`, `prepare_commit_review`.
//! Escrita local (Fase 2, cada uma uma operação predefinida): `create_branch`, `switch_branch`,
//! `create_commit`.
//! Sincronização remota (Fase 3): `get_sync_status` (somente leitura, sem rede) e as operações
//! predefinidas `fetch_remote`, `pull_branch` e `push_branch`.
//! Importação de ZIP (Fase 4): `inspect_zip`, `prepare_import_branch`, `apply_import`,
//! `rollback_import`, `create_import_commit`, `push_import_branch`, `find_import_operation` e
//! `finish_import`. O backend revalida tudo (caminhos, plano, estado do repositório); a interface
//! nunca é a fronteira de confiança. O envio reutiliza o `push_branch` da Fase 3.
//! Nenhum comando aceita texto de comando Git vindo da interface.

use gitflow_git::{
    self as core, ApplyImportRequest, BranchList, BranchNameCheck, CommitRequest, CommitResult,
    CommitReview, CreateBranchRequest, CreatedBranch, DiffRequest, ErrorCode, FetchRequest,
    FetchResult, FileDiff, FinishImportRequest, Git, GitAvailability, GitError,
    ImportCommitRequest, ImportOperation, ImportPlan, ImportResult, InspectRequest,
    PrepareBranchRequest, PreparedBranch, PullRequest, PullResult, PushImportRequest, PushRequest,
    PushResult, RepositoryInfo, RepositoryStatus, ReviewRequest, RollbackResult,
    SwitchBranchRequest, SwitchResult, SyncStatus,
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

/// Branches locais (nunca referências remotas) e branch atual.
#[tauri::command]
pub async fn list_branches(repo_path: String) -> Result<BranchList, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::list_branches(&git, &repo_path)
    })
    .await
}

/// Verifica (somente leitura) se um nome de branch pode ser criado.
#[tauri::command]
pub async fn check_branch_name(repo_path: String, name: String) -> Result<BranchNameCheck, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::check_branch_name(&git, &repo_path, &name)
    })
    .await
}

/// Cria uma branch local a partir de uma branch-base. **Não** troca de branch.
#[tauri::command]
pub async fn create_branch(request: CreateBranchRequest) -> Result<CreatedBranch, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::create_branch(&git, &request)
    })
    .await
}

/// Troca de branch somente quando não há alterações locais em risco. Nunca força a troca.
#[tauri::command]
pub async fn switch_branch(request: SwitchBranchRequest) -> Result<SwitchResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::switch_branch(&git, &request)
    })
    .await
}

/// Valida os arquivos selecionados e devolve o que será registrado (somente leitura).
#[tauri::command]
pub async fn prepare_commit_review(request: ReviewRequest) -> Result<CommitReview, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::prepare_commit_review(&git, &request)
    })
    .await
}

/// Cria um commit local apenas com os arquivos selecionados. Nunca executa `push`.
#[tauri::command]
pub async fn create_commit(request: CommitRequest) -> Result<CommitResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::create_commit(&git, &request)
    })
    .await
}

/// Remotos, upstream e situação de sincronização da branch atual. Somente leitura; não usa a rede.
#[tauri::command]
pub async fn get_sync_status(repo_path: String) -> Result<SyncStatus, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::get_sync_status(&git, &repo_path)
    })
    .await
}

/// Atualiza as referências de um remoto (`git fetch`). Não altera arquivos nem integra commits.
#[tauri::command]
pub async fn fetch_remote(request: FetchRequest) -> Result<FetchResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::fetch_remote(&git, &request)
    })
    .await
}

/// Baixa e integra as alterações do upstream. Bloqueado com alterações locais, conflitos ou
/// operação em andamento; nunca força nem descarta trabalho local.
#[tauri::command]
pub async fn pull_branch(request: PullRequest) -> Result<PullResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::pull_branch(&git, &request)
    })
    .await
}

/// Envia os commits da branch atual ao seu upstream. Nunca força, nunca cria upstream.
#[tauri::command]
pub async fn push_branch(request: PushRequest) -> Result<PushResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::push_branch(&git, &request)
    })
    .await
}

// ---------------------------------------------------------------------------
// Fase 4 — importação de ZIP
// ---------------------------------------------------------------------------

/// Inspeciona o ZIP **sem extrair nada** e devolve o plano de importação (somente leitura).
#[tauri::command]
pub async fn inspect_zip(request: InspectRequest) -> Result<ImportPlan, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::inspect_zip(&git, &request)
    })
    .await
}

/// Cria a branch de trabalho a partir da branch-base e troca para ela (após recalcular o plano).
#[tauri::command]
pub async fn prepare_import_branch(
    request: PrepareBranchRequest,
) -> Result<PreparedBranch, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::prepare_import_branch(&git, &request)
    })
    .await
}

/// Grava os arquivos do plano confirmado (arquivos novos e substituições autorizadas).
#[tauri::command]
pub async fn apply_import(request: ApplyImportRequest) -> Result<ImportResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::apply_import(&git, &request)
    })
    .await
}

/// Desfaz uma importação aplicada (ou parcial) e ainda sem commit, preservando edições posteriores.
#[tauri::command]
pub async fn rollback_import(request: FinishImportRequest) -> Result<RollbackResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::rollback_import(&git, &request)
    })
    .await
}

/// Cria o commit da importação, restrito aos arquivos que pertencem à operação.
#[tauri::command]
pub async fn create_import_commit(request: ImportCommitRequest) -> Result<CommitResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::create_import_commit(&git, &request)
    })
    .await
}

/// Envia o commit da importação reutilizando o `push_branch` da Fase 3.
#[tauri::command]
pub async fn push_import_branch(request: PushImportRequest) -> Result<PushResult, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::push_import_branch(&git, &request)
    })
    .await
}

/// Operação de importação ainda não encerrada neste repositório (permite retomar).
#[tauri::command]
pub async fn find_import_operation(repo_path: String) -> Result<Option<ImportOperation>, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::find_import_operation(&git, &repo_path)
    })
    .await
}

/// Encerra a operação (após o commit ou a reversão) e remove temporários e backups.
#[tauri::command]
pub async fn finish_import(request: FinishImportRequest) -> Result<ImportOperation, GitError> {
    blocking(move || {
        let git = Git::locate()?;
        core::finish_import(&git, &request)
    })
    .await
}
