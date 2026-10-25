//! Branches locais: listagem, validação de nomes, criação e troca segura.
//!
//! - Somente `refs/heads/` é consultado: referências remotas nunca aparecem como locais.
//! - Nomes são validados pelo próprio Git (`check-ref-format --branch`) e nunca chegam a um
//!   comando sem passar pela validação; o Git é chamado sem shell, com argumentos separados.
//! - A troca de branch **nunca** usa `--force`, `--discard-changes`, `stash`, `reset` ou `clean`:
//!   se houver alterações não commitadas em arquivos rastreados, a operação é bloqueada antes
//!   de chamar o Git, e o Git ainda recusa qualquer troca que sobrescreva arquivos locais.
//! - O resultado de cada operação é conferido depois de executada (não se presume sucesso).

use crate::{
    error::{truncate_chars, ErrorCode, GitError, Result},
    lock::RepoGuard,
    repository::require_repository_root,
    runner::{Git, RawOutput, RunOptions},
    status::{get_status, Operation},
};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

const MAX_BRANCH_NAME_CHARS: usize = 200;
const MAX_SUBJECT_CHARS: usize = 200;
/// Quantos arquivos são listados na mensagem de bloqueio da troca de branch.
const MAX_BLOCKING_FILES_LISTED: usize = 12;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchEntry {
    /// Nome sem o prefixo `refs/heads/`.
    pub name: String,
    /// `true` para a branch atualmente ativa (HEAD).
    pub current: bool,
    /// Commit para o qual a branch aponta.
    pub oid: String,
    pub upstream: Option<String>,
    /// Assunto do último commit da branch.
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchList {
    pub repo_path: String,
    /// Branch atual; `None` com HEAD desanexado.
    pub current: Option<String>,
    pub detached: bool,
    /// A branch atual ainda não tem commits (por isso não aparece em `branches`).
    pub unborn: bool,
    pub head_oid: Option<String>,
    pub branches: Vec<BranchEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchNameCheck {
    pub valid: bool,
    /// Motivo da rejeição, em português, quando `valid` é `false`.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchRequest {
    pub repo_path: String,
    pub name: String,
    /// Branch local usada como ponto de partida (escolhida explicitamente pelo usuário).
    pub base: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedBranch {
    pub repo_path: String,
    pub name: String,
    pub base: String,
    /// Commit em que a nova branch foi criada (conferido após a criação).
    pub oid: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchBranchRequest {
    pub repo_path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchResult {
    pub repo_path: String,
    pub previous: Option<String>,
    /// Branch ativa **depois** da troca, lida novamente do Git.
    pub current: String,
    pub head_oid: Option<String>,
}

// ----------------------------------------------------------------------------
// HEAD
// ----------------------------------------------------------------------------

pub(crate) struct HeadState {
    /// `None` com HEAD desanexado.
    pub branch: Option<String>,
    /// `None` quando a branch ainda não tem commits.
    pub oid: Option<String>,
}

fn quiet_opts() -> RunOptions<'static> {
    RunOptions {
        timeout: Duration::from_secs(15),
        max_stdout: 64 * 1024,
        ok_exit_codes: &[0, 1],
    }
}

fn write_opts() -> RunOptions<'static> {
    RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 256 * 1024,
        ok_exit_codes: &[0],
    }
}

/// Commit apontado por HEAD, ou `None` se o repositório ainda não tem commits.
pub(crate) fn head_oid(git: &Git, root: &Path) -> Result<Option<String>> {
    let out = git.run(
        Some(root),
        &["rev-parse", "--verify", "-q", "HEAD"],
        &quiet_opts(),
    )?;
    if out.exit_code != 0 {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let oid = text.trim();
    Ok(if oid.is_empty() {
        None
    } else {
        Some(oid.to_string())
    })
}

pub(crate) fn head_state(git: &Git, root: &Path) -> Result<HeadState> {
    let sym = git.run(Some(root), &["symbolic-ref", "-q", "HEAD"], &quiet_opts())?;
    let branch = if sym.exit_code == 0 {
        let text = String::from_utf8_lossy(&sym.stdout);
        let line = text.trim_end_matches(['\n', '\r']);
        line.strip_prefix("refs/heads/").map(|s| s.to_string())
    } else {
        None
    };
    let oid = head_oid(git, root)?;
    Ok(HeadState { branch, oid })
}

pub(crate) fn operation_name(operation: Operation) -> &'static str {
    match operation {
        Operation::Merge => "merge",
        Operation::Rebase => "rebase",
        Operation::CherryPick => "cherry-pick",
        Operation::Revert => "revert",
        Operation::Bisect => "bisect",
    }
}

// ----------------------------------------------------------------------------
// Listagem
// ----------------------------------------------------------------------------

pub fn list_branches(git: &Git, repo_path: &str) -> Result<BranchList> {
    let root = require_repository_root(git, repo_path)?;
    let head = head_state(git, &root)?;
    let branches = read_local_branches(git, &root)?;
    Ok(BranchList {
        repo_path: root.to_string_lossy().into_owned(),
        current: head.branch.clone(),
        detached: head.branch.is_none(),
        unborn: head.branch.is_some() && head.oid.is_none(),
        head_oid: head.oid,
        branches,
    })
}

fn read_local_branches(git: &Git, root: &Path) -> Result<Vec<BranchEntry>> {
    let opts = RunOptions {
        timeout: Duration::from_secs(30),
        max_stdout: 4 * 1024 * 1024,
        ..RunOptions::default()
    };
    let out = git.run(
        Some(root),
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname)%1f%(objectname)%1f%(upstream:short)%1f%(HEAD)%1f%(contents:subject)",
            "refs/heads/",
        ],
        &opts,
    )?;
    if out.stdout_truncated {
        return Err(GitError::new(
            ErrorCode::OutputTooLarge,
            "Há branches demais para listar.",
        ));
    }
    parse_branch_list(&String::from_utf8_lossy(&out.stdout))
}

