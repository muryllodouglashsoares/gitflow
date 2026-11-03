//! Merge manual entre duas branches locais (Fase 5) e proteção da `main`.
//!
//! Fluxo em duas etapas, ambas calculadas pelo Git real:
//!
//! 1. [`preview_merge`] (somente leitura): valida origem/destino, lê o estado do repositório,
//!    mede a divergência, prevê conflitos e devolve bloqueios explicados. O resultado carrega um
//!    `token` que identifica **exatamente** o que foi revisado (branch atual, OIDs das duas
//!    branches, base do merge, operação em andamento e arquivos alterados).
//! 2. [`merge_branches`] (escrita): exige confirmação explícita (e confirmação específica quando o
//!    destino é `main`/`master`), adquire a trava do repositório, **recalcula** a prévia e recusa
//!    a execução se o `token` não for idêntico (`StaleReview`). Só então executa `git merge`.
//!
//! Regras de segurança:
//! - o merge integra a **origem no destino**, e o destino precisa ser a branch atual: o módulo
//!   nunca troca de branch sozinho (a troca é uma ação separada e explícita, `switch_branch`);
//! - nunca usa `stash`, `reset`, `clean`, `--force`, `--allow-unrelated-histories` nem
//!   `--autostash`; `rerere` é desativado para que nenhum conflito seja resolvido por conta própria;
//! - alterações locais (preparadas ou não) e arquivos não rastreados que seriam sobrescritos
//!   bloqueiam a operação antes de chamar o Git;
//! - Git sem shell, argumentos separados, `refs/heads/<nome>` para evitar ambiguidade;
//! - o resultado é conferido depois (HEAD, ancestralidade, pais do commit, status): sucesso só é
//!   informado quando o estado real do Git o comprova, e conflitos nunca são apresentados como sucesso.

use crate::{
    branches::{check_branch_name_syntax, head_state, operation_name, read_local_branches, BranchEntry},
    error::{truncate_chars, ErrorCode, GitError, Result},
    lock::RepoGuard,
    remote::{redact, PROTECTED_NAMES},
    repository::require_repository_root,
    runner::{Git, RawOutput, RunOptions},
    status::{get_status, Operation, RepositoryStatus, StatusSummary},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path, time::Duration};

/// Quantos commits exclusivos da origem aparecem no resumo da prévia.
const MAX_COMMITS_LISTED: usize = 10;
/// Quantos arquivos são citados em cada mensagem de bloqueio.
const MAX_FILES_LISTED: usize = 12;
const MAX_SUBJECT_CHARS: usize = 200;
const MAX_OUTPUT_SHOWN_CHARS: usize = 4000;
const MERGE_TIMEOUT_SECS: u64 = 300;

