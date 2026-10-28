//! Gravação controlada da importação e sua reversão.
//!
//! Ordem de segurança de `apply_import`:
//!
//! 1. confere o estado do repositório (limpo, sem operação em andamento, na branch de trabalho,
//!    no mesmo commit da branch-base) e **recalcula o plano**; qualquer diferença em relação ao
//!    que o usuário confirmou aborta antes de gravar;
//! 2. **fase A (staging)**: extrai somente os arquivos autorizados para
//!    `<git-dir>/gitflow-import/<id>/staged/`, conferindo a soma SHA-256 de cada um contra o
//!    plano, e copia para `backup/` os arquivos que serão substituídos. Nada no repositório foi
//!    tocado até aqui; se algo falhar, a pasta da operação é removida;
//! 3. **fase B (gravação)**: para cada arquivo, valida de novo o destino (sem links simbólicos,
//!    sem sobrescrever o que não foi autorizado), grava em um arquivo temporário vizinho e o
//!    coloca no lugar (`hard_link` para arquivos novos — falha se o destino apareceu —, e
//!    renomeação para substituições). Uma falha no meio **não é desfeita automaticamente**: o
//!    resultado parcial é informado e `rollback_import` restaura o estado anterior com
//!    segurança.
//!
//! A reversão não depende de um registro de progresso: decide por conteúdo. Um arquivo só é
//! removido/restaurado se o conteúdo atual for exatamente o que a importação gravou; se o
//! usuário o editou depois, ele é preservado e listado.

use super::{
    base_branch_oid, build_plan, dest_path, ensure_plan_unchanged, hash_file, hex, journal,
    repo_blockers, ImportLimits, ImportOperationRequest, PlanParams,
};
use crate::{
    branches::head_oid,
    error::{ErrorCode, GitError, Result},
    lock::RepoGuard,
    repository::require_repository_root,
    runner::Git,
    status::get_status,
};
use journal::{EntryAction, ImportState, Journal, JournalEntry};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