/// Interpreta a saída de `git for-each-ref` no formato usado por `read_local_branches`
/// (campos separados por `0x1F`, um registro por linha).
pub fn parse_branch_list(text: &str) -> Result<Vec<BranchEntry>> {
    let mut branches = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.splitn(5, '\u{1f}').collect();
        if fields.len() != 5 {
            return Err(unexpected("registro de branch incompleto"));
        }
        let Some(name) = fields[0].strip_prefix("refs/heads/") else {
            return Err(unexpected("referência fora de refs/heads/"));
        };
        let non_empty = |s: &str| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        };
        branches.push(BranchEntry {
            name: name.to_string(),
            current: fields[3] == "*",
            oid: fields[1].to_string(),
            upstream: non_empty(fields[2]),
            subject: non_empty(fields[4]).map(|s| truncate_chars(&s, MAX_SUBJECT_CHARS)),
        });
    }
    Ok(branches)
}

fn unexpected(what: &str) -> GitError {
    GitError::new(
        ErrorCode::UnexpectedOutput,
        "Não foi possível interpretar a resposta do Git.",
    )
    .with_details(what)
}

// ----------------------------------------------------------------------------
// Validação de nomes
// ----------------------------------------------------------------------------

/// Regras básicas, verificadas antes de qualquer chamada ao Git.
pub fn check_branch_name_syntax(name: &str) -> std::result::Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("Informe um nome para a branch.");
    }
    if name != name.trim() {
        return Err("O nome não pode começar nem terminar com espaços.");
    }
    if name.chars().count() > MAX_BRANCH_NAME_CHARS {
        return Err("O nome é longo demais (máximo de 200 caracteres).");
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("O nome contém caracteres de controle.");
    }
    if name.starts_with('-') {
        return Err("O nome não pode começar com hífen.");
    }
    if name == "HEAD" {
        return Err("“HEAD” não pode ser usado como nome de branch.");
    }
    Ok(())
}

/// `Ok(None)` = nome utilizável; `Ok(Some(motivo))` = rejeitado; `Err` = falha ao executar o Git.
fn validate_new_branch_name(
    git: &Git,
    root: &Path,
    name: &str,
    existing: &[BranchEntry],
) -> Result<Option<String>> {
    if let Err(reason) = check_branch_name_syntax(name) {
        return Ok(Some(reason.to_string()));
    }

    // O próprio Git decide se o nome é uma referência válida. A saída precisa ser idêntica ao
    // nome informado: isso rejeita formas especiais como `@{-1}`, que o Git expandiria.
    let raw = git.run_raw(
        Some(root),
        &["check-ref-format", "--branch", name],
        None,
        &quiet_opts(),
    )?;
    let accepted = raw.exit_code == Some(0)
        && String::from_utf8_lossy(&raw.stdout).trim_end_matches(['\n', '\r']) == name;
    if !accepted {
        return Ok(Some(
            "O Git não aceita este nome. Evite espaços, “..”, “~”, “^”, “:”, “?”, “*”, “[”, “\\”, \
             “@{”, terminar com “/”, “.” ou “.lock”, e barras duplicadas."
                .to_string(),
        ));
    }

    let lower = name.to_lowercase();
    for branch in existing {
        if branch.name == name {
            return Ok(Some("Já existe uma branch local com este nome.".to_string()));
        }
        let other = branch.name.to_lowercase();
        if other == lower {
            return Ok(Some(format!(
                "Já existe a branch “{}”, que difere apenas em maiúsculas/minúsculas. \
                 O Windows não diferencia essas letras.",
                branch.name
            )));
        }
        if other.starts_with(&format!("{lower}/")) || lower.starts_with(&format!("{other}/")) {
            return Ok(Some(format!(
                "O nome conflita com a branch “{}”: uma branch não pode ser a “pasta” de outra.",
                branch.name
            )));
        }
    }
    Ok(None)
}

