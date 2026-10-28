//! Registro persistente (diário) de uma importação.
//!
//! Cada operação tem uma pasta própria **dentro do diretório Git do repositório**
//! (`<git-dir>/gitflow-import/<id-da-operação>/`), que:
//! - nunca aparece no diretório de trabalho nem no `git status`, e jamais entra em um commit;
//! - é privada do repositório (não há pasta temporária compartilhada);
//! - guarda `journal.json` (o que foi planejado e em que etapa está), `staged/` (conteúdo já
//!   validado, antes de tocar no repositório) e `backup/` (cópia dos arquivos substituídos).
//!
//! O diário contém apenas caminhos relativos, somas SHA-256 e nomes de branch: nenhum conteúdo de
//! arquivo, credencial ou segredo. Os backups só são apagados quando a operação termina
//! (`finish_import`) ou é desfeita (`rollback_import`).

use crate::{
    error::{ErrorCode, GitError, Result},
    runner::{Git, RunOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const JOURNAL_FILE: &str = "journal.json";
const JOURNAL_VERSION: u32 = 1;
pub(crate) const OPS_DIR: &str = "gitflow-import";

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Etapa em que a operação se encontra (persistida).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImportState {
    /// Gravando arquivos no repositório (ou interrompida durante a gravação).
    Applying,
    /// A gravação falhou no meio: parte dos arquivos pode ter sido aplicada.
    Partial,
    /// Todos os arquivos autorizados foram aplicados; falta revisar e commitar.
    Applied,
    /// O commit foi criado; falta (opcionalmente) enviar ao remoto.
    Committed,
    /// A importação foi desfeita (arquivos restaurados).
    RolledBack,
    /// Operação encerrada.
    Finished,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EntryAction {
    Add,
    Replace,
    Identical,
    Blocked,
    Ignored,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub path: String,
    pub action: EntryAction,
    pub new_sha256: String,
    pub old_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub version: u32,
    pub op_id: String,
    pub created_at: u64,
    pub repo_path: String,
    pub work_branch: String,
    pub base_branch: String,
    pub base_oid: String,
    pub zip_path: String,
    pub zip_sha256: String,
    pub source_prefix: String,
    pub state: ImportState,
    pub entries: Vec<JournalEntry>,
    pub commit_oid: Option<String>,
}

impl Journal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        op_id: String,
        repo_path: String,
        work_branch: String,
        base_branch: String,
        base_oid: String,
        zip_path: String,
        zip_sha256: String,
        source_prefix: String,
        entries: Vec<JournalEntry>,
    ) -> Self {
        Journal {
            version: JOURNAL_VERSION,
            op_id,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            repo_path,
            work_branch,
            base_branch,
            base_oid,
            zip_path,
            zip_sha256,
            source_prefix,
            state: ImportState::Applying,
            entries,
            commit_oid: None,
        }
    }

    pub fn allows(&self, path: &str) -> bool {
        self.entries.iter().any(|e| e.path == path)
    }
}

/// Identificador novo e imprevisível o bastante para nomear a pasta da operação.
pub fn new_op_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(seq.to_le_bytes());
    let digest = hasher.finalize();
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// O identificador vira nome de pasta: só aceita exatamente o formato gerado por `new_op_id`.
pub fn validate_op_id(op_id: &str) -> Result<()> {
    if op_id.len() == 16
        && op_id
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(GitError::invalid_argument(
            "O identificador da operação de importação é inválido.",
        ))
    }
}

fn io_error(what: &str, err: std::io::Error) -> GitError {
    GitError::new(ErrorCode::PathNotAccessible, what.to_string()).with_details(err.to_string())
}

/// Diretório Git absoluto do repositório (funciona também com worktrees vinculados).
pub fn git_dir(git: &Git, root: &Path) -> Result<PathBuf> {
    let opts = RunOptions {
        timeout: Duration::from_secs(15),
        max_stdout: 64 * 1024,
        ..RunOptions::default()
    };
    let out = git.run(Some(root), &["rev-parse", "--absolute-git-dir"], &opts)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.trim_end_matches(['\n', '\r']);
    if line.is_empty() {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git não informou o diretório de metadados do repositório.",
        ));
    }
    Ok(PathBuf::from(line))
}

