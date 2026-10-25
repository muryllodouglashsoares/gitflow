//! `gitflow-git`: camada de **leitura** do Git usada pelo GitFlow Desktop.
//!
//! Este crate não depende do Tauri. Ele executa o Git instalado no computador de forma
//! controlada (sem shell, com argumentos separados, tempo limite e ambiente endurecido)
//! e devolve modelos serializáveis para a interface.
//!
//! Nesta fase não existe nenhuma operação de escrita: apenas consultas
//! (`rev-parse`, `status`, `diff`).

pub mod diff;
pub mod error;
pub mod repository;
pub mod runner;
pub mod status;

pub use diff::{get_file_diff, DiffContent, DiffRequest, DiffTarget, FileDiff};
pub use error::{ErrorCode, GitError, Result};
pub use repository::{check_availability, open_repository, GitAvailability, RepositoryInfo};
pub use runner::Git;
pub use status::{get_status, RepositoryStatus};
