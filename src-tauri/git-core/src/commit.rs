//! Commit seletivo local: revisão do que será registrado e criação do commit.
//!
//! ## Como a seleção vira commit
//!
//! O commit usa `git commit --only -- <arquivos selecionados>`, a semântica do próprio Git
//! para "registre exatamente estes arquivos":
//!
//! - o conteúdo registrado de cada arquivo selecionado é o **conteúdo atual dele no diretório
//!   de trabalho** (alterações preparadas + não preparadas), e a revisão mostra exatamente isso
//!   (`git diff HEAD -- <arquivo>`, ver `DiffTarget::Head`);
//! - arquivos **não selecionados** não entram no commit e **não são tocados**, e o que já estava
//!   preparado (staged) para eles continua preparado depois do commit;
//! - nunca é usado `git add .`, `git add -A` nem `commit -a`; nenhum arquivo é alterado,
//!   apagado ou sobrescrito;
//! - arquivos novos (não rastreados) selecionados recebem `git add --intent-to-add` (apenas
//!   registra o caminho no índice, sem preparar o conteúdo) para que o Git os reconheça; se o
//!   commit falhar, essa marcação é desfeita e eles voltam a ser "não rastreados".
//!
//! Os caminhos viajam por `stdin` (`--pathspec-from-file=- --pathspec-file-nul`) e são lidos de
//! forma literal (`--literal-pathspecs`): nomes com espaços, acentos, `[ ] * ?`, `:` ou iniciados
//! por hífen nunca são interpretados como opções ou padrões, e não há limite de linha de comando.
//!
//! ## Revisão × commit
//!
//! A revisão devolve uma "impressão digital" (branch, HEAD e conteúdo atual de cada arquivo
//! selecionado). Antes de commitar, ela é recalculada: se algo mudou desde a revisão, o commit
//! é recusado com `StaleReview` e a interface precisa mostrar o diff atualizado.

use crate::{
    branches::{head_oid, head_state, operation_name},
    diff::{validate_relative_path, DiffTarget},
    error::{ErrorCode, GitError, Result},
    lock::RepoGuard,
    repository::require_repository_root,
    runner::{Git, RawOutput, RunOptions},
    status::{get_status, ChangeKind, RepositoryStatus},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    fs,
    hash::{Hash, Hasher},
    io,
    path::Path,
    time::Duration,
};

const MAX_FILES: usize = 1000;
const MAX_MESSAGE_CHARS: usize = 8000;
const MAX_REPORTED_FILES: usize = 500;
/// Arquivos por chamada de `hash-object` (mantém a linha de comando bem abaixo do limite do Windows).
const HASH_CHUNK_FILES: usize = 50;

// ----------------------------------------------------------------------------
// Modelos
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub repo_path: String,
    /// Caminhos (relativos à raiz) dos arquivos selecionados.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFile {
    pub path: String,
    /// Caminho original, quando há uma renomeação preparada (o commit inclui os dois caminhos).
    pub orig_path: Option<String>,
    pub untracked: bool,
    pub has_staged: bool,
    pub has_unstaged: bool,
    /// Alvo a usar em `get_file_diff` para exibir o que será registrado neste arquivo.
    pub diff_target: DiffTarget,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitReview {
    pub repo_path: String,
    pub branch: String,
    pub head_oid: Option<String>,
    pub files: Vec<ReviewFile>,
    /// Arquivos com alterações preparadas que NÃO foram selecionados: continuam preparados
    /// e fora do commit.
    pub preserved_staged: Vec<String>,
    /// Identifica o conteúdo revisado; precisa ser devolvido em `create_commit`.
    pub fingerprint: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRequest {
    pub repo_path: String,
    pub files: Vec<String>,
    pub message: String,
    /// Branch que a interface mostrou ao usuário.
    pub expected_branch: String,
    /// Impressão digital recebida em `CommitReview`.
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommittedFile {
    /// Letra do Git: `A`, `M`, `D`, `R`, `C`, `T`.
    pub status: String,
    pub path: String,
    pub orig_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub repo_path: String,
    pub branch: String,
    /// Hash completo do commit, lido do Git depois de criado.
    pub oid: String,
    pub short_oid: String,
    pub subject: String,
    pub files: Vec<CommittedFile>,
    /// Ex.: "2 files changed, 3 insertions(+), 1 deletion(-)", como informado pelo Git.
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
struct Selected {
    path: String,
    orig_path: Option<String>,
    untracked: bool,
    has_staged: bool,
    has_unstaged: bool,
}

fn commit_opts() -> RunOptions<'static> {
    // Hooks (pre-commit etc.) e assinatura GPG podem demorar.
    RunOptions {
        timeout: Duration::from_secs(120),
        max_stdout: 256 * 1024,
        ok_exit_codes: &[0],
    }
}

fn write_opts() -> RunOptions<'static> {
    RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 256 * 1024,
        ok_exit_codes: &[0],
    }
}

