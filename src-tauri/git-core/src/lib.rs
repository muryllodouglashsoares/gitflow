//! `gitflow-git`: camada Git do GitFlow Desktop (consultas e operações locais predefinidas).
//!
//! Este crate não depende do Tauri. Ele executa o Git instalado no computador de forma
//! controlada (sem shell, com argumentos separados, tempo limite e ambiente endurecido)
//! e devolve modelos serializáveis para a interface.
//!
//! Consultas (`rev-parse`, `status`, `diff`, `for-each-ref`) e, a partir da Fase 2, as
//! operações locais de escrita **predefinidas**: criar branch, trocar de branch e criar commit
//! seletivo. Não existe execução de comandos arbitrários: cada operação monta seus próprios
//! argumentos, valida as entradas e confere o resultado real depois de executar.

pub mod branches;
pub mod commit;
pub mod diff;
pub mod error;
pub mod lock;
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
pub use repository::{check_availability, open_repository, GitAvailability, RepositoryInfo};
pub use runner::Git;
pub use status::{get_status, RepositoryStatus};