// ----------------------------------------------------------------------------
// Modelos
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePreviewRequest {
    pub repo_path: String,
    /// Branch cujos commits serão integrados.
    pub source: String,
    /// Branch que receberá os commits (precisa ser a branch atual para executar).
    pub target: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MergeRelation {
    /// As duas branches apontam para o mesmo commit.
    Identical,
    /// Tudo o que a origem tem já está no destino: não há o que integrar.
    AlreadyIntegrated,
    /// O destino é ancestral da origem: o Git apenas avança o ponteiro (sem commit de merge).
    FastForward,
    /// Há commits novos nos dois lados: será criado um commit de merge.
    Diverged,
    /// Sem histórico em comum: o aplicativo não integra históricos independentes.
    Unrelated,
}

/// Como o merge será concluído, se executado agora.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MergeStrategy {
    FastForward,
    MergeCommit,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BlockerKind {
    UnbornHead,
    DetachedHead,
    OperationInProgress,
    Conflicts,
    StagedChanges,
    UnstagedChanges,
    UntrackedOverwrite,
    UnrelatedHistories,
    /// O destino não é a branch atual: é preciso trocar de branch (ação explícita separada).
    TargetNotCurrent,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MergeBlocker {
    pub kind: BlockerKind,
    pub message: String,
    /// Arquivos que causam o bloqueio (limitados; veja `files_total`).
    pub files: Vec<String>,
    /// Quantidade real de arquivos envolvidos (pode ser maior que `files.len()`).
    pub files_total: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitBrief {
    pub oid: String,
    pub subject: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ForecastState {
    /// Não se aplica (avanço rápido, já integrado, histórico independente).
    NotApplicable,
    /// O Git simulou a integração e não encontrou conflitos.
    Clean,
    /// O Git simulou a integração e encontrou conflitos.
    Conflicts,
    /// Não foi possível simular (Git antigo ou falha): não presuma nada.
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConflictForecast {
    pub state: ForecastState,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePreview {
    pub repo_path: String,
    pub source: BranchEntry,
    pub target: BranchEntry,
    /// O destino é `main`/`master`: a execução exige confirmação específica no backend.
    pub target_protected: bool,
    /// Texto exato que o usuário precisa digitar para integrar na branch principal.
    pub required_confirmation: Option<String>,
    pub current_branch: Option<String>,
    pub head_oid: Option<String>,
    pub target_is_current: bool,
    pub relation: MergeRelation,
    pub strategy: Option<MergeStrategy>,
    pub merge_base: Option<String>,
    /// Commits que só a origem tem (serão integrados).
    pub source_only: u32,
    /// Commits que só o destino tem.
    pub target_only: u32,
    /// Os mais recentes entre os commits exclusivos da origem.
    pub source_commits: Vec<CommitBrief>,
    pub forecast: ConflictForecast,
    pub summary: StatusSummary,
    pub operation: Option<Operation>,
    pub blockers: Vec<MergeBlocker>,
    /// Observações que não impedem a operação.
    pub notes: Vec<String>,
    /// Nenhum bloqueio e há algo a integrar.
    pub can_execute: bool,
    /// Identifica o que foi revisado; precisa voltar idêntico em `merge_branches`.
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequest {
    pub repo_path: String,
    pub source: String,
    pub target: String,
    /// `token` da prévia que o usuário revisou.
    pub token: String,
    /// Confirmação explícita da integração. Sem ela, nada é executado.
    pub confirmed: bool,
    /// Para `main`/`master`: o nome do destino, digitado pelo usuário.
    pub confirm_target: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MergeOutcome {
    FastForward,
    MergeCommit,
    /// O Git parou por conflitos: o merge **não** foi concluído e continua em andamento.
    Conflicts,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConflictEntry {
    pub path: String,
    /// Código do Git (`UU`, `AA`, `DU`…).
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeResult {
    pub repo_path: String,
    pub source: String,
    pub target: String,
    pub outcome: MergeOutcome,
    pub source_oid: String,
    /// HEAD do destino antes do merge.
    pub head_before: Option<String>,
    /// HEAD do destino depois do merge (lido de novo do Git).
    pub head_after: Option<String>,
    /// `true` quando o HEAD resultante é um commit de merge.
    pub merge_commit: bool,
    /// Commits exclusivos da origem no momento da prévia.
    pub integrated_commits: u32,
    pub conflicts: Vec<ConflictEntry>,
    pub exit_code: Option<i32>,
    /// Saída real do Git (stdout + stderr), limitada em tamanho.
    pub output: String,
    /// Estado do repositório lido **depois** da operação.
    pub status: RepositoryStatus,
}

// ----------------------------------------------------------------------------
// Utilidades
// ----------------------------------------------------------------------------

/// `true` para as branches que recebem tratamento especial (`main` e `master`).
pub fn is_protected_branch(name: &str) -> bool {
    PROTECTED_NAMES.contains(&name)
}

fn read_opts() -> RunOptions<'static> {
    RunOptions {
        timeout: Duration::from_secs(30),
        max_stdout: 4 * 1024 * 1024,
        ok_exit_codes: &[0, 1],
    }
}

fn head_ref(name: &str) -> String {
    format!("refs/heads/{name}")
}

fn unexpected(what: &str) -> GitError {
    GitError::new(
        ErrorCode::UnexpectedOutput,
        "Não foi possível interpretar a resposta do Git.",
    )
    .with_details(what)
}

fn text_of(raw: &RawOutput) -> String {
    String::from_utf8_lossy(&raw.stdout).trim().to_string()
}

fn run_read(git: &Git, root: &Path, args: &[&str]) -> Result<RawOutput> {
    let raw = git.run_raw(Some(root), args, None, &read_opts())?;
    if raw.stdout_truncated {
        return Err(GitError::new(
            ErrorCode::OutputTooLarge,
            "A resposta do Git ficou grande demais para ser analisada com segurança.",
        ));
    }
    Ok(raw)
}

/// `git merge-base --is-ancestor <ancestor> <descendant>` (códigos: 0 = sim, 1 = não).
fn is_ancestor(git: &Git, root: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let raw = run_read(git, root, &["merge-base", "--is-ancestor", ancestor, descendant])?;
    match raw.exit_code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(GitError::new(
            ErrorCode::CommandFailed,
            "O Git não conseguiu comparar o histórico das duas branches.",
        )
        .with_details(String::from_utf8_lossy(&raw.stderr))),
    }
}

fn count_commits(git: &Git, root: &Path, range: &str) -> Result<u32> {
    let raw = run_read(git, root, &["rev-list", "--count", range])?;
    if raw.exit_code != Some(0) {
        return Err(GitError::new(
            ErrorCode::CommandFailed,
            "O Git não conseguiu contar os commits da integração.",
        )
        .with_details(String::from_utf8_lossy(&raw.stderr)));
    }
    text_of(&raw)
        .parse()
        .map_err(|_| unexpected("contagem de commits inválida"))
}

fn list_commits(git: &Git, root: &Path, range: &str) -> Result<Vec<CommitBrief>> {
    let limit = MAX_COMMITS_LISTED.to_string();
    let raw = run_read(
        git,
        root,
        &["log", "-n", limit.as_str(), "--format=%H%x1f%s", range],
    )?;
    if raw.exit_code != Some(0) {
        return Err(GitError::new(
            ErrorCode::CommandFailed,
            "O Git não conseguiu listar os commits da origem.",
        )
        .with_details(String::from_utf8_lossy(&raw.stderr)));
    }
    let text = String::from_utf8_lossy(&raw.stdout).into_owned();
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let (oid, subject) = line.split_once('\u{1f}').unwrap_or((line, ""));
        out.push(CommitBrief {
            oid: oid.to_string(),
            subject: truncate_chars(subject.trim(), MAX_SUBJECT_CHARS),
        });
    }
    Ok(out)
}

fn merge_base(git: &Git, root: &Path, a: &str, b: &str) -> Result<Option<String>> {
    let raw = run_read(git, root, &["merge-base", a, b])?;
    match raw.exit_code {
        Some(0) => {
            let oid = text_of(&raw);
            Ok(if oid.is_empty() { None } else { Some(oid) })
        }
        Some(1) => Ok(None),
        _ => Err(GitError::new(
            ErrorCode::CommandFailed,
            "O Git não conseguiu encontrar a base comum das branches.",
        )
        .with_details(String::from_utf8_lossy(&raw.stderr))),
    }
}

/// Caminhos alterados entre a base e a origem (os que o merge pode escrever no diretório de trabalho).
fn paths_changed(git: &Git, root: &Path, base: &str, source_ref: &str) -> Result<Vec<String>> {
    let raw = run_read(
        git,
        root,
        &["diff", "--name-only", "--no-renames", "-z", base, source_ref],
    )?;
    if raw.exit_code != Some(0) {
        return Err(GitError::new(
            ErrorCode::CommandFailed,
            "O Git não conseguiu listar os arquivos afetados pela integração.",
        )
        .with_details(String::from_utf8_lossy(&raw.stderr)));
    }
    Ok(raw
        .stdout
        .split(|b| *b == 0)
        .filter(|t| !t.is_empty())
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .collect())
}

/// Simula a integração em memória (`git merge-tree --write-tree`): não toca em referências, no
/// índice nem no diretório de trabalho. Exige Git 2.38+; em versões antigas devolve `Unavailable`.
fn forecast_conflicts(git: &Git, root: &Path, target_ref: &str, source_ref: &str) -> ConflictForecast {
    let unavailable = ConflictForecast {
        state: ForecastState::Unavailable,
        files: Vec::new(),
    };
    let Ok(raw) = run_read(
        git,
        root,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            "-z",
            target_ref,
            source_ref,
        ],
    ) else {
        return unavailable;
    };
    match raw.exit_code {
        Some(0) => ConflictForecast {
            state: ForecastState::Clean,
            files: Vec::new(),
        },
        Some(1) => {
            // Saída com -z: `<oid da árvore>\0<arquivo>\0<arquivo>\0…`
            let mut tokens = raw
                .stdout
                .split(|b| *b == 0)
                .filter(|t| !t.is_empty())
                .map(|t| String::from_utf8_lossy(t).into_owned());
            let _tree = tokens.next();
            let mut files: Vec<String> = tokens.collect();
            files.sort();
            files.dedup();
            ConflictForecast {
                state: ForecastState::Conflicts,
                files,
            }
        }
        _ => unavailable,
    }
}

fn limited(files: &[String]) -> Vec<String> {
    files.iter().take(MAX_FILES_LISTED).cloned().collect()
}

fn blocker(kind: BlockerKind, message: impl Into<String>, files: &[String]) -> MergeBlocker {
    MergeBlocker {
        kind,
        message: message.into(),
        files: limited(files),
        files_total: files.len(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Impressão digital do que foi revisado. Muda se qualquer um destes itens mudar.
fn fingerprint(parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hex(&hasher.finalize())
}

fn validate_branches<'a>(
    branches: &'a [BranchEntry],
    source: &str,
    target: &str,
) -> Result<(&'a BranchEntry, &'a BranchEntry)> {
    if let Err(reason) = check_branch_name_syntax(source) {
        return Err(GitError::invalid_argument(format!("Branch de origem inválida: {reason}")));
    }
    if let Err(reason) = check_branch_name_syntax(target) {
        return Err(GitError::invalid_argument(format!("Branch de destino inválida: {reason}")));
    }
    if source == target {
        return Err(GitError::invalid_argument(
            "A origem e o destino são a mesma branch. Escolha duas branches diferentes.",
        ));
    }
    let Some(src) = branches.iter().find(|b| b.name == source) else {
        return Err(GitError::invalid_argument(format!(
            "A branch de origem “{source}” não existe entre as branches locais. Atualize a lista."
        )));
    };
    let Some(dst) = branches.iter().find(|b| b.name == target) else {
        return Err(GitError::invalid_argument(format!(
            "A branch de destino “{target}” não existe entre as branches locais. Atualize a lista."
        )));
    };
    Ok((src, dst))
}

// ----------------------------------------------------------------------------
// Prévia (somente leitura)
// ----------------------------------------------------------------------------

/// Analisa a integração `source → target` sem alterar nada.
pub fn preview_merge(git: &Git, request: &MergePreviewRequest) -> Result<MergePreview> {
    let root = require_repository_root(git, &request.repo_path)?;
    build_preview(git, &root, &request.source, &request.target)
}

fn build_preview(git: &Git, root: &Path, source_name: &str, target_name: &str) -> Result<MergePreview> {
    let root_text = root.to_string_lossy().into_owned();
    let branches = read_local_branches(git, root)?;
    let (source, target) = validate_branches(&branches, source_name, target_name)?;
    let (source, target) = (source.clone(), target.clone());

    let head = head_state(git, root)?;
    let status = get_status(git, &root_text)?;
    let source_ref = head_ref(&source.name);
    let target_ref = head_ref(&target.name);

    // --- relação entre as branches (sempre pelo Git, nunca presumida) ---
    let relation = if source.oid == target.oid {
        MergeRelation::Identical
    } else if is_ancestor(git, root, &source.oid, &target.oid)? {
        MergeRelation::AlreadyIntegrated
    } else if is_ancestor(git, root, &target.oid, &source.oid)? {
        MergeRelation::FastForward
    } else if merge_base(git, root, &target_ref, &source_ref)?.is_some() {
        MergeRelation::Diverged
    } else {
        MergeRelation::Unrelated
    };
    let base = match relation {
        MergeRelation::FastForward => Some(target.oid.clone()),
        MergeRelation::Diverged => merge_base(git, root, &target_ref, &source_ref)?,
        _ => None,
    };

    let source_range = format!("{target_ref}..{source_ref}");
    let target_range = format!("{source_ref}..{target_ref}");
    let (source_only, target_only, source_commits) = if relation == MergeRelation::Unrelated {
        (0, 0, Vec::new())
    } else {
        (
            count_commits(git, root, &source_range)?,
            count_commits(git, root, &target_range)?,
            list_commits(git, root, &source_range)?,
        )
    };

    let forecast = if relation == MergeRelation::Diverged {
        forecast_conflicts(git, root, &target_ref, &source_ref)
    } else {
        ConflictForecast {
            state: ForecastState::NotApplicable,
            files: Vec::new(),
        }
    };

    let strategy = match relation {
        MergeRelation::FastForward => Some(MergeStrategy::FastForward),
        MergeRelation::Diverged => Some(MergeStrategy::MergeCommit),
        _ => None,
    };

    // --- bloqueios ---
    let mut blockers = Vec::new();
    let mut notes = Vec::new();

    if status.branch.unborn {
        blockers.push(blocker(
            BlockerKind::UnbornHead,
            "A branch atual ainda não tem commits. Crie o primeiro commit antes de integrar branches.",
            &[],
        ));
    }
    if status.branch.detached {
        blockers.push(blocker(
            BlockerKind::DetachedHead,
            "O repositório está com o HEAD desanexado (fora de qualquer branch). Troque para a branch de \
             destino antes de integrar, para que o merge não crie commits soltos.",
            &[],
        ));
    }
    if let Some(operation) = status.operation {
        blockers.push(blocker(
            BlockerKind::OperationInProgress,
            format!(
                "Há um {} em andamento neste repositório. Conclua ou cancele essa operação antes de iniciar outra integração.",
                operation_name(operation)
            ),
            &[],
        ));
    }
    let conflicted: Vec<String> = status
        .files
        .iter()
        .filter(|f| f.conflicted)
        .map(|f| f.path.clone())
        .collect();
    if !conflicted.is_empty() {
        blockers.push(blocker(
            BlockerKind::Conflicts,
            format!(
                "Há {} arquivo(s) em conflito de uma integração anterior. Resolva-os antes de integrar outra branch.",
                conflicted.len()
            ),
            &conflicted,
        ));
    }
    let staged: Vec<String> = status
        .files
        .iter()
        .filter(|f| f.index.is_some())
        .map(|f| f.path.clone())
        .collect();
    if !staged.is_empty() {
        blockers.push(blocker(
            BlockerKind::StagedChanges,
            format!(
                "Há {} arquivo(s) com alterações preparadas (staged) e ainda não commitadas. O merge foi \
                 bloqueado para não misturar nem arriscar o seu trabalho. Faça o commit dessas alterações \
                 e tente novamente; o aplicativo não guarda nem descarta alterações para forçar a integração.",
                staged.len()
            ),
            &staged,
        ));
    }
    let unstaged: Vec<String> = status
        .files
        .iter()
        .filter(|f| f.worktree.is_some())
        .map(|f| f.path.clone())
        .collect();
    if !unstaged.is_empty() {
        blockers.push(blocker(
            BlockerKind::UnstagedChanges,
            format!(
                "Há {} arquivo(s) modificados ainda não preparados. O merge foi bloqueado para não \
                 arriscar o seu trabalho. Faça o commit dessas alterações e tente novamente; o aplicativo \
                 não guarda nem descarta alterações para forçar a integração.",
                unstaged.len()
            ),
            &unstaged,
        ));
    }

    let untracked: Vec<String> = status
        .files
        .iter()
        .filter(|f| f.untracked)
        .map(|f| f.path.clone())
        .collect();
    if relation == MergeRelation::Unrelated {
        blockers.push(blocker(
            BlockerKind::UnrelatedHistories,
            "As duas branches não têm histórico em comum. O aplicativo não integra históricos \
             independentes (isso exigiria uma opção arriscada do Git).",
            &[],
        ));
    }
    if let (false, Some(base)) = (untracked.is_empty(), base.as_deref()) {
        let touched: HashSet<String> = paths_changed(git, root, base, &source_ref)?.into_iter().collect();
        let colliding: Vec<String> = untracked.iter().filter(|p| touched.contains(*p)).cloned().collect();
        if !colliding.is_empty() {
            blockers.push(blocker(
                BlockerKind::UntrackedOverwrite,
                format!(
                    "{} arquivo(s) não rastreados existem na sua pasta e a branch de origem também os \
                     cria. A integração os sobrescreveria. Mova, renomeie ou faça o commit desses \
                     arquivos antes de continuar.",
                    colliding.len()
                ),
                &colliding,
            ));
        } else {
            notes.push(format!(
                "Há {} arquivo(s) não rastreados; eles não são afetados por esta integração.",
                untracked.len()
            ));
        }
    } else if !untracked.is_empty() {
        notes.push(format!(
            "Há {} arquivo(s) não rastreados; eles não são afetados por esta integração.",
            untracked.len()
        ));
    }

    let target_is_current = head.branch.as_deref() == Some(target.name.as_str());
    if !target_is_current {
        let now = head
            .branch
            .as_deref()
            .map(|b| format!("A branch atual é “{b}”."))
            .unwrap_or_else(|| "Nenhuma branch está ativa no momento.".to_string());
        blockers.push(blocker(
            BlockerKind::TargetNotCurrent,
            format!(
                "O merge altera a branch ativa, e o destino escolhido é “{}”. {now} Troque para “{}” \
                 (ação explícita) e revise a integração de novo; o aplicativo não troca de branch sozinho.",
                target.name, target.name
            ),
            &[],
        ));
    }

    match relation {
        MergeRelation::Diverged => match forecast.state {
            ForecastState::Conflicts => notes.push(format!(
                "A simulação do Git prevê conflito em {} arquivo(s). Se acontecer, o merge ficará em \
                 andamento para você resolver manualmente.",
                forecast.files.len()
            )),
            ForecastState::Unavailable => notes.push(
                "Não foi possível simular a integração (requer Git 2.38 ou mais recente). Conflitos só \
                 serão conhecidos ao executar."
                    .to_string(),
            ),
            _ => {}
        },
        _ => {}
    }

    let can_execute = blockers.is_empty()
        && matches!(relation, MergeRelation::FastForward | MergeRelation::Diverged);

    // --- impressão digital da revisão ---
    let mut parts = vec![
        root_text.clone(),
        head.branch.clone().unwrap_or_else(|| "(detached)".into()),
        head.oid.clone().unwrap_or_default(),
        source.name.clone(),
        source.oid.clone(),
        target.name.clone(),
        target.oid.clone(),
        base.clone().unwrap_or_default(),
        format!("{:?}", relation),
        status.operation.map(operation_name).unwrap_or("").to_string(),
    ];
    let mut entries: Vec<String> = status
        .files
        .iter()
        .map(|f| {
            format!(
                "{}|{:?}|{:?}|{}|{}",
                f.path, f.index, f.worktree, f.untracked, f.conflicted
            )
        })
        .collect();
    entries.sort();
    parts.extend(entries);
    let token = fingerprint(&parts);

    let target_protected = is_protected_branch(&target.name);
    Ok(MergePreview {
        repo_path: root_text,
        required_confirmation: target_protected.then(|| target.name.clone()),
        target_protected,
        source,
        target,
        current_branch: head.branch,
        head_oid: head.oid,
        target_is_current,
        relation,
        strategy,
        merge_base: base,
        source_only,
        target_only,
        source_commits,
        forecast,
        summary: status.summary,
        operation: status.operation,
        blockers,
        notes,
        can_execute,
        token,
    })
}

// ----------------------------------------------------------------------------
// Execução
// ----------------------------------------------------------------------------

fn shown_output(raw: &RawOutput) -> String {
    let stderr = String::from_utf8_lossy(&raw.stderr);
    let stdout = String::from_utf8_lossy(&raw.stdout);
    let mut text = stdout.trim().to_string();
    if !stderr.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(stderr.trim());
    }
    truncate_chars(&redact(&text), MAX_OUTPUT_SHOWN_CHARS)
}

fn block_message(blockers: &[MergeBlocker]) -> GitError {
    let first = blockers
        .first()
        .map(|b| b.message.clone())
        .unwrap_or_else(|| "O repositório não está em condições de receber o merge.".to_string());
    let mut details = String::new();
    for b in blockers {
        if !details.is_empty() {
            details.push('\n');
        }
        details.push_str(&format!("• {}", b.message));
        for f in &b.files {
            details.push_str(&format!("\n    {f}"));
        }
        if b.files_total > b.files.len() {
            details.push_str(&format!("\n    … e mais {}", b.files_total - b.files.len()));
        }
    }
    GitError::blocked(first).with_details(details)
}

/// Executa o merge `source → target` **somente** se a confirmação estiver completa e o estado
/// do repositório continuar idêntico ao da prévia revisada.
pub fn merge_branches(git: &Git, request: &MergeRequest) -> Result<MergeResult> {
    let root = require_repository_root(git, &request.repo_path)?;

    // 1. Confirmações (antes de qualquer outra coisa e antes de tocar no repositório).
    if !request.confirmed {
        return Err(GitError::blocked(
            "A integração não foi confirmada. Revise a prévia e confirme explicitamente para executar o merge.",
        ));
    }
    if is_protected_branch(&request.target)
        && request.confirm_target.as_deref() != Some(request.target.as_str())
    {
        return Err(GitError::blocked(format!(
            "“{}” é a branch principal e exige uma confirmação específica: digite “{}” para autorizar \
             a integração.",
            request.target, request.target
        )));
    }

    // 2. Exclusividade e revalidação completa com o estado REAL neste instante.
    let _guard = RepoGuard::acquire(&root)?;
    let preview = build_preview(git, &root, &request.source, &request.target)?;
    if preview.token != request.token {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            "O repositório ou as branches mudaram desde a revisão (outra branch ativa, novos commits, \
             arquivos alterados ou uma operação em andamento). Nada foi alterado. Revise a integração \
             novamente antes de confirmar.",
        ));
    }
    if !preview.blockers.is_empty() {
        return Err(block_message(&preview.blockers));
    }
    let strategy_flag = match preview.relation {
        MergeRelation::FastForward => "--ff-only",
        MergeRelation::Diverged => "--no-ff",
        MergeRelation::Identical | MergeRelation::AlreadyIntegrated => {
            return Err(GitError::blocked(format!(
                "Não há nada a integrar: “{}” já está contida em “{}”.",
                preview.source.name, preview.target.name
            )));
        }
        MergeRelation::Unrelated => {
            return Err(GitError::blocked(
                "As branches não têm histórico em comum e não serão integradas.",
            ));
        }
    };

    // 3. Execução: argumentos separados, sem opções que descartem ou resolvam algo por conta própria.
    let head_before = preview.head_oid.clone();
    let source_ref = head_ref(&preview.source.name);
    // A referência começa com `refs/heads/`, portanto não pode ser confundida com uma opção.
    let args: [&str; 7] = [
        "-c",
        "rerere.enabled=false",
        "merge",
        "--no-edit",
        "--no-autostash",
        strategy_flag,
        source_ref.as_str(),
    ];
    let opts = RunOptions {
        timeout: Duration::from_secs(MERGE_TIMEOUT_SECS),
        max_stdout: 1024 * 1024,
        ok_exit_codes: &[0],
    };
    let raw = match git.run_raw(Some(&root), &args, None, &opts) {
        Ok(raw) => raw,
        Err(err) => {
            // A execução foi interrompida (tempo limite, processo morto…): não presume intacto.
            return Err(GitError::new(
                err.code,
                format!(
                    "{} O repositório pode ter sido modificado parcialmente: atualize e confira o \
                     estado antes de continuar.",
                    err.message
                ),
            )
            .with_details(err.details.unwrap_or_default()));
        }
    };

    // 4. Estado REAL depois da execução.
    let root_text = root.to_string_lossy().into_owned();
    let after = get_status(git, &root_text)?;
    let head_after = head_state(git, &root)?.oid;
    let output = shown_output(&raw);

    let conflicts: Vec<ConflictEntry> = after
        .files
        .iter()
        .filter(|f| f.conflicted)
        .map(|f| ConflictEntry {
            path: f.path.clone(),
            code: f.conflict_code.clone(),
        })
        .collect();

    if raw.exit_code != Some(0) {
        if !conflicts.is_empty() {
            return Ok(MergeResult {
                repo_path: root_text,
                source: preview.source.name,
                target: preview.target.name,
                outcome: MergeOutcome::Conflicts,
                source_oid: preview.source.oid,
                head_before,
                head_after,
                merge_commit: false,
                integrated_commits: preview.source_only,
                conflicts,
                exit_code: raw.exit_code,
                output,
                status: after,
            });
        }
        if after.operation == Some(Operation::Merge) {
            return Err(GitError::new(
                ErrorCode::CommandFailed,
                "O merge ficou em andamento sem conflitos (por exemplo, um hook do repositório recusou \
                 o commit). Nada foi resolvido automaticamente. Conclua com “git commit” ou aborte com \
                 “git merge --abort” e atualize o aplicativo.",
            )
            .with_details(output));
        }
        if head_after != head_before {
            return Err(GitError::new(
                ErrorCode::UnexpectedOutput,
                "O Git terminou com erro, mas o HEAD mudou. Confira o estado do repositório antes de continuar.",
            )
            .with_details(output));
        }
        return Err(classify_merge_failure(&raw, &output));
    }

    // 5. Sucesso declarado pelo Git: confirme que o resultado é o esperado antes de informar.
    let target_ref = head_ref(&preview.target.name);
    let after_branches = read_local_branches(git, &root)?;
    let target_after = after_branches.iter().find(|b| b.name == preview.target.name);
    let source_after = after_branches.iter().find(|b| b.name == preview.source.name);
    let (Some(target_after), Some(source_after)) = (target_after, source_after) else {
        return Err(unexpected("branch desapareceu depois do merge"));
    };
    if source_after.oid != preview.source.oid {
        return Err(unexpected("a branch de origem mudou durante o merge"));
    }
    if Some(&target_after.oid) != head_after.as_ref() {
        return Err(unexpected("o HEAD não corresponde à branch de destino"));
    }
    if head_after == head_before {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git informou sucesso, mas a branch de destino não mudou. Atualize e confira o estado.",
        )
        .with_details(output));
    }
    if !is_ancestor(git, &root, &preview.source.oid, &target_ref)? {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git informou sucesso, mas os commits da origem não estão no destino. Confira o histórico.",
        )
        .with_details(output));
    }

    let parents = {
        let raw = run_read(git, &root, &["rev-list", "--parents", "-n", "1", target_ref.as_str()])?;
        text_of(&raw)
            .split_whitespace()
            .skip(1)
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let (outcome, merge_commit) = match preview.relation {
        MergeRelation::FastForward if target_after.oid == preview.source.oid => {
            (MergeOutcome::FastForward, false)
        }
        MergeRelation::Diverged
            if parents.len() == 2
                && Some(&parents[0]) == head_before.as_ref()
                && parents[1] == preview.source.oid =>
        {
            (MergeOutcome::MergeCommit, true)
        }
        _ => {
            return Err(GitError::new(
                ErrorCode::UnexpectedOutput,
                "O resultado do merge não corresponde ao esperado para esta integração. Confira o histórico.",
            )
            .with_details(output));
        }
    };

    Ok(MergeResult {
        repo_path: root_text,
        source: preview.source.name,
        target: preview.target.name,
        outcome,
        source_oid: preview.source.oid,
        head_before,
        head_after,
        merge_commit,
        integrated_commits: preview.source_only,
        conflicts: Vec::new(),
        exit_code: raw.exit_code,
        output,
        status: after,
    })
}

/// Traduz uma falha do `git merge` que **não** deixou conflitos nem alterou o HEAD.
fn classify_merge_failure(raw: &RawOutput, output: &str) -> GitError {
    let lower = String::from_utf8_lossy(&raw.stderr).to_lowercase()
        + "\n"
        + &String::from_utf8_lossy(&raw.stdout).to_lowercase();
    let message = if lower.contains("would be overwritten") || lower.contains("overwritten by merge") {
        "O Git recusou o merge para não sobrescrever arquivos locais. Nada foi alterado. Faça o commit \
         ou mova esses arquivos e tente novamente."
    } else if lower.contains("unrelated histories") {
        "O Git recusou o merge porque as branches não têm histórico em comum. Nada foi alterado."
    } else if lower.contains("not possible to fast-forward") || lower.contains("diverging branches") {
        "O avanço rápido não é mais possível: o destino recebeu commits depois da revisão. Nada foi \
         alterado. Revise a integração novamente."
    } else if lower.contains("author identity unknown") || lower.contains("unable to auto-detect email") {
        "O Git não sabe quem você é e não pôde criar o commit de merge. Configure seu nome e e-mail \
         (git config --global user.name e user.email) e tente novamente. Nada foi alterado."
    } else if lower.contains("hook") {
        "Um hook do repositório recusou o merge. Nada foi alterado; veja os detalhes."
    } else if lower.contains("gpg") {
        "A assinatura do commit de merge falhou. Verifique a configuração de assinatura (GPG) do Git."
    } else {
        "O Git não concluiu o merge. O repositório não foi alterado; veja os detalhes."
    };
    GitError::new(ErrorCode::CommandFailed, message).with_details(output)
}

// ----------------------------------------------------------------------------
// Testes unitários (funções puras; os cenários com Git real estão em tests/phase5.rs)
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_and_master_are_protected() {
        assert!(is_protected_branch("main"));
        assert!(is_protected_branch("master"));
        assert!(!is_protected_branch("feature/main"));
        assert!(!is_protected_branch("Main"));
    }

    #[test]
    fn fingerprint_changes_with_any_part() {
        let a = fingerprint(&["x".into(), "y".into()]);
        assert_eq!(a, fingerprint(&["x".into(), "y".into()]));
        assert_ne!(a, fingerprint(&["x".into(), "z".into()]));
        // Separador evita que ["ab","c"] e ["a","bc"] colidam.
        assert_ne!(fingerprint(&["ab".into(), "c".into()]), fingerprint(&["a".into(), "bc".into()]));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn validate_branches_rejects_bad_selections() {
        let entry = |name: &str| BranchEntry {
            name: name.into(),
            current: false,
            oid: "1".into(),
            upstream: None,
            subject: None,
        };
        let list = vec![entry("main"), entry("feature/x")];
        assert!(validate_branches(&list, "feature/x", "main").is_ok());
        assert!(validate_branches(&list, "main", "main").is_err());
        assert!(validate_branches(&list, "nao-existe", "main").is_err());
        assert!(validate_branches(&list, "feature/x", "nao-existe").is_err());
        assert!(validate_branches(&list, "-rf", "main").is_err());
        assert!(validate_branches(&list, "", "main").is_err());
    }
}