// ----------------------------------------------------------------------------
// Modelos
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyImportRequest {
    pub repo_path: String,
    pub zip_path: String,
    #[serde(default)]
    pub source_prefix: String,
    pub base_branch: String,
    pub work_branch: String,
    /// Identificador do plano que o usuário confirmou.
    pub plan_id: String,
    /// Arquivos do plano que o usuário optou por NÃO importar.
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    /// Confirmação explícita de que arquivos existentes serão substituídos.
    #[serde(default)]
    pub confirm_overwrites: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppliedFile {
    pub path: String,
    pub action: EntryAction,
    /// O Git enxerga alteração neste arquivo (se não, ele não poderá ser commitado).
    pub visible_in_git: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub op_id: String,
    /// `applied` (tudo gravado) ou `partial` (a gravação falhou no meio).
    pub state: ImportState,
    pub repo_path: String,
    pub work_branch: String,
    pub applied: Vec<AppliedFile>,
    pub failure: Option<ImportFailure>,
    /// Arquivos autorizados que não chegaram a ser gravados.
    pub not_attempted: Vec<String>,
    pub identical_skipped: usize,
    /// Orientação de recuperação, quando o resultado é parcial.
    pub recovery: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackResult {
    pub op_id: String,
    pub state: ImportState,
    /// Arquivos substituídos que voltaram ao conteúdo original.
    pub restored: Vec<String>,
    /// Arquivos novos que foram removidos.
    pub removed: Vec<String>,
    /// Arquivos que NÃO foram tocados porque o conteúdo atual não é o gravado pela importação.
    pub kept_modified: Vec<String>,
    /// Problemas ao restaurar (por exemplo, backup ausente).
    pub problems: Vec<String>,
}

/// Opções internas (pontos de injeção usados nos testes).
#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    pub limits: ImportLimits,
    /// Simula uma falha de gravação depois de N arquivos (somente para testes).
    #[doc(hidden)]
    pub fail_after_files: Option<usize>,
}

fn io_err(what: &str, err: io::Error) -> GitError {
    GitError::new(ErrorCode::PathNotAccessible, what.to_string()).with_details(err.to_string())
}

// ----------------------------------------------------------------------------
// Importação
// ----------------------------------------------------------------------------

pub fn apply_import(git: &Git, request: &ApplyImportRequest) -> Result<ImportResult> {
    apply_import_with(git, request, &ApplyOptions::default())
}

pub fn apply_import_with(
    git: &Git,
    request: &ApplyImportRequest,
    options: &ApplyOptions,
) -> Result<ImportResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;
    let root_text = root.to_string_lossy().into_owned();

    if request.work_branch == request.base_branch {
        return Err(GitError::invalid_argument(
            "A branch de trabalho precisa ser diferente da branch-base.",
        ));
    }

    // 1) Estado do repositório, agora.
    let status = get_status(git, &root_text)?;
    let blockers = repo_blockers(&status, &request.work_branch, "branch de trabalho");
    if let Some(first) = blockers.first() {
        return Err(GitError::blocked(first.clone()).with_details(blockers.join("\n")));
    }
    let base_oid = base_branch_oid(git, &root_text, &request.base_branch)?;
    if head_oid(git, &root)?.as_deref() != Some(base_oid.as_str()) {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            format!(
                "A branch de trabalho “{}” não está no mesmo commit da branch-base “{}”. \
                 Prepare a branch de importação novamente.",
                request.work_branch, request.base_branch
            ),
        ));
    }

    // 2) Plano recalculado a partir do disco e comparado com o confirmado.
    let params = PlanParams {
        root: &root,
        zip_path: &request.zip_path,
        source_prefix: &request.source_prefix,
        base_branch: &request.base_branch,
        base_oid: &base_oid,
    };
    let (plan, mut archive) = build_plan(git, &params, &options.limits)?;
    ensure_plan_unchanged(&plan, &request.plan_id)?;

    // 3) Exclusões e autorização.
    let excluded: HashSet<&str> = request.excluded_paths.iter().map(String::as_str).collect();
    for path in &excluded {
        let known = plan.entries.iter().any(|e| {
            e.path == *path && matches!(e.action, EntryAction::Add | EntryAction::Replace)
        });
        if !known {
            return Err(GitError::invalid_argument(format!(
                "“{path}” não é um arquivo a importar neste plano e não pode ser excluído dele."
            )));
        }
    }
    let authorized: Vec<_> = plan
        .entries
        .iter()
        .filter(|e| matches!(e.action, EntryAction::Add | EntryAction::Replace))
        .filter(|e| !excluded.contains(e.path.as_str()))
        .collect();
    if authorized.is_empty() {
        return Err(GitError::invalid_argument(
            "Nenhum arquivo foi autorizado para importação.",
        ));
    }
    let replaces: Vec<&str> = authorized
        .iter()
        .filter(|e| e.action == EntryAction::Replace)
        .map(|e| e.path.as_str())
        .collect();
    if !replaces.is_empty() && !request.confirm_overwrites {
        return Err(GitError::blocked(format!(
            "{} arquivo(s) existente(s) seriam substituídos e isso não foi confirmado.",
            replaces.len()
        )));
    }

    // 4) Pasta da operação + diário (antes de qualquer escrita).
    let git_dir = journal::git_dir(git, &root)?;
    let op_id = journal::new_op_id();
    let op_dir = journal::op_dir(&git_dir, &op_id)?;
    let staged = journal::staged_dir(&op_dir);
    let backup = journal::backup_dir(&op_dir);

    let entries: Vec<JournalEntry> = authorized
        .iter()
        .map(|e| JournalEntry {
            path: e.path.clone(),
            action: e.action,
            new_sha256: e.new_sha256.clone().unwrap_or_default(),
            old_sha256: e.old_sha256.clone(),
        })
        .collect();
    let mut record = Journal::new(
        op_id.clone(),
        root_text.clone(),
        request.work_branch.clone(),
        request.base_branch.clone(),
        base_oid,
        request.zip_path.clone(),
        plan.zip_sha256.clone(),
        plan.source_prefix.clone(),
        entries.clone(),
    );
    journal::save(&op_dir, &record)?;

    // 5) Fase A: staging + backups. Falhou? Nada foi tocado no repositório.
    let identical_skipped = plan.summary.identical;
    if let Err(err) = stage_all(
        &mut archive,
        &authorized,
        &root,
        &staged,
        &backup,
        &options.limits,
    ) {
        let _ = fs::remove_dir_all(&op_dir);
        return Err(err);
    }

    // 6) Fase B: gravação no repositório.
    let mut applied_paths: Vec<usize> = Vec::new();
    let mut failure: Option<ImportFailure> = None;
    for (i, entry) in entries.iter().enumerate() {
        let outcome = if options.fail_after_files == Some(i) {
            Err("falha simulada (teste)".to_string())
        } else {
            place_file(&root, entry, &staged, &op_id, i)
        };
        match outcome {
            Ok(()) => applied_paths.push(i),
            Err(message) => {
                failure = Some(ImportFailure {
                    path: entry.path.clone(),
                    message,
                });
                break;
            }
        }
    }

    // 7) Resultado real, conferido no Git.
    let state_after = get_status(git, &root_text).ok();
    let visible = |path: &str| {
        state_after
            .as_ref()
            .map(|s| s.files.iter().any(|f| f.path == path))
            .unwrap_or(false)
    };
    let applied: Vec<AppliedFile> = applied_paths
        .iter()
        .map(|&i| AppliedFile {
            path: entries[i].path.clone(),
            action: entries[i].action,
            visible_in_git: visible(&entries[i].path),
        })
        .collect();
    let not_attempted: Vec<String> = entries
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            !applied_paths.contains(i)
                && failure.as_ref().map(|f| f.path.as_str()) != Some(entries[*i].path.as_str())
        })
        .map(|(_, e)| e.path.clone())
        .collect();

    record.state = if failure.is_some() {
        ImportState::Partial
    } else {
        ImportState::Applied
    };
    // Se o diário não puder ser atualizado, o resultado continua sendo informado (os arquivos já
    // foram gravados) e a falha de registro é apontada na recuperação.
    let journal_saved = journal::save(&op_dir, &record).is_ok();
    if failure.is_none() {
        let _ = fs::remove_dir_all(&staged);
    }

    let recovery = failure.as_ref().map(|f| {
        format!(
            "A gravação parou em “{}”: {}. {} arquivo(s) já foram gravados e {} não foram tentados. \
             Nada foi commitado. Use “Desfazer importação” para restaurar o estado anterior com os backups \
             (arquivos que você tenha editado depois são preservados){}.",
            f.path,
            f.message,
            applied_paths.len(),
            not_attempted.len(),
            if journal_saved { "" } else { ". O registro da operação não pôde ser atualizado; confira o repositório com `git status`" }
        )
    });

    Ok(ImportResult {
        op_id,
        state: record.state,
        repo_path: root_text,
        work_branch: request.work_branch.clone(),
        applied,
        failure,
        not_attempted,
        identical_skipped,
        recovery,
    })
}