fn read_opts() -> RunOptions<'static> {
    RunOptions {
        timeout: Duration::from_secs(20),
        max_stdout: 1024 * 1024,
        ok_exit_codes: &[0],
    }
}

fn unexpected(what: &str) -> GitError {
    GitError::new(
        ErrorCode::UnexpectedOutput,
        "Não foi possível interpretar a resposta do Git.",
    )
    .with_details(what)
}

// ----------------------------------------------------------------------------
// Validações
// ----------------------------------------------------------------------------

/// Valida a mensagem e devolve a versão sem espaços/linhas em branco nas pontas.
pub fn validate_commit_message(message: &str) -> Result<String> {
    if message.contains('\0') {
        return Err(GitError::invalid_argument(
            "A mensagem do commit contém caracteres inválidos.",
        ));
    }
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Err(GitError::invalid_argument(
            "A mensagem do commit não pode ser vazia.",
        ));
    }
    if trimmed.chars().count() > MAX_MESSAGE_CHARS {
        return Err(GitError::invalid_argument(format!(
            "A mensagem do commit é longa demais (máximo de {MAX_MESSAGE_CHARS} caracteres)."
        )));
    }
    Ok(trimmed.to_string())
}

/// Condições do repositório que impedem qualquer commit por aqui. Devolve a branch atual.
fn ensure_committable(status: &RepositoryStatus) -> Result<String> {
    if let Some(operation) = status.operation {
        return Err(GitError::blocked(format!(
            "Há um {} em andamento. Conclua ou cancele essa operação antes de criar um commit por aqui.",
            operation_name(operation)
        )));
    }
    if status.summary.conflicted > 0 {
        return Err(GitError::blocked(
            "Há arquivos em conflito. Resolva os conflitos antes de criar um commit.",
        ));
    }
    if status.branch.detached {
        return Err(GitError::blocked(
            "O HEAD está desanexado (fora de qualquer branch), e um commit feito assim pode se perder. \
             Crie uma branch neste ponto pelo Git (por exemplo, “git switch -c nome”) e volte aqui.",
        ));
    }
    match &status.branch.name {
        Some(name) => Ok(name.clone()),
        None => Err(GitError::blocked(
            "Não foi possível identificar a branch atual.",
        )),
    }
}

/// Confere cada caminho recebido contra o estado REAL do repositório.
fn resolve_selection(status: &RepositoryStatus, files: &[String]) -> Result<Vec<Selected>> {
    if files.is_empty() {
        return Err(GitError::invalid_argument(
            "Selecione ao menos um arquivo para o commit.",
        ));
    }
    if files.len() > MAX_FILES {
        return Err(GitError::invalid_argument(format!(
            "Há arquivos demais selecionados (máximo de {MAX_FILES} por commit)."
        )));
    }

    let mut seen: HashSet<&str> = HashSet::new();
    let mut selected = Vec::with_capacity(files.len());
    for path in files {
        validate_relative_path(path)?;
        if !seen.insert(path.as_str()) {
            return Err(GitError::invalid_argument(format!(
                "O arquivo “{path}” foi selecionado mais de uma vez."
            )));
        }
        let Some(entry) = status.files.iter().find(|f| f.path == *path) else {
            return Err(GitError::new(
                ErrorCode::StaleReview,
                format!(
                    "O arquivo “{path}” não tem mais alterações pendentes. Atualize a lista e selecione novamente."
                ),
            ));
        };
        if entry.conflicted {
            return Err(GitError::blocked(format!(
                "“{path}” está em conflito. Resolva o conflito antes de incluí-lo em um commit."
            )));
        }
        if entry.submodule || entry.path.ends_with('/') {
            return Err(GitError::blocked(format!(
                "“{path}” é um submódulo, uma pasta ou um repositório aninhado, o que ainda não é suportado."
            )));
        }
        let orig_path = if entry.index == Some(ChangeKind::Renamed) {
            entry.orig_path.clone()
        } else {
            None
        };
        selected.push(Selected {
            path: path.clone(),
            orig_path,
            untracked: entry.untracked,
            has_staged: entry.index.is_some(),
            has_unstaged: entry.worktree.is_some(),
        });
    }
    selected.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(selected)
}