pub fn ops_root(git_dir: &Path) -> PathBuf {
    git_dir.join(OPS_DIR)
}

pub fn op_dir(git_dir: &Path, op_id: &str) -> Result<PathBuf> {
    validate_op_id(op_id)?;
    Ok(ops_root(git_dir).join(op_id))
}

pub fn staged_dir(op_dir: &Path) -> PathBuf {
    op_dir.join("staged")
}

pub fn backup_dir(op_dir: &Path) -> PathBuf {
    op_dir.join("backup")
}

/// Grava o diário de forma atômica (arquivo temporário + renomeação).
pub fn save(op_dir: &Path, journal: &Journal) -> Result<()> {
    fs::create_dir_all(op_dir).map_err(|e| {
        io_error(
            "Não foi possível criar a pasta de trabalho da importação.",
            e,
        )
    })?;
    let json = serde_json::to_vec_pretty(journal).map_err(|e| {
        GitError::new(
            ErrorCode::UnexpectedOutput,
            "Não foi possível registrar o andamento da importação.",
        )
        .with_details(e.to_string())
    })?;
    let tmp = op_dir.join(format!("{JOURNAL_FILE}.tmp"));
    fs::write(&tmp, json)
        .map_err(|e| io_error("Não foi possível gravar o registro da importação.", e))?;
    fs::rename(&tmp, op_dir.join(JOURNAL_FILE))
        .map_err(|e| io_error("Não foi possível gravar o registro da importação.", e))
}

pub fn load(op_dir: &Path) -> Result<Journal> {
    let bytes = fs::read(op_dir.join(JOURNAL_FILE)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GitError::new(
                ErrorCode::PathNotFound,
                "Não existe registro desta operação de importação (ela foi encerrada ou não pertence a este repositório).",
            )
        } else {
            io_error("Não foi possível ler o registro da importação.", e)
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|e| {
        GitError::new(
            ErrorCode::UnexpectedOutput,
            "O registro da importação está ilegível. Verifique o repositório manualmente.",
        )
        .with_details(e.to_string())
    })
}

/// Carrega o diário e confere que pertence ao repositório informado.
pub fn load_for(git_dir: &Path, root: &Path, op_id: &str) -> Result<(PathBuf, Journal)> {
    let dir = op_dir(git_dir, op_id)?;
    let journal = load(&dir)?;
    if journal.op_id != op_id || Path::new(&journal.repo_path) != root {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            "Esta operação de importação pertence a outro repositório.",
        ));
    }
    Ok((dir, journal))
}

/// Todos os diários legíveis do repositório (mais recente primeiro).
pub fn list(git_dir: &Path) -> Vec<Journal> {
    let mut out: Vec<Journal> = Vec::new();
    let Ok(entries) = fs::read_dir(ops_root(git_dir)) else {
        return out;
    };
    for entry in entries.flatten() {
        if let Ok(journal) = load(&entry.path()) {
            out.push(journal);
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.op_id.cmp(&a.op_id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_ids_are_unique_and_valid() {
        let a = new_op_id();
        let b = new_op_id();
        assert_ne!(a, b);
        assert!(validate_op_id(&a).is_ok());
    }

    #[test]
    fn op_id_validation_rejects_path_tricks() {
        for bad in [
            "",
            "../x",
            "..",
            "0123456789abcde",
            "0123456789abcdeF",
            "0123456789abcde/",
            "zzzzzzzzzzzzzzzz",
        ] {
            assert!(validate_op_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn journal_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let op = dir.path().join("op");
        let mut journal = Journal::new(
            new_op_id(),
            "/repo".into(),
            "import/x".into(),
            "main".into(),
            "abc".into(),
            "/tmp/a.zip".into(),
            "00".into(),
            "".into(),
            vec![JournalEntry {
                path: "a.txt".into(),
                action: EntryAction::Add,
                new_sha256: "11".into(),
                old_sha256: None,
            }],
        );
        save(&op, &journal).unwrap();
        journal.state = ImportState::Applied;
        save(&op, &journal).unwrap();
        let loaded = load(&op).unwrap();
        assert_eq!(loaded.state, ImportState::Applied);
        assert!(loaded.allows("a.txt"));
        assert!(!loaded.allows("b.txt"));
    }
}
