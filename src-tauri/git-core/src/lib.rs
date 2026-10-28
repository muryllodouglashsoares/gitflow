//! `gitflow-git`: camada Git do GitFlow Desktop (consultas e operações locais predefinidas).
//!
//! Este crate não depende do Tauri. Ele executa o Git instalado no computador de forma
//! controlada (sem shell, com argumentos separados, tempo limite e ambiente endurecido)
//! e devolve modelos serializáveis para a interface.
//!
//! Consultas (`rev-parse`, `status`, `diff`, `for-each-ref`) e, a partir da Fase 2, as
//! operações locais de escrita **predefinidas**: criar branch, trocar de branch e criar commit
//! seletivo. A Fase 3 acrescenta o diagnóstico de sincronização e `fetch`, `pull` e `push`
//! predefinidos (módulo `remote`). A Fase 4 acrescenta a importação segura de ZIP (módulo `import`). Não existe execução de comandos arbitrários: cada operação monta seus próprios
//! argumentos, valida as entradas e confere o resultado real depois de executar.

pub mod branches;
pub mod commit;
pub mod diff;
pub mod error;
pub mod import;
pub mod lock;
pub mod remote;
pub mod repository;
pub mod runner;
pub mod status;

pub use branches::{
    check_branch_name, create_branch, list_branches, switch_branch, BranchEntry, BranchList,
    BranchNameCheck, CreateBranchRequest, CreatedBranch, SwitchBranchRequest, SwitchResult,
};
pub use commit::{
    create_commit, prepare_commit_review, CommitRequest, CommitResult, CommitReview,
    CommittedFile, ReviewFile, ReviewRequest,
};
pub use diff::{get_file_diff, DiffContent, DiffRequest, DiffTarget, FileDiff};
pub use error::{ErrorCode, GitError, Result};
pub use import::{
    apply_import, create_import_commit, find_import_operation, finish_import, inspect_zip,
    prepare_import_branch, push_import_branch, rollback_import, ApplyImportRequest, EntryAction,
    FinishImportRequest, ImportCommitRequest, ImportOperation, ImportOperationRequest, ImportPlan,
    ImportResult, ImportState, InspectRequest, PlanEntry, PlanSummary, PrepareBranchRequest,
    PreparedBranch, PushImportRequest, RollbackResult, ZipLayout,
};
pub use remote::{
    fetch_remote, get_sync_status, pull_branch, push_branch, FetchRequest, FetchResult,
    PullOutcome, PullRequest, PullResult, PullStrategy, PushRequest, PushResult, RemoteInfo,
    SyncState, SyncStatus, SyncTarget, UpstreamInfo,
};
pub use repository::{check_availability, open_repository, GitAvailability, RepositoryInfo};
pub use runner::Git;
pub use status::{get_status, RepositoryStatus};