// ----------------------------------------------------------------------------
// Impressão digital do conteúdo revisado
// ----------------------------------------------------------------------------

fn path_error(err: io::Error) -> GitError {
    GitError::new(
        ErrorCode::PathNotAccessible,
        "Não foi possível ler um dos arquivos selecionados.",
    )
    .with_details(err.to_string())
}

/// Identificador do conteúdo atual de cada arquivo (mesmo cálculo que o Git faria ao prepará-lo:
/// filtros e conversões de fim de linha incluídos).
fn content_ids(git: &Git, root: &Path, selected: &[Selected]) -> Result<Vec<String>> {
    let mut ids: Vec<String> = vec![String::new(); selected.len()];
    let mut to_hash: Vec<usize> = Vec::new();

    for (i, item) in selected.iter().enumerate() {
        let full = root.join(&item.path);
        match fs::symlink_metadata(&full) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = fs::read_link(&full).map_err(path_error)?;
                ids[i] = format!("symlink:{}", target.to_string_lossy());
            }
            Ok(meta) if meta.is_dir() => ids[i] = "dir".to_string(),
            Ok(_) => to_hash.push(i),
            Err(err) if err.kind() == io::ErrorKind::NotFound => ids[i] = "deleted".to_string(),
            Err(err) => return Err(path_error(err)),
        }
    }

    for chunk in to_hash.chunks(HASH_CHUNK_FILES) {
        let mut args: Vec<&str> = vec!["hash-object", "--"];
        for &i in chunk {
            args.push(selected[i].path.as_str());
        }
        let out = git.run(Some(root), &args, &read_opts())?;
        let text = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() != chunk.len() {
            return Err(unexpected("quantidade inesperada de hashes de arquivos"));
        }
        for (k, &i) in chunk.iter().enumerate() {
            ids[i] = lines[k].trim().to_string();
        }
    }
    Ok(ids)
}