/// Verificação somente leitura usada pelo formulário para avisar antes de enviar.
pub fn check_branch_name(git: &Git, repo_path: &str, name: &str) -> Result<BranchNameCheck> {
    let root = require_repository_root(git, repo_path)?;
    let existing = read_local_branches(git, &root)?;
    let reason = validate_new_branch_name(git, &root, name, &existing)?;
    Ok(BranchNameCheck {
        valid: reason.is_none(),
        reason,
    })
}

// ----------------------------------------------------------------------------
// Criação
// ----------------------------------------------------------------------------

/// Cria uma branch local **sem trocar para ela** (a branch atual não muda).
pub fn create_branch(git: &Git, request: &CreateBranchRequest) -> Result<CreatedBranch> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;

    let head = head_state(git, &root)?;
    if head.oid.is_none() {
        return Err(GitError::blocked(
            "Este repositório ainda não tem commits. Crie o primeiro commit antes de criar outras branches.",
        ));
    }

    let existing = read_local_branches(git, &root)?;
    if let Some(reason) = validate_new_branch_name(git, &root, &request.name, &existing)? {
        return Err(GitError::invalid_argument(reason));
    }
    let Some(base) = existing.iter().find(|b| b.name == request.base) else {
        return Err(GitError::invalid_argument(
            "A branch-base escolhida não existe entre as branches locais.",
        ));
    };

    // `refs/heads/<base>` evita ambiguidade com tags ou outras referências de mesmo nome.
    let base_ref = format!("refs/heads/{}", base.name);
    git.run(
        Some(&root),
        &[
            "branch",
            "--no-track",
            request.name.as_str(),
            base_ref.as_str(),
        ],
        &write_opts(),
    )?;

    // Confere o resultado real em vez de presumir sucesso.
    let new_ref = format!("refs/heads/{}", request.name);
    let out = git.run(
        Some(&root),
        &["rev-parse", "--verify", "-q", new_ref.as_str()],
        &quiet_opts(),
    )?;
    let oid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.exit_code != 0 || oid.is_empty() || oid != base.oid {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git informou sucesso, mas a nova branch não foi encontrada como esperado. Atualize a lista e confira.",
        ));
    }

    Ok(CreatedBranch {
        repo_path: root.to_string_lossy().into_owned(),
        name: request.name.clone(),
        base: base.name.clone(),
        oid,
    })
}

// ----------------------------------------------------------------------------
// Troca de branch
// ----------------------------------------------------------------------------