/// Extrai (com conferência de integridade) e faz backup. Não escreve no repositório.
fn stage_all(
    archive: &mut ZipArchive<File>,
    authorized: &[&super::PlanEntry],
    root: &Path,
    staged: &Path,
    backup: &Path,
    limits: &ImportLimits,
) -> Result<()> {
    let changed = || {
        GitError::new(
            ErrorCode::StaleReview,
            "O conteúdo do ZIP ou dos arquivos de destino mudou desde a prévia. Nada foi alterado: \
             inspecione o pacote novamente.",
        )
    };
    for entry in authorized {
        let expected = entry.new_sha256.as_deref().unwrap_or_default();
        let target = join_rel(staged, &entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                io_err(
                    "Não foi possível preparar a área temporária da importação.",
                    e,
                )
            })?;
        }

        let mut zipped = archive.by_index(entry.zip_index).map_err(|_| changed())?;
        if zipped.name() != entry.zip_name {
            return Err(changed());
        }
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| {
                io_err(
                    "Não foi possível gravar na área temporária da importação.",
                    e,
                )
            })?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut len = 0u64;
        loop {
            let n = zipped.read(&mut buf).map_err(|_| changed())?;
            if n == 0 {
                break;
            }
            len += n as u64;
            if len > limits.max_file_bytes {
                return Err(changed());
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| {
                io_err(
                    "Não foi possível gravar na área temporária da importação.",
                    e,
                )
            })?;
        }
        out.sync_all().map_err(|e| {
            io_err(
                "Não foi possível gravar na área temporária da importação.",
                e,
            )
        })?;
        if hex(&hasher.finalize()) != expected {
            return Err(changed());
        }
        drop(zipped);

        if entry.action == EntryAction::Replace {
            let source = dest_path(root, &entry.path);
            let copy = join_rel(backup, &entry.path);
            if let Some(parent) = copy.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    io_err(
                        "Não foi possível criar o backup dos arquivos substituídos.",
                        e,
                    )
                })?;
            }
            fs::copy(&source, &copy).map_err(|e| {
                io_err(
                    "Não foi possível criar o backup de um arquivo que seria substituído.",
                    e,
                )
            })?;
            let backed_up = hash_file(&copy)
                .map_err(|e| io_err("Não foi possível conferir o backup de um arquivo.", e))?;
            if Some(backed_up.as_str()) != entry.old_sha256.as_deref() {
                return Err(changed());
            }
        }
    }
    Ok(())
}

fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for component in rel.split('/') {
        path.push(component);
    }
    path
}

/// Cria as pastas que faltam, recusando links simbólicos e arquivos no lugar de pastas.
fn ensure_parent_dirs(root: &Path, rel: &str) -> std::result::Result<(), String> {
    let components: Vec<&str> = rel.split('/').collect();
    let mut current = root.to_path_buf();
    for component in &components[..components.len() - 1] {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("uma pasta do caminho virou um link simbólico".into())
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err("existe um arquivo onde deveria haver uma pasta".into()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|e| format!("não foi possível criar a pasta: {e}"))?;
            }
            Err(err) => return Err(format!("não foi possível examinar a pasta: {err}")),
        }
    }
    Ok(())
}

/// Coloca um arquivo validado no repositório. Erro = nada foi substituído por este arquivo.
fn place_file(
    root: &Path,
    entry: &JournalEntry,
    staged: &Path,
    op_id: &str,
    seq: usize,
) -> std::result::Result<(), String> {
    ensure_parent_dirs(root, &entry.path)?;
    let dest = dest_path(root, &entry.path);
    let source = join_rel(staged, &entry.path);

    let existing_permissions = match entry.action {
        EntryAction::Add => {
            match fs::symlink_metadata(&dest) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(
                        "o arquivo apareceu depois da inspeção e não será sobrescrito".into(),
                    )
                }
                Err(err) => return Err(format!("não foi possível examinar o destino: {err}")),
            }
            None
        }
        EntryAction::Replace => {
            let meta = fs::symlink_metadata(&dest)
                .map_err(|e| format!("não foi possível examinar o arquivo a substituir: {e}"))?;
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err("o destino deixou de ser um arquivo comum".into());
            }
            let current = hash_file(&dest)
                .map_err(|e| format!("não foi possível ler o arquivo existente: {e}"))?;
            if Some(current.as_str()) != entry.old_sha256.as_deref() {
                return Err(
                    "o arquivo existente mudou depois da inspeção e não será sobrescrito".into(),
                );
            }
            Some(meta.permissions())
        }
        _ => return Err("ação inesperada no plano".into()),
    };

    let parent = dest.parent().ok_or("destino sem pasta")?;
    let tmp = parent.join(format!(".gitflow-import-{op_id}-{seq}.tmp"));
    let write_tmp = || -> io::Result<()> {
        let mut input = File::open(&source)?;
        let mut out = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        io::copy(&mut input, &mut out)?;
        out.sync_all()?;
        if let Some(permissions) = &existing_permissions {
            fs::set_permissions(&tmp, permissions.clone())?;
        }
        Ok(())
    };
    if let Err(err) = write_tmp() {
        let _ = fs::remove_file(&tmp);
        return Err(format!("não foi possível gravar o arquivo: {err}"));
    }

    let placed = match entry.action {
        EntryAction::Add => match fs::hard_link(&tmp, &dest) {
            Ok(()) => {
                let _ = fs::remove_file(&tmp);
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&tmp);
                Err("o arquivo apareceu depois da inspeção e não será sobrescrito".to_string())
            }
            // Sistemas de arquivos sem hard link: confere de novo e renomeia.
            Err(_) => {
                if fs::symlink_metadata(&dest).is_ok() {
                    let _ = fs::remove_file(&tmp);
                    Err("o arquivo apareceu depois da inspeção e não será sobrescrito".to_string())
                } else {
                    fs::rename(&tmp, &dest).map_err(|e| {
                        let _ = fs::remove_file(&tmp);
                        format!("não foi possível colocar o arquivo no lugar: {e}")
                    })
                }
            }
        },
        _ => fs::rename(&tmp, &dest).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("não foi possível substituir o arquivo: {e}")
        }),
    };
    placed?;

    // Confere o que realmente ficou no disco.
    match hash_file(&dest) {
        Ok(sha) if sha == entry.new_sha256 => Ok(()),
        Ok(_) => Err("o conteúdo gravado não confere com o esperado".into()),
        Err(err) => Err(format!(
            "não foi possível conferir o arquivo gravado: {err}"
        )),
    }
}