fn compute_fingerprint(
    git: &Git,
    root: &Path,
    branch: &str,
    head: &Option<String>,
    selected: &[Selected],
) -> Result<String> {
    let ids = content_ids(git, root, selected)?;
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    branch.hash(&mut hasher);
    head.hash(&mut hasher);
    for (item, id) in selected.iter().zip(ids.iter()) {
        item.path.hash(&mut hasher);
        item.orig_path.hash(&mut hasher);
        item.untracked.hash(&mut hasher);
        id.hash(&mut hasher);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

// ----------------------------------------------------------------------------
// Revisão
// ----------------------------------------------------------------------------

/// Valida a seleção e devolve o que será registrado (somente leitura: não altera o repositório).
pub fn prepare_commit_review(git: &Git, request: &ReviewRequest) -> Result<CommitReview> {
    let status = get_status(git, &request.repo_path)?;
    let branch = ensure_committable(&status)?;
    let selected = resolve_selection(&status, &request.files)?;

    let root = Path::new(&status.repo_path);
    let head = head_oid(git, root)?;
    let fingerprint = compute_fingerprint(git, root, &branch, &head, &selected)?;

    let mut covered: HashSet<&str> = HashSet::new();
    for item in &selected {
        covered.insert(item.path.as_str());
        if let Some(orig) = item.orig_path.as_deref() {
            covered.insert(orig);
        }
    }
    let preserved_staged: Vec<String> = status
        .files
        .iter()
        .filter(|f| f.index.is_some() && !f.conflicted && !covered.contains(f.path.as_str()))
        .map(|f| f.path.clone())
        .collect();

    let files = selected
        .iter()
        .map(|s| ReviewFile {
            path: s.path.clone(),
            orig_path: s.orig_path.clone(),
            untracked: s.untracked,
            has_staged: s.has_staged,
            has_unstaged: s.has_unstaged,
            diff_target: if s.untracked {
                DiffTarget::Untracked
            } else {
                DiffTarget::Head
            },
        })
        .collect();

    Ok(CommitReview {
        repo_path: status.repo_path.clone(),
        branch,
        head_oid: head,
        files,
        preserved_staged,
        fingerprint,
    })
}

// ----------------------------------------------------------------------------
// Commit
// ----------------------------------------------------------------------------

fn nul_joined(paths: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for path in paths {
        out.extend_from_slice(path.as_bytes());
        out.push(0);
    }
    out
}

/// Cria o commit com os arquivos selecionados e devolve o resultado REAL lido do Git.
pub fn create_commit(git: &Git, request: &CommitRequest) -> Result<CommitResult> {
    let message = validate_commit_message(&request.message)?;
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;

    // Tudo é conferido de novo AGORA, com o estado atual do repositório.
    let status = get_status(git, &request.repo_path)?;
    let branch = ensure_committable(&status)?;
    if branch != request.expected_branch {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            format!(
                "A branch ativa mudou de “{}” para “{}” desde a revisão. Revise as alterações novamente.",
                request.expected_branch, branch
            ),
        ));
    }
    let selected = resolve_selection(&status, &request.files)?;
    let head_before = head_oid(git, &root)?;
    let current = compute_fingerprint(git, &root, &branch, &head_before, &selected)?;
    if current != request.fingerprint {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            "O conteúdo dos arquivos mudou desde a revisão. O diff foi atualizado: revise e confirme novamente.",
        ));
    }

    let mut specs: Vec<&str> = Vec::new();
    for item in &selected {
        specs.push(item.path.as_str());
        if let Some(orig) = item.orig_path.as_deref() {
            specs.push(orig);
        }
    }
    let untracked: Vec<&str> = selected
        .iter()
        .filter(|s| s.untracked)
        .map(|s| s.path.as_str())
        .collect();

    // 1) Arquivos novos: só registra o caminho no índice (intent-to-add), sem preparar conteúdo.
    if !untracked.is_empty() {
        let input = nul_joined(&untracked);
        let outcome = git.run_raw(
            Some(&root),
            &[
                "--literal-pathspecs",
                "add",
                "--intent-to-add",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            Some(input.as_slice()),
            &write_opts(),
        );
        match outcome {
            Ok(raw) if raw.exit_code == Some(0) => {}
            Ok(raw) => {
                rollback_intent_to_add(git, &root, &untracked, &head_before);
                return Err(commit_failure(
                    "Não foi possível registrar os arquivos novos selecionados. Nada foi commitado.",
                    &raw,
                ));
            }
            Err(err) => {
                rollback_intent_to_add(git, &root, &untracked, &head_before);
                return Err(err);
            }
        }
    }

    // 2) Commit seletivo.
    let message_arg = format!("--message={message}");
    let input = nul_joined(&specs);
    let outcome = git.run_raw(
        Some(&root),
        &[
            "--literal-pathspecs",
            "commit",
            "--quiet",
            "--only",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
            message_arg.as_str(),
        ],
        Some(input.as_slice()),
        &commit_opts(),
    );
    let raw = match outcome {
        Ok(raw) => raw,
        Err(err) => {
            rollback_intent_to_add(git, &root, &untracked, &head_before);
            return Err(err);
        }
    };
    if raw.exit_code != Some(0) {
        rollback_intent_to_add(git, &root, &untracked, &head_before);
        return Err(commit_failure(
            "O Git não criou o commit. Nada foi commitado e seus arquivos não foram alterados.",
            &raw,
        ));
    }

    // 3) Confere o resultado: só há sucesso se o HEAD realmente avançou.
    let head_after = head_oid(git, &root).ok().flatten();
    let Some(oid) = head_after else {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git retornou sucesso, mas não foi possível confirmar o novo commit. Atualize e confira o histórico.",
        ));
    };
    if Some(&oid) == head_before.as_ref() {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git retornou sucesso, mas nenhum commit novo foi criado. Atualize e confira o histórico.",
        ));
    }

    // O commit já existe: falhas nas leituras abaixo não o desfazem; usamos valores de reserva.
    let short_oid = git
        .run(Some(&root), &["rev-parse", "--short", "HEAD"], &read_opts())
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| oid.chars().take(7).collect());
    let files = git
        .run(
            Some(&root),
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--name-status",
                "-r",
                "-z",
                "-M",
                "HEAD",
            ],
            &read_opts(),
        )
        .map(|o| parse_name_status_z(&o.stdout))
        .unwrap_or_default();
    let summary = git
        .run(
            Some(&root),
            &["diff-tree", "--root", "--no-commit-id", "--shortstat", "-M", "HEAD"],
            &read_opts(),
        )
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    let branch_now = head_state(git, &root)
        .ok()
        .and_then(|h| h.branch)
        .unwrap_or(branch);
    let subject = message
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();

    Ok(CommitResult {
        repo_path: root.to_string_lossy().into_owned(),
        branch: branch_now,
        oid,
        short_oid,
        subject,
        files,
        summary,
    })
}