/// Troca para uma branch local existente, **apenas** se não houver trabalho local em risco.
pub fn switch_branch(git: &Git, request: &SwitchBranchRequest) -> Result<SwitchResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;

    let name = request.name.as_str();
    if let Err(reason) = check_branch_name_syntax(name) {
        return Err(GitError::invalid_argument(reason));
    }
    let existing = read_local_branches(git, &root)?;
    if !existing.iter().any(|b| b.name == name) {
        return Err(GitError::invalid_argument(
            "A branch de destino não existe entre as branches locais.",
        ));
    }

    let head = head_state(git, &root)?;
    if head.branch.as_deref() == Some(name) {
        return Err(GitError::invalid_argument("Esta já é a branch atual."));
    }

    // Verificação de segurança com o estado REAL do repositório neste instante.
    let status = get_status(git, &request.repo_path)?;
    if let Some(operation) = status.operation {
        return Err(GitError::blocked(format!(
            "Há um {} em andamento. Conclua ou cancele essa operação antes de trocar de branch.",
            operation_name(operation)
        )));
    }
    if status.summary.conflicted > 0 {
        return Err(GitError::blocked(
            "Há arquivos em conflito. Resolva os conflitos antes de trocar de branch.",
        ));
    }
    let at_risk: Vec<&str> = status
        .files
        .iter()
        .filter(|f| f.index.is_some() || f.worktree.is_some())
        .map(|f| f.path.as_str())
        .collect();
    if !at_risk.is_empty() {
        let listed: Vec<&str> = at_risk
            .iter()
            .take(MAX_BLOCKING_FILES_LISTED)
            .copied()
            .collect();
        let mut details = listed.join("\n");
        if at_risk.len() > listed.len() {
            details.push_str(&format!("\n… e mais {}", at_risk.len() - listed.len()));
        }
        return Err(GitError::blocked(format!(
            "Há {} arquivo(s) com alterações ainda não commitadas. A troca foi bloqueada para não \
             arriscar o seu trabalho. Faça o commit dessas alterações, ou guarde-as/descarte-as \
             manualmente (por exemplo, com “git stash” ou “git restore”) e tente novamente.",
            at_risk.len()
        ))
        .with_details(details));
    }

    // Sem `--force`: o Git continua recusando qualquer troca que sobrescreva arquivos locais
    // (inclusive não rastreados). `--no-guess` impede criar branch a partir de remota por engano.
    let raw = git.run_raw(
        Some(&root),
        &["switch", "--no-guess", name],
        None,
        &write_opts(),
    )?;
    if raw.exit_code != Some(0) {
        return Err(switch_failure(&raw));
    }

    // Confere o resultado real.
    let after = head_state(git, &root)?;
    if after.branch.as_deref() != Some(name) {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git informou sucesso, mas a branch atual não é a esperada. Atualize e confira.",
        ));
    }
    Ok(SwitchResult {
        repo_path: root.to_string_lossy().into_owned(),
        previous: head.branch,
        current: name.to_string(),
        head_oid: after.oid,
    })
}

fn switch_failure(raw: &RawOutput) -> GitError {
    let stderr = String::from_utf8_lossy(&raw.stderr).into_owned();
    let lower = stderr.to_lowercase();
    if lower.contains("would be overwritten") {
        return GitError::blocked(
            "O Git recusou a troca porque ela sobrescreveria arquivos locais. Nada foi alterado. \
             Mova, commite ou guarde esses arquivos manualmente e tente novamente.",
        )
        .with_details(&stderr);
    }
    if lower.contains("already checked out") || lower.contains("already used by worktree") {
        return GitError::blocked(
            "Esta branch já está aberta em outra pasta de trabalho (git worktree). Nada foi alterado.",
        )
        .with_details(&stderr);
    }
    if lower.contains("is not a git command") {
        return GitError::new(
            ErrorCode::GitUnavailable,
            "A versão do Git instalada é antiga demais (é necessário o Git 2.23 ou superior).",
        )
        .with_details(&stderr);
    }
    GitError::new(
        ErrorCode::CommandFailed,
        "O Git não conseguiu trocar de branch. Nada foi afirmado como concluído.",
    )
    .with_details(&stderr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_rejects_empty_and_dangerous_names() {
        assert!(check_branch_name_syntax("").is_err());
        assert!(check_branch_name_syntax("   ").is_err());
        assert!(check_branch_name_syntax(" feature").is_err());
        assert!(check_branch_name_syntax("feature ").is_err());
        assert!(check_branch_name_syntax("-delete").is_err());
        assert!(check_branch_name_syntax("--force").is_err());
        assert!(check_branch_name_syntax("HEAD").is_err());
        assert!(check_branch_name_syntax("a\nb").is_err());
        assert!(check_branch_name_syntax("a\0b").is_err());
        assert!(check_branch_name_syntax(&"x".repeat(201)).is_err());
    }

    #[test]
    fn syntax_accepts_usual_names() {
        for name in ["feature/login", "fix-123", "release_1.2", "ação-ç", "a/b/c"] {
            assert!(check_branch_name_syntax(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn parses_branch_records() {
        let text = "refs/heads/feat/x\u{1f}aaaa\u{1f}origin/feat/x\u{1f} \u{1f}Add x\n\
                    refs/heads/main\u{1f}bbbb\u{1f}\u{1f}*\u{1f}Initial commit\n\n";
        let list = parse_branch_list(text).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "feat/x");
        assert!(!list[0].current);
        assert_eq!(list[0].upstream.as_deref(), Some("origin/feat/x"));
        assert_eq!(list[1].name, "main");
        assert!(list[1].current);
        assert_eq!(list[1].upstream, None);
        assert_eq!(list[1].subject.as_deref(), Some("Initial commit"));
    }

    #[test]
    fn rejects_non_local_refs_and_malformed_records() {
        assert!(parse_branch_list("refs/remotes/origin/x\u{1f}a\u{1f}\u{1f} \u{1f}s\n").is_err());
        assert!(parse_branch_list("refs/heads/x\u{1f}a\n").is_err());
    }
}