// ----------------------------------------------------------------------------
// Reversão
// ----------------------------------------------------------------------------

/// Desfaz uma importação ainda **não commitada**: remove os arquivos novos e restaura os
/// substituídos a partir dos backups, mas só onde o conteúdo atual é o que a importação gravou.
pub fn rollback_import(git: &Git, request: &ImportOperationRequest) -> Result<RollbackResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;
    let root_text = root.to_string_lossy().into_owned();
    let git_dir = journal::git_dir(git, &root)?;
    let (dir, mut record) = journal::load_for(&git_dir, &root, &request.op_id)?;

    match record.state {
        ImportState::Applying | ImportState::Partial | ImportState::Applied => {}
        ImportState::RolledBack => {
            return Err(GitError::blocked("Esta importação já foi desfeita."))
        }
        ImportState::Committed | ImportState::Finished => {
            return Err(GitError::blocked(
                "O commit da importação já foi criado, então ela não pode ser desfeita por aqui. \
                 Para reverter um commit, use “git revert” (o histórico não é reescrito por este aplicativo).",
            ))
        }
    }

    let status = get_status(git, &root_text)?;
    if status.branch.name.as_deref() != Some(record.work_branch.as_str()) {
        return Err(GitError::blocked(format!(
            "Volte para a branch de trabalho “{}” antes de desfazer a importação.",
            record.work_branch
        )));
    }
    if let Some(operation) = status.operation {
        return Err(GitError::blocked(format!(
            "Há um {} em andamento. Conclua ou cancele essa operação antes de desfazer a importação.",
            crate::branches::operation_name(operation)
        )));
    }
    // Se a branch de trabalho já recebeu commits (por exemplo, o usuário commitou por fora do
    // fluxo), os arquivos importados fazem parte do histórico: desfazer pelo disco os removeria
    // ou reverteria como se fossem alterações pendentes. Isso é um `git revert`, não um rollback.
    let head = head_oid(git, &root)?;
    if head.as_deref() != Some(record.base_oid.as_str()) {
        return Err(GitError::blocked(
            "A branch de trabalho já tem commits depois da importação, então ela não pode ser desfeita por aqui. \
             Para reverter um commit, use “git revert” (o histórico não é reescrito por este aplicativo).",
        ));
    }
    if let Some(staged) = status
        .files
        .iter()
        .find(|f| f.index.is_some() && record.allows(&f.path))
    {
        return Err(GitError::blocked(format!(
            "“{}” está preparado (staged) no Git. Remova-o do índice (git restore --staged) antes de desfazer a importação.",
            staged.path
        )));
    }

    let backup = journal::backup_dir(&dir);
    let mut result = RollbackResult {
        op_id: record.op_id.clone(),
        state: ImportState::RolledBack,
        restored: Vec::new(),
        removed: Vec::new(),
        kept_modified: Vec::new(),
        problems: Vec::new(),
    };

    for entry in &record.entries {
        let dest = dest_path(&root, &entry.path);
        // Nunca segue links simbólicos: se algo no caminho virou um link, não mexe.
        if path_crosses_symlink(&root, &entry.path) {
            result.kept_modified.push(entry.path.clone());
            continue;
        }
        let current = match fs::symlink_metadata(&dest) {
            Ok(meta) if meta.is_file() => hash_file(&dest).ok(),
            Ok(_) => {
                result.kept_modified.push(entry.path.clone());
                continue;
            }
            Err(_) => None,
        };
        match entry.action {
            EntryAction::Add => match current {
                None => {} // nunca foi gravado (ou já foi removido)
                Some(sha) if sha == entry.new_sha256 => match fs::remove_file(&dest) {
                    Ok(()) => {
                        prune_empty_parents(&root, &dest);
                        result.removed.push(entry.path.clone());
                    }
                    Err(err) => result
                        .problems
                        .push(format!("não foi possível remover “{}”: {err}", entry.path)),
                },
                Some(_) => result.kept_modified.push(entry.path.clone()),
            },
            EntryAction::Replace => match current {
                Some(sha) if Some(sha.as_str()) == entry.old_sha256.as_deref() => {} // já original
                Some(sha) if sha == entry.new_sha256 => {
                    let source = join_rel(&backup, &entry.path);
                    match restore_from_backup(&source, &dest, entry.old_sha256.as_deref()) {
                        Ok(()) => result.restored.push(entry.path.clone()),
                        Err(message) => result.problems.push(format!(
                            "não foi possível restaurar “{}”: {message}",
                            entry.path
                        )),
                    }
                }
                _ => result.kept_modified.push(entry.path.clone()),
            },
            _ => {}
        }
    }

    record.state = ImportState::RolledBack;
    journal::save(&dir, &record)?;
    let _ = fs::remove_dir_all(journal::staged_dir(&dir));
    if result.kept_modified.is_empty() && result.problems.is_empty() {
        let _ = fs::remove_dir_all(&backup);
    }
    Ok(result)
}