/// Desfaz o `--intent-to-add` dos arquivos novos quando o commit não aconteceu, devolvendo-os
/// ao estado "não rastreado". Só age se o HEAD não mudou (nenhum commit foi criado) e só em
/// caminhos que eram não rastreados, portanto nunca remove trabalho já preparado.
fn rollback_intent_to_add(git: &Git, root: &Path, untracked: &[&str], head_before: &Option<String>) {
    if untracked.is_empty() {
        return;
    }
    match head_oid(git, root) {
        Ok(now) if &now == head_before => {}
        _ => return,
    }
    let input = nul_joined(untracked);
    let _ = git.run_raw(
        Some(root),
        &[
            "--literal-pathspecs",
            "reset",
            "--quiet",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
        Some(input.as_slice()),
        &write_opts(),
    );
}

fn commit_failure(fallback: &str, raw: &RawOutput) -> GitError {
    let stderr = String::from_utf8_lossy(&raw.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&raw.stdout).into_owned();
    let combined = format!("{}\n{}", stderr.trim(), stdout.trim());
    let lower = combined.to_lowercase();

    let message = if lower.contains("author identity unknown")
        || lower.contains("please tell me who you are")
        || lower.contains("unable to auto-detect email")
    {
        "O Git não sabe quem você é. Configure seu nome e e-mail (git config --global user.name e \
         git config --global user.email) e tente novamente."
    } else if lower.contains("nothing to commit")
        || lower.contains("no changes added to commit")
        || lower.contains("nothing added to commit")
    {
        "Os arquivos selecionados não têm alterações para registrar (podem ter voltado ao conteúdo \
         do último commit). Atualize a lista."
    } else if lower.contains("index.lock") {
        "Há um bloqueio do Git neste repositório (index.lock). Feche outros programas que usam este \
         repositório; se nenhum estiver em uso, apague o arquivo .git/index.lock manualmente."
    } else if lower.contains("hook") {
        "Um hook do repositório (por exemplo, pre-commit) recusou o commit. Veja os detalhes."
    } else if lower.contains("gpg") {
        "A assinatura do commit falhou. Verifique a configuração de assinatura (GPG) do Git."
    } else if lower.contains("unknown option") && lower.contains("pathspec") {
        "A versão do Git instalada é antiga demais (é necessário o Git 2.25 ou superior)."
    } else {
        fallback
    };
    GitError::new(ErrorCode::CommandFailed, message).with_details(&combined)
}

/// Interpreta `git diff-tree --name-status -z`.
pub fn parse_name_status_z(output: &[u8]) -> Vec<CommittedFile> {
    let mut tokens = output
        .split(|b| *b == 0)
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .filter(|t| !t.is_empty());
    let mut files = Vec::new();
    while let Some(code) = tokens.next() {
        if files.len() >= MAX_REPORTED_FILES {
            break;
        }
        let letter = code.chars().next().unwrap_or('?');
        if letter == 'R' || letter == 'C' {
            let (Some(orig), Some(path)) = (tokens.next(), tokens.next()) else {
                break;
            };
            files.push(CommittedFile {
                status: letter.to_string(),
                path,
                orig_path: Some(orig),
            });
        } else {
            let Some(path) = tokens.next() else {
                break;
            };
            files.push(CommittedFile {
                status: letter.to_string(),
                path,
                orig_path: None,
            });
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_validation() {
        assert!(validate_commit_message("").is_err());
        assert!(validate_commit_message("   \n\t ").is_err());
        assert!(validate_commit_message("a\0b").is_err());
        assert!(validate_commit_message(&"x".repeat(MAX_MESSAGE_CHARS + 1)).is_err());
        assert_eq!(validate_commit_message("  ajusta tela \n\n").unwrap(), "ajusta tela");
        assert_eq!(validate_commit_message("-começa com hífen").unwrap(), "-começa com hífen");
    }

    #[test]
    fn nul_joined_terminates_every_path() {
        assert_eq!(nul_joined(&["a b", "ç"]), b"a b\0\xc3\xa7\0".to_vec());
        assert!(nul_joined(&[]).is_empty());
    }

    #[test]
    fn parses_name_status() {
        let data = b"M\0src/a.txt\0A\0novo arquivo.txt\0R100\0velho.txt\0novo nome.txt\0D\0x.txt\0";
        let files = parse_name_status_z(data);
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].status, "M");
        assert_eq!(files[1].path, "novo arquivo.txt");
        assert_eq!(files[2].status, "R");
        assert_eq!(files[2].orig_path.as_deref(), Some("velho.txt"));
        assert_eq!(files[2].path, "novo nome.txt");
        assert_eq!(files[3].status, "D");
    }

    #[test]
    fn name_status_tolerates_truncated_output() {
        assert!(parse_name_status_z(b"R100\0so-um-caminho\0").is_empty());
        assert!(parse_name_status_z(b"").is_empty());
    }
}