fn path_crosses_symlink(root: &Path, rel: &str) -> bool {
    let mut current = root.to_path_buf();
    for component in rel.split('/') {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

fn restore_from_backup(
    backup: &Path,
    dest: &Path,
    expected_sha: Option<&str>,
) -> std::result::Result<(), String> {
    let actual = hash_file(backup).map_err(|e| format!("backup ausente ou ilegível ({e})"))?;
    if Some(actual.as_str()) != expected_sha {
        return Err("o backup não confere com o conteúdo original".into());
    }
    let permissions = fs::metadata(dest).map(|m| m.permissions()).ok();
    let parent = dest.parent().ok_or("destino sem pasta")?;
    let tmp = parent.join(format!(".gitflow-restore-{}.tmp", &actual[..12]));
    let write = || -> io::Result<()> {
        let mut input = File::open(backup)?;
        let mut out = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        io::copy(&mut input, &mut out)?;
        out.sync_all()?;
        if let Some(p) = &permissions {
            fs::set_permissions(&tmp, p.clone())?;
        }
        Ok(())
    };
    if let Err(err) = write() {
        let _ = fs::remove_file(&tmp);
        return Err(err.to_string());
    }
    fs::rename(&tmp, dest).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Remove pastas que ficaram vazias depois de apagar um arquivo novo (para quando encontra uma
/// pasta com conteúdo ou chega na raiz).
fn prune_empty_parents(root: &Path, file: &Path) {
    let mut current = file.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        if fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}
