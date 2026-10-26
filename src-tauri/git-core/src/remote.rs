//! Sincronização remota (Fase 3): diagnóstico de remotos/upstream e as operações predefinidas
//! `fetch`, `pull` e `push`.
//!
//! Regras desta camada:
//! - o Git é chamado sem shell e com argumentos separados (o remoto vem de uma lista fechada:
//!   só aceitamos nomes que o próprio Git informa em `git remote`);
//! - **nenhuma** operação usa `--force`, `--force-with-lease`, `--all`, `--mirror`, `--tags`,
//!   `reset`, `clean`, `stash`, `checkout --` nem altera configurações do Git;
//! - `push` usa um refspec explícito (`refs/heads/<local>:refs/heads/<remota>`) derivado do
//!   upstream já configurado: nunca cria nem altera upstream, nunca usa `-u`;
//! - antes de executar, o estado é lido de novo (dentro da trava do repositório) e comparado com o
//!   destino que o usuário confirmou; se algo mudou, a operação é recusada (`StaleReview`);
//! - o resultado nunca é presumido: o código de saída, o HEAD e as referências são conferidos;
//! - credenciais: o runner mantém `GIT_TERMINAL_PROMPT=0`. A autenticação é a que o próprio Git
//!   já tem configurada (Git Credential Manager, SSH). Nada é solicitado, gravado ou logado, e
//!   URLs/saídas passam por `sanitize_url`/`redact` antes de chegar à interface.

use crate::{
    branches::{head_oid, head_state, operation_name},
    error::{truncate_chars, ErrorCode, GitError, Result},
    lock::RepoGuard,
    repository::require_repository_root,
    runner::{Git, RawOutput, RunOptions},
    status::{get_status, RepositoryStatus},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    time::{Duration, UNIX_EPOCH},
};

/// Tempo máximo de uma operação de rede (fetch/pull/push). Operações locais usam limites bem
/// menores; o limite global do runner continua valendo para todas as consultas.
const NETWORK_TIMEOUT_SECS: u64 = 300;
const MAX_OUTPUT_SHOWN_CHARS: usize = 4000;
const MAX_BLOCKING_FILES_LISTED: usize = 12;
/// Branches para as quais o envio exige uma confirmação adicional e explícita.
const PROTECTED_NAMES: &[&str] = &["main", "master"];

// ----------------------------------------------------------------------------
// Modelos
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub name: String,
    /// URL de busca já higienizada (sem usuário/senha em URLs HTTP).
    pub fetch_url: Option<String>,
    pub push_url: Option<String>,
}

/// Situação da branch atual em relação ao upstream, **segundo as referências locais**.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    /// Nenhum remoto configurado.
    NoRemote,
    /// HEAD desanexado: não há branch para sincronizar.
    DetachedHead,
    /// Branch sem commits.
    Unborn,
    /// Há remoto, mas a branch atual não tem upstream.
    NoUpstream,
    /// O upstream aponta para um remoto inexistente ou local (`.`), ou tem formato inesperado.
    UpstreamInvalid,
    /// Upstream configurado, mas a referência remota ainda não existe localmente (falta fetch,
    /// ou a branch remota foi removida).
    UpstreamNotFetched,
    Synced,
    Ahead,
    Behind,
    Diverged,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PullStrategy {
    Merge,
    Rebase,
    FastForwardOnly,
    /// Nenhuma estratégia configurada pelo usuário.
    Unspecified,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamInfo {
    pub remote: String,
    /// Nome da branch no remoto (ex.: `main`).
    pub remote_branch: String,
    /// Nome de exibição (ex.: `origin/main`).
    pub display: String,
    /// Referência local de acompanhamento (ex.: `refs/remotes/origin/main`); pode estar vazia.
    pub tracking_ref: String,
    pub tracking_ref_exists: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub repo_path: String,
    pub branch: Option<String>,
    pub head_oid: Option<String>,
    pub remotes: Vec<RemoteInfo>,
    pub upstream: Option<UpstreamInfo>,
    pub state: SyncState,
    /// Commits locais que o upstream (conforme as referências locais) ainda não tem.
    pub ahead: Option<u32>,
    /// Commits do upstream (conforme as referências locais) que a branch local não tem.
    pub behind: Option<u32>,
    pub pull_strategy: PullStrategy,
    /// Segundos desde a época Unix da última gravação de `FETCH_HEAD` (último fetch/pull).
    pub last_fetch_at: Option<u64>,
}

/// Destino que o usuário viu e confirmou. O backend recalcula o destino e recusa se mudou.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncTarget {
    pub branch: String,
    pub remote: String,
    pub remote_branch: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchRequest {
    pub repo_path: String,
    pub remote: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchResult {
    pub repo_path: String,
    pub remote: String,
    /// Referências de `refs/remotes/<remoto>/` que foram criadas, movidas ou removidas.
    pub updated_refs: usize,
    pub output: String,
    pub sync: SyncStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub repo_path: String,
    pub target: SyncTarget,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PullOutcome {
    AlreadyUpToDate,
    Updated,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    pub repo_path: String,
    pub target: SyncTarget,
    pub outcome: PullOutcome,
    /// `true` quando o HEAD resultante é um commit de merge.
    pub merge_commit: bool,
    pub head_before: Option<String>,
    pub head_after: Option<String>,
    pub output: String,
    pub sync: SyncStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub repo_path: String,
    pub target: SyncTarget,
    /// Confirmação adicional exigida quando o destino é `main` ou `master`.
    #[serde(default)]
    pub confirm_protected_branch: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub repo_path: String,
    pub target: SyncTarget,
    /// Quantos commits locais o upstream não tinha antes do envio.
    pub pushed_commits: u32,
    pub local_oid: String,
    pub output: String,
    pub sync: SyncStatus,
}

// ----------------------------------------------------------------------------
// Opções de execução
// ----------------------------------------------------------------------------

fn query_opts() -> RunOptions<'static> {
    RunOptions {
        timeout: Duration::from_secs(15),
        max_stdout: 1024 * 1024,
        ok_exit_codes: &[0],
    }
}

/// Para consultas em que o código 1 significa "não existe" (`config --get`, `rev-parse --verify`).
fn quiet_opts() -> RunOptions<'static> {
    RunOptions {
        timeout: Duration::from_secs(15),
        max_stdout: 64 * 1024,
        ok_exit_codes: &[0, 1],
    }
}

fn run_query(git: &Git, root: &Path, args: &[&str]) -> Result<String> {
    let out = git.run(Some(root), args, &query_opts())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Executa uma operação de rede com o limite de tempo padrão (`NETWORK_TIMEOUT_SECS`).
fn run_network(git: &Git, root: &Path, args: &[&str]) -> Result<RawOutput> {
    run_with_timeout(git, root, args, Duration::from_secs(NETWORK_TIMEOUT_SECS))
}

/// O tempo limite recebe uma mensagem própria: o processo foi encerrado, mas o servidor pode ter
/// concluído o trabalho, então o resultado é "não confirmado" (nunca "falhou" nem "sucesso").
fn run_with_timeout(
    git: &Git,
    root: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<RawOutput> {
    let opts = RunOptions {
        timeout,
        max_stdout: 1024 * 1024,
        ok_exit_codes: &[0],
    };
    git.run_raw(Some(root), args, None, &opts).map_err(|err| {
        if err.code == ErrorCode::Timeout {
            GitError::new(
                ErrorCode::Timeout,
                format!(
                    "A operação de rede passou de {} s e foi interrompida. O resultado NÃO foi \
                     confirmado: atualize o diagnóstico de sincronização e confira o estado \
                     antes de tentar de novo.",
                    timeout.as_secs().max(1)
                ),
            )
        } else {
            err
        }
    })
}

// ----------------------------------------------------------------------------
// Higienização de URLs e saídas
// ----------------------------------------------------------------------------

/// Remove credenciais de uma URL. Em `http(s)://usuario:senha@host/...` o usuário também é
/// removido; em `ssh://usuario:senha@host` só a senha. URLs no formato `git@host:caminho` não
/// carregam segredo e ficam como estão.
pub fn sanitize_url(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, tail) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            let host = &authority[at + 1..];
            let scheme_lower = scheme.to_ascii_lowercase();
            let keeps_user = scheme_lower.ends_with("ssh");
            if keeps_user {
                let user = userinfo.split(':').next().unwrap_or("");
                return format!("{scheme}://{user}@{host}{tail}");
            }
            return format!("{scheme}://{host}{tail}");
        }
    }
    url.to_string()
}

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    for marker in ["authorization:", "set-cookie:", "cookie:"] {
        if let Some(pos) = lower.find(marker) {
            return format!("{}{} [oculto]", &line[..pos], &line[pos..pos + marker.len()]);
        }
    }
    line.split(' ')
        .map(|word| {
            if word.contains("://") {
                sanitize_url(word)
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Remove credenciais e cabeçalhos de autenticação de uma saída do Git antes de exibi-la.
pub fn redact(text: &str) -> String {
    text.lines().map(redact_line).collect::<Vec<_>>().join("\n")
}

fn combined_output(raw: &RawOutput) -> String {
    let stderr = String::from_utf8_lossy(&raw.stderr);
    let stdout = String::from_utf8_lossy(&raw.stdout);
    let mut text = stderr.trim().to_string();
    if !stdout.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(stdout.trim());
    }
    text
}

fn shown_output(raw: &RawOutput) -> String {
    truncate_chars(&redact(&combined_output(raw)), MAX_OUTPUT_SHOWN_CHARS)
}

// ----------------------------------------------------------------------------
// Diagnóstico
// ----------------------------------------------------------------------------

/// Interpreta `git remote -v`: `nome<TAB>url (fetch|push)`. Devolve `(nome, url, é_push)`.
pub fn parse_remote_verbose(text: &str) -> Vec<(String, String, bool)> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('\t')?;
            let (url, kind) = rest.rsplit_once(' ')?;
            let is_push = match kind {
                "(fetch)" => false,
                "(push)" => true,
                _ => return None,
            };
            Some((name.to_string(), url.to_string(), is_push))
        })
        .collect()
}

fn list_remotes(git: &Git, root: &Path) -> Result<Vec<RemoteInfo>> {
    let names = run_query(git, root, &["remote"])?;
    let verbose = run_query(git, root, &["remote", "-v"])?;
    let entries = parse_remote_verbose(&verbose);
    let mut remotes = Vec::new();
    for name in names.lines().map(str::trim).filter(|n| !n.is_empty()) {
        let url_of = |push: bool| {
            entries
                .iter()
                .find(|e| e.0 == name && e.2 == push)
                .map(|e| sanitize_url(&e.1))
        };
        remotes.push(RemoteInfo {
            name: name.to_string(),
            fetch_url: url_of(false),
            push_url: url_of(true),
        });
    }
    Ok(remotes)
}

/// Só aceita um remoto que o Git realmente lista (nunca texto livre vindo da interface).
fn require_known_remote(remotes: &[RemoteInfo], name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('-') || name.contains(['\0', '\n', '\r']) {
        return Err(GitError::invalid_argument("O nome do remoto é inválido."));
    }
    if !remotes.iter().any(|r| r.name == name) {
        return Err(GitError::invalid_argument(
            "O remoto informado não está configurado neste repositório.",
        ));
    }
    Ok(())
}

fn config_get(git: &Git, root: &Path, key: &str) -> Result<Option<String>> {
    let out = git.run(Some(root), &["config", "--get", key], &quiet_opts())?;
    if out.exit_code != 0 {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_ascii_lowercase();
    Ok(if value.is_empty() { None } else { Some(value) })
}

/// Estratégia de pull que o **usuário** configurou (a precedência é a do próprio Git).
pub fn classify_pull_strategy(
    branch_rebase: Option<&str>,
    pull_rebase: Option<&str>,
    pull_ff: Option<&str>,
) -> PullStrategy {
    fn rebase_value(value: &str) -> PullStrategy {
        match value {
            "false" | "no" | "off" | "0" => PullStrategy::Merge,
            _ => PullStrategy::Rebase,
        }
    }
    match (branch_rebase, pull_rebase, pull_ff) {
        (Some(v), _, _) | (None, Some(v), _) => rebase_value(v),
        (None, None, Some("only")) => PullStrategy::FastForwardOnly,
        (None, None, Some(_)) => PullStrategy::Merge,
        (None, None, None) => PullStrategy::Unspecified,
    }
}

fn read_pull_strategy(git: &Git, root: &Path, branch: Option<&str>) -> Result<PullStrategy> {
    let branch_rebase = match branch {
        Some(name) => config_get(git, root, &format!("branch.{name}.rebase"))?,
        None => None,
    };
    let pull_rebase = config_get(git, root, "pull.rebase")?;
    let pull_ff = config_get(git, root, "pull.ff")?;
    Ok(classify_pull_strategy(
        branch_rebase.as_deref(),
        pull_rebase.as_deref(),
        pull_ff.as_deref(),
    ))
}

struct RawUpstream {
    tracking_ref: String,
    short: String,
    remote: String,
    remote_ref: String,
}

fn read_upstream(git: &Git, root: &Path, branch: &str) -> Result<Option<RawUpstream>> {
    let full = format!("refs/heads/{branch}");
    let args = [
        "for-each-ref",
        "--format=%(refname)%00%(upstream)%00%(upstream:short)%00%(upstream:remotename)%00%(upstream:remoteref)",
        full.as_str(),
    ];
    let text = run_query(git, root, &args)?;
    let Some(line) = text.lines().next() else {
        return Ok(None);
    };
    let parts: Vec<&str> = line.split('\0').collect();
    if parts.len() != 5 {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git informou o upstream em um formato inesperado.",
        ));
    }
    if parts[0] != full || parts[3].is_empty() || parts[4].is_empty() {
        return Ok(None);
    }
    Ok(Some(RawUpstream {
        tracking_ref: parts[1].to_string(),
        short: parts[2].to_string(),
        remote: parts[3].to_string(),
        remote_ref: parts[4].to_string(),
    }))
}

fn ref_oid(git: &Git, root: &Path, reference: &str) -> Result<Option<String>> {
    let out = git.run(
        Some(root),
        &["rev-parse", "--verify", "-q", reference],
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

fn ahead_behind(git: &Git, root: &Path, tracking_ref: &str) -> Result<(u32, u32)> {
    let range = format!("HEAD...{tracking_ref}");
    let text = run_query(
        git,
        root,
        &["rev-list", "--left-right", "--count", range.as_str()],
    )?;
    let mut numbers = text.split_whitespace().map(|n| n.parse::<u32>());
    match (numbers.next(), numbers.next()) {
        (Some(Ok(ahead)), Some(Ok(behind))) => Ok((ahead, behind)),
        _ => Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git informou a comparação com o upstream em um formato inesperado.",
        )),
    }
}

fn read_last_fetch_at(git: &Git, root: &Path) -> Option<u64> {
    let text = run_query(git, root, &["rev-parse", "--absolute-git-dir"]).ok()?;
    let git_dir = Path::new(text.trim_end_matches(['\n', '\r']));
    let modified = fs::metadata(git_dir.join("FETCH_HEAD")).ok()?.modified().ok()?;
    modified.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn state_from_counts(ahead: u32, behind: u32) -> SyncState {
    match (ahead, behind) {
        (0, 0) => SyncState::Synced,
        (_, 0) => SyncState::Ahead,
        (0, _) => SyncState::Behind,
        _ => SyncState::Diverged,
    }
}

fn read_sync_status(git: &Git, root: &Path) -> Result<SyncStatus> {
    let remotes = list_remotes(git, root)?;
    let head = head_state(git, root)?;
    let pull_strategy = read_pull_strategy(git, root, head.branch.as_deref())?;

    let mut sync = SyncStatus {
        repo_path: root.to_string_lossy().into_owned(),
        branch: head.branch.clone(),
        head_oid: head.oid.clone(),
        remotes,
        upstream: None,
        state: SyncState::NoRemote,
        ahead: None,
        behind: None,
        pull_strategy,
        last_fetch_at: read_last_fetch_at(git, root),
    };

    if sync.remotes.is_empty() {
        return Ok(sync);
    }
    let Some(branch) = head.branch.as_deref() else {
        sync.state = SyncState::DetachedHead;
        return Ok(sync);
    };
    if head.oid.is_none() {
        sync.state = SyncState::Unborn;
        return Ok(sync);
    }
    let Some(raw) = read_upstream(git, root, branch)? else {
        sync.state = SyncState::NoUpstream;
        return Ok(sync);
    };

    let remote_branch = raw
        .remote_ref
        .strip_prefix("refs/heads/")
        .unwrap_or("")
        .to_string();
    let valid = raw.remote != "."
        && !remote_branch.is_empty()
        && sync.remotes.iter().any(|r| r.name == raw.remote);
    let tracking_exists =
        valid && !raw.tracking_ref.is_empty() && ref_oid(git, root, &raw.tracking_ref)?.is_some();

    sync.upstream = Some(UpstreamInfo {
        remote: raw.remote.clone(),
        remote_branch,
        display: raw.short.clone(),
        tracking_ref: raw.tracking_ref.clone(),
        tracking_ref_exists: tracking_exists,
    });

    if !valid {
        sync.state = SyncState::UpstreamInvalid;
        return Ok(sync);
    }
    if !tracking_exists {
        sync.state = SyncState::UpstreamNotFetched;
        return Ok(sync);
    }
    let (ahead, behind) = ahead_behind(git, root, &raw.tracking_ref)?;
    sync.ahead = Some(ahead);
    sync.behind = Some(behind);
    sync.state = state_from_counts(ahead, behind);
    Ok(sync)
}

/// Remotos, upstream e situação de sincronização da branch atual (somente leitura; não acessa a rede).
pub fn get_sync_status(git: &Git, repo_path: &str) -> Result<SyncStatus> {
    let root = require_repository_root(git, repo_path)?;
    read_sync_status(git, &root)
}

// ----------------------------------------------------------------------------
// Pré-condições comuns
// ----------------------------------------------------------------------------

/// A branch atual precisa ter um upstream utilizável para pull e push.
fn ensure_usable_upstream(sync: &SyncStatus, action: &str) -> Result<()> {
    let message = match sync.state {
        SyncState::NoRemote => format!(
            "Este repositório não tem remoto configurado, então não há para onde {action}. \
             Adicione um remoto fora do aplicativo (por exemplo, “git remote add origin <url>”)."
        ),
        SyncState::DetachedHead => format!(
            "O HEAD está desanexado (sem branch). Troque para uma branch antes de {action}."
        ),
        SyncState::Unborn => format!(
            "A branch atual ainda não tem commits. Crie um commit antes de {action}."
        ),
        SyncState::NoUpstream => format!(
            "A branch atual não tem branch remota de acompanhamento (upstream), e o aplicativo \
             não define upstream sozinho. Defina-o fora do aplicativo (por exemplo, \
             “git branch --set-upstream-to=<remoto>/<branch>”) antes de {action}."
        ),
        SyncState::UpstreamInvalid => format!(
            "O upstream configurado para esta branch aponta para um remoto que não existe ou para \
             uma branch local. Corrija a configuração fora do aplicativo antes de {action}."
        ),
        _ => return Ok(()),
    };
    Err(GitError::blocked(message))
}

/// O destino confirmado pelo usuário precisa ser exatamente o destino atual.
fn verify_target(sync: &SyncStatus, target: &SyncTarget) -> Result<UpstreamInfo> {
    let (Some(branch), Some(upstream)) = (sync.branch.as_deref(), sync.upstream.as_ref()) else {
        return Err(GitError::blocked(
            "Não foi possível determinar o destino da sincronização.",
        ));
    };
    if branch != target.branch
        || upstream.remote != target.remote
        || upstream.remote_branch != target.remote_branch
    {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            "O destino mudou desde a confirmação (branch ou upstream diferentes). Nada foi \
             executado: atualize o diagnóstico e confirme de novo.",
        ));
    }
    Ok(upstream.clone())
}

fn ensure_no_operation_or_conflict(status: &RepositoryStatus, action: &str) -> Result<()> {
    if let Some(operation) = status.operation {
        return Err(GitError::blocked(format!(
            "Há um {} em andamento. Conclua ou cancele essa operação antes de {action}.",
            operation_name(operation)
        )));
    }
    if status.summary.conflicted > 0 {
        return Err(GitError::blocked(format!(
            "Há arquivos em conflito. Resolva os conflitos antes de {action}."
        )));
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Fetch
// ----------------------------------------------------------------------------

fn remote_ref_snapshot(git: &Git, root: &Path, remote: &str) -> Result<HashMap<String, String>> {
    let pattern = format!("refs/remotes/{remote}/");
    let text = run_query(
        git,
        root,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            pattern.as_str(),
        ],
    )?;
    Ok(text
        .lines()
        .filter_map(|line| line.rsplit_once(' '))
        .map(|(name, oid)| (name.to_string(), oid.to_string()))
        .collect())
}

fn count_changed_refs(before: &HashMap<String, String>, after: &HashMap<String, String>) -> usize {
    let changed_or_new = after
        .iter()
        .filter(|(name, oid)| before.get(*name) != Some(*oid))
        .count();
    let removed = before.keys().filter(|name| !after.contains_key(*name)).count();
    changed_or_new + removed
}

/// Atualiza as referências de um remoto (`git fetch <remoto>`). Não altera arquivos de trabalho,
/// branches locais nem cria commits, e não integra nada (isso é o `pull`).
pub fn fetch_remote(git: &Git, request: &FetchRequest) -> Result<FetchResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;

    let remotes = list_remotes(git, &root)?;
    require_known_remote(&remotes, &request.remote)?;

    let before = remote_ref_snapshot(git, &root, &request.remote)?;
    let raw = run_network(
        git,
        &root,
        &["fetch", "--no-recurse-submodules", request.remote.as_str()],
    )?;
    if raw.exit_code != Some(0) {
        return Err(classify_remote_failure(RemoteOp::Fetch, &raw));
    }
    let after = remote_ref_snapshot(git, &root, &request.remote)?;

    Ok(FetchResult {
        repo_path: root.to_string_lossy().into_owned(),
        remote: request.remote.clone(),
        updated_refs: count_changed_refs(&before, &after),
        output: shown_output(&raw),
        sync: read_sync_status(git, &root)?,
    })
}

// ----------------------------------------------------------------------------
// Pull
// ----------------------------------------------------------------------------

fn ensure_pull_allowed(sync: &SyncStatus, status: &RepositoryStatus) -> Result<()> {
    ensure_usable_upstream(sync, "baixar alterações")?;
    ensure_no_operation_or_conflict(status, "baixar alterações")?;

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
            "Há {} arquivo(s) com alterações ainda não commitadas. O pull foi bloqueado para não \
             arriscar o seu trabalho. Faça o commit dessas alterações, ou guarde-as/descarte-as \
             manualmente (por exemplo, com “git stash”), e tente novamente.",
            at_risk.len()
        ))
        .with_details(details));
    }

    if sync.state == SyncState::Diverged {
        match sync.pull_strategy {
            PullStrategy::Unspecified => {
                return Err(GitError::blocked(
                    "A branch divergiu do upstream (há commits dos dois lados) e nenhuma estratégia \
                     de pull está configurada. O aplicativo não escolhe entre merge e rebase por \
                     você: defina “pull.rebase” fora do aplicativo (false = merge, true = rebase) \
                     ou integre manualmente, e tente novamente.",
                ));
            }
            PullStrategy::FastForwardOnly => {
                return Err(GitError::blocked(
                    "A branch divergiu do upstream e o seu Git está configurado para aceitar apenas \
                     avanço rápido (pull.ff = only), o que é impossível neste caso. Integre \
                     manualmente (merge ou rebase) e tente novamente.",
                ));
            }
            PullStrategy::Merge | PullStrategy::Rebase => {}
        }
    }
    Ok(())
}

fn is_merge_commit(git: &Git, root: &Path) -> Result<bool> {
    let text = run_query(git, root, &["rev-list", "--parents", "-n", "1", "HEAD"])?;
    Ok(text.split_whitespace().count() > 2)
}

/// Baixa e integra as alterações do upstream (`git pull <remoto> <branch>`).
///
/// Bloqueado quando há alterações locais em arquivos rastreados, conflitos ou outra operação em
/// andamento. A estratégia é a configurada pelo usuário; sem nenhuma configurada, o pull só aceita
/// avanço rápido (`--ff-only`), o que nunca cria merge nem rebase. Conflitos não são resolvidos.
pub fn pull_branch(git: &Git, request: &PullRequest) -> Result<PullResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;
    let root_text = root.to_string_lossy().into_owned();

    let sync = read_sync_status(git, &root)?;
    ensure_usable_upstream(&sync, "baixar alterações")?;
    let upstream = verify_target(&sync, &request.target)?;
    let status = get_status(git, &root_text)?;
    ensure_pull_allowed(&sync, &status)?;

    let head_before = head_oid(git, &root)?;
    let mut args: Vec<&str> = vec!["pull", "--no-edit", "--no-recurse-submodules"];
    if sync.pull_strategy == PullStrategy::Unspecified {
        args.push("--ff-only");
    }
    args.push(upstream.remote.as_str());
    args.push(upstream.remote_branch.as_str());

    let raw = run_network(git, &root, &args)?;
    if raw.exit_code != Some(0) {
        // A prova de conflito é o estado real do repositório, não o texto da mensagem.
        if let Ok(after) = get_status(git, &root_text) {
            if after.operation.is_some() || after.summary.conflicted > 0 {
                return Err(GitError::new(
                    ErrorCode::MergeConflict,
                    "A integração gerou conflitos e foi interrompida. O repositório está em estado \
                     de integração e nada foi resolvido automaticamente. Resolva os conflitos \
                     manualmente (ou aborte com “git merge --abort” / “git rebase --abort”) e \
                     atualize o aplicativo.",
                )
                .with_details(redact(&combined_output(&raw))));
            }
        }
        return Err(classify_remote_failure(RemoteOp::Pull, &raw));
    }

    let head_after = head_oid(git, &root)?;
    let outcome = if head_before == head_after {
        PullOutcome::AlreadyUpToDate
    } else {
        PullOutcome::Updated
    };
    let merge_commit = outcome == PullOutcome::Updated && is_merge_commit(git, &root)?;

    Ok(PullResult {
        repo_path: root_text,
        target: request.target.clone(),
        outcome,
        merge_commit,
        head_before,
        head_after,
        output: shown_output(&raw),
        sync: read_sync_status(git, &root)?,
    })
}

// ----------------------------------------------------------------------------
// Push
// ----------------------------------------------------------------------------

fn ensure_push_allowed(sync: &SyncStatus, status: &RepositoryStatus) -> Result<u32> {
    ensure_usable_upstream(sync, "enviar commits")?;
    ensure_no_operation_or_conflict(status, "enviar commits")?;

    let display = sync
        .upstream
        .as_ref()
        .map(|u| u.display.clone())
        .unwrap_or_default();
    match sync.state {
        SyncState::UpstreamNotFetched => Err(GitError::blocked(format!(
            "A branch remota {display} ainda não existe localmente (nunca foi buscada ou foi \
             removida no remoto). Faça um fetch para atualizar as referências antes de enviar; o \
             aplicativo não cria branches remotas sozinho."
        ))),
        SyncState::Behind => Err(GitError::blocked(format!(
            "O upstream {display} tem commits que a sua branch não tem e não há nada local para \
             enviar. Baixe as alterações (pull) antes."
        ))),
        SyncState::Diverged => Err(GitError::blocked(format!(
            "A branch divergiu de {display}: há commits novos nos dois lados, e o remoto \
             rejeitaria o envio. Integre as alterações do remoto (pull) antes de enviar."
        ))),
        SyncState::Synced => Err(GitError::blocked(format!(
            "Não há commits locais para enviar: a branch está sincronizada com {display} \
             (conforme o último fetch)."
        ))),
        SyncState::Ahead => Ok(sync.ahead.unwrap_or(0)),
        _ => Err(GitError::blocked("A branch não está pronta para o envio.")),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PushLine {
    pub flag: char,
    pub refs: String,
    pub summary: String,
}

/// Interpreta a saída de `git push --porcelain`: `<flag>\t<origem>:<destino>\t<resumo>`.
pub fn parse_push_porcelain(stdout: &str) -> Vec<PushLine> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let flag = parts.next()?;
            let refs = parts.next()?;
            let summary = parts.next().unwrap_or("");
            if flag.chars().count() != 1 {
                return None;
            }
            Some(PushLine {
                flag: flag.chars().next()?,
                refs: refs.to_string(),
                summary: summary.to_string(),
            })
        })
        .collect()
}

/// Envia os commits da branch atual para o upstream já configurado
/// (`git push --porcelain <remoto> refs/heads/<local>:refs/heads/<remota>`).
///
/// Nunca usa `--force`, `-u`, `--all`, `--mirror` nem `--tags`. Para `main`/`master` exige a
/// confirmação adicional `confirm_protected_branch`.
pub fn push_branch(git: &Git, request: &PushRequest) -> Result<PushResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;
    let root_text = root.to_string_lossy().into_owned();

    let sync = read_sync_status(git, &root)?;
    ensure_usable_upstream(&sync, "enviar commits")?;
    let upstream = verify_target(&sync, &request.target)?;
    let status = get_status(git, &root_text)?;
    let pushed_commits = ensure_push_allowed(&sync, &status)?;

    if PROTECTED_NAMES.contains(&upstream.remote_branch.as_str())
        && !request.confirm_protected_branch
    {
        return Err(GitError::blocked(format!(
            "O destino é a branch “{}” do remoto “{}”. O envio para essa branch exige uma \
             confirmação adicional e explícita.",
            upstream.remote_branch, upstream.remote
        )));
    }

    let local_oid = sync.head_oid.clone().unwrap_or_default();
    let refspec = format!(
        "refs/heads/{}:refs/heads/{}",
        request.target.branch, upstream.remote_branch
    );
    let raw = run_network(
        git,
        &root,
        &[
            "push",
            "--porcelain",
            "--no-recurse-submodules",
            upstream.remote.as_str(),
            refspec.as_str(),
        ],
    )?;

    let lines = parse_push_porcelain(&String::from_utf8_lossy(&raw.stdout));
    let rejected = lines.iter().any(|l| l.flag == '!');
    if raw.exit_code != Some(0) || rejected {
        return Err(classify_remote_failure(RemoteOp::Push, &raw));
    }

    Ok(PushResult {
        repo_path: root_text,
        target: request.target.clone(),
        pushed_commits,
        local_oid,
        output: shown_output(&raw),
        sync: read_sync_status(git, &root)?,
    })
}

// ----------------------------------------------------------------------------
// Classificação de falhas
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteOp {
    Fetch,
    Pull,
    Push,
}

impl RemoteOp {
    fn goal(self) -> &'static str {
        match self {
            RemoteOp::Fetch => "atualizar as referências remotas",
            RemoteOp::Pull => "baixar e integrar as alterações",
            RemoteOp::Push => "enviar os commits",
        }
    }
}

fn has_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Traduz uma falha de fetch/pull/push em um erro compreensível. Usa o código de saída e as
/// evidências devolvidas pelo Git (linhas `--porcelain` do push e padrões estáveis do transporte);
/// quando nada é reconhecido, devolve uma mensagem genérica com os detalhes higienizados.
fn classify_remote_failure(op: RemoteOp, raw: &RawOutput) -> GitError {
    let details = redact(&combined_output(raw));
    let lower = details.to_lowercase();
    let fail = |code: ErrorCode, message: &str| GitError::new(code, message).with_details(&details);

    if raw.exit_code.is_none() {
        return fail(
            ErrorCode::CommandFailed,
            "O Git foi encerrado antes de terminar (sem código de saída normal). O resultado NÃO \
             foi confirmado: atualize o diagnóstico e confira o estado antes de tentar de novo.",
        );
    }
    if has_any(&lower, &["index.lock", "another git process seems to be running"]) {
        return fail(
            ErrorCode::RepositoryBusy,
            "Outro processo do Git está usando este repositório. Aguarde ele terminar e tente \
             novamente.",
        );
    }

    if op == RemoteOp::Push {
        let stdout = String::from_utf8_lossy(&raw.stdout);
        let rejects: Vec<PushLine> = parse_push_porcelain(&stdout)
            .into_iter()
            .filter(|l| l.flag == '!')
            .collect();
        if !rejects.is_empty() {
            let reason = rejects
                .iter()
                .map(|l| l.summary.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let all = format!("{reason} {lower}");
            let message = if has_any(&reason, &["non-fast-forward", "fetch first", "stale info"]) {
                "O remoto recusou o envio porque tem commits que a sua branch ainda não tem \
                 (outra pessoa enviou antes, ou o seu estado remoto estava desatualizado). Nada foi \
                 enviado. Faça um fetch e depois um pull para integrar as alterações e tente de \
                 novo."
            } else if has_any(&all, &["protected branch", "gh006", "protected"]) {
                "O remoto recusou o envio porque a branch de destino é protegida. Nada foi \
                 enviado. Use outra branch ou o fluxo de revisão exigido pelo repositório."
            } else if has_any(&all, &["hook declined", "pre-receive", "update hook"]) {
                "Uma regra do servidor (hook) recusou o envio. Nada foi enviado. Veja os detalhes \
                 técnicos para saber o motivo."
            } else {
                "O remoto recusou o envio. Nada foi enviado. Veja os detalhes técnicos para saber \
                 o motivo."
            };
            return fail(ErrorCode::PushRejected, message);
        }
    }

    if op == RemoteOp::Pull {
        if has_any(
            &lower,
            &[
                "would be overwritten by merge",
                "would be overwritten by checkout",
                "your local changes to the following files would be overwritten",
                "untracked working tree files would be overwritten",
                "please commit your changes or stash them",
                "cannot pull with rebase",
            ],
        ) {
            return fail(
                ErrorCode::OperationBlocked,
                "O Git recusou o pull porque ele sobrescreveria arquivos locais. Nada foi alterado. \
                 Mova, commite ou guarde esses arquivos manualmente e tente de novo.",
            );
        }
        if has_any(
            &lower,
            &[
                "not possible to fast-forward",
                "need to specify how to reconcile divergent branches",
                "diverging branches",
            ],
        ) {
            return fail(
                ErrorCode::OperationBlocked,
                "A branch local e o upstream divergiram e o Git não pode integrá-los sem uma \
                 estratégia (merge ou rebase). Nada foi alterado. Defina “pull.rebase” fora do \
                 aplicativo ou integre manualmente.",
            );
        }
        if has_any(&lower, &["unrelated histories"]) {
            return fail(
                ErrorCode::OperationBlocked,
                "O Git recusou o pull porque os históricos não têm origem em comum. Nada foi \
                 alterado.",
            );
        }
        if has_any(&lower, &["unmerged paths", "unresolved conflict"]) {
            return fail(
                ErrorCode::OperationBlocked,
                "Há conflitos não resolvidos no repositório. Resolva-os antes de fazer pull.",
            );
        }
    }

    if has_any(
        &lower,
        &[
            "ssl certificate problem",
            "certificate verify failed",
            "unable to get local issuer certificate",
            "self signed certificate",
            "self-signed certificate",
            "server certificate verification failed",
            "ssl_connect",
            "schannel: ",
            "gnutls_handshake",
            "tls connection",
        ],
    ) {
        return fail(
            ErrorCode::TlsError,
            "A conexão segura (TLS/HTTPS) com o remoto foi recusada por causa do certificado. \
             Verifique a data do computador, o proxy/antivírus da rede e o certificado do servidor. \
             O aplicativo não desativa a verificação de certificados.",
        );
    }

    if has_any(&lower, &["host key verification failed", "no matching host key"]) {
        return fail(
            ErrorCode::AuthFailed,
            "O SSH não reconhece este servidor (verificação da chave do host). Conecte-se uma vez \
             pelo terminal (por exemplo, “ssh -T git@<servidor>”) para confirmar a chave e tente \
             novamente.",
        );
    }
    if has_any(
        &lower,
        &[
            "could not read username",
            "could not read password",
            "terminal prompts disabled",
        ],
    ) {
        return fail(
            ErrorCode::AuthFailed,
            "O remoto pediu usuário e senha, mas o aplicativo não consegue digitá-los (prompts \
             interativos estão desativados). Configure o Git Credential Manager (HTTPS) ou uma \
             chave SSH, autentique uma vez fora do aplicativo (por exemplo, com um fetch no \
             terminal) e tente novamente.",
        );
    }
    if has_any(
        &lower,
        &[
            "authentication failed",
            "invalid username or password",
            "invalid credentials",
            "bad credentials",
            "returned error: 401",
            "http basic: access denied",
            "permission denied (publickey",
            "permission denied, please try again",
            "could not open a connection to your authentication agent",
            "agent refused operation",
            "no such identity",
            "enter passphrase",
            "error in libcrypto",
        ],
    ) {
        return fail(
            ErrorCode::AuthFailed,
            "O Git não conseguiu autenticar no remoto: as credenciais estão ausentes, inválidas ou \
             expiradas, ou a chave SSH/agente não está disponível. O aplicativo não pede nem guarda \
             senhas ou tokens: configure a autenticação fora dele (Git Credential Manager para \
             HTTPS; chave SSH carregada no agente) e tente novamente.",
        );
    }
    if has_any(
        &lower,
        &[
            "permission to ",
            "denied to ",
            "returned error: 403",
            "access denied",
            "you do not have permission",
            "write access to repository not granted",
            "insufficient permission",
        ],
    ) {
        return fail(
            ErrorCode::PermissionDenied,
            "Você está autenticado, mas a conta não tem permissão para esta operação neste \
             repositório. Peça acesso de escrita ao responsável ou use um fork.",
        );
    }
    if has_any(
        &lower,
        &[
            "does not appear to be a git repository",
            "returned error: 404",
            "repository not found",
        ],
    ) || (lower.contains("repository") && lower.contains("not found"))
    {
        return fail(
            ErrorCode::RemoteNotFound,
            "O repositório remoto não foi encontrado ou a sua conta não tem acesso a ele. \
             Confira a URL do remoto e as credenciais.",
        );
    }
    if has_any(
        &lower,
        &[
            "couldn't find remote ref",
            "no such remote ref",
            "remote ref does not exist",
            "no such ref was fetched",
        ],
    ) {
        return fail(
            ErrorCode::RemoteBranchMissing,
            "A branch remota não existe no servidor. Confira o upstream da branch atual.",
        );
    }
    if has_any(
        &lower,
        &[
            "could not resolve host",
            "could not resolve hostname",
            "name or service not known",
            "temporary failure in name resolution",
            "failed to connect",
            "connection refused",
            "connection timed out",
            "connection reset",
            "connection closed",
            "network is unreachable",
            "no route to host",
            "operation timed out",
            "couldn't connect to server",
            "ssh: connect to host",
            "returned error: 5",
            "the remote end hung up unexpectedly",
            "early eof",
            "rpc failed",
            "unexpected disconnect",
            "unable to access",
        ],
    ) {
        return fail(
            ErrorCode::NetworkError,
            "Não foi possível falar com o servidor remoto (sem conexão, DNS, servidor indisponível \
             ou conexão interrompida). Verifique a internet e tente novamente. O estado remoto \
             exibido pode estar desatualizado.",
        );
    }

    fail(
        ErrorCode::CommandFailed,
        &format!(
            "O Git não conseguiu {}. Nada foi afirmado como concluído; veja os detalhes técnicos.",
            op.goal()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(code: Option<i32>, stdout: &str, stderr: &str) -> RawOutput {
        RawOutput {
            exit_code: code,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            stdout_truncated: false,
        }
    }

    #[test]
    fn sanitize_removes_http_credentials() {
        assert_eq!(
            sanitize_url("https://user:s3cret@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(
            sanitize_url("https://ghp_token@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
    }

    #[test]
    fn sanitize_keeps_ssh_user_but_drops_password() {
        assert_eq!(sanitize_url("ssh://git:pw@host/a/b.git"), "ssh://git@host/a/b.git");
        assert_eq!(sanitize_url("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert_eq!(sanitize_url("/tmp/remoto com espaço/r.git"), "/tmp/remoto com espaço/r.git");
        assert_eq!(sanitize_url("https://github.com/a@b/c.git"), "https://github.com/a@b/c.git");
    }

    #[test]
    fn redact_hides_credentials_and_auth_headers() {
        let text = "fatal: unable to access 'https://u:p@host/x.git/': 403\n> Authorization: Basic QWxhZGRpbjpvcGVu";
        let out = redact(text);
        assert!(!out.contains("u:p@"));
        assert!(!out.contains("QWxhZGRpbjpvcGVu"));
        assert!(out.contains("https://host/x.git/"));
    }

    #[test]
    fn parses_remote_verbose() {
        let text = "origin\thttps://u:p@h/x.git (fetch)\norigin\thttps://h/x.git (push)\nnoise\n";
        let parsed = parse_remote_verbose(text);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], ("origin".into(), "https://u:p@h/x.git".into(), false));
        assert!(parsed[1].2);
    }

    #[test]
    fn pull_strategy_precedence() {
        assert_eq!(classify_pull_strategy(None, None, None), PullStrategy::Unspecified);
        assert_eq!(classify_pull_strategy(None, Some("false"), None), PullStrategy::Merge);
        assert_eq!(classify_pull_strategy(None, Some("true"), None), PullStrategy::Rebase);
        assert_eq!(classify_pull_strategy(None, Some("merges"), None), PullStrategy::Rebase);
        assert_eq!(classify_pull_strategy(Some("true"), Some("false"), None), PullStrategy::Rebase);
        assert_eq!(classify_pull_strategy(None, None, Some("only")), PullStrategy::FastForwardOnly);
        assert_eq!(classify_pull_strategy(None, None, Some("false")), PullStrategy::Merge);
    }

    #[test]
    fn counts_map_to_states() {
        assert_eq!(state_from_counts(0, 0), SyncState::Synced);
        assert_eq!(state_from_counts(2, 0), SyncState::Ahead);
        assert_eq!(state_from_counts(0, 3), SyncState::Behind);
        assert_eq!(state_from_counts(1, 1), SyncState::Diverged);
    }

    #[test]
    fn parses_push_porcelain() {
        let out = "To /tmp/r.git\n!\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)\nDone\n";
        let lines = parse_push_porcelain(out);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].flag, '!');
        assert!(lines[0].summary.contains("fetch first"));
        let ok = parse_push_porcelain(" \trefs/heads/a:refs/heads/a\t1..2\nDone\n");
        assert_eq!(ok[0].flag, ' ');
    }

    #[test]
    fn classifies_authentication_and_network_failures() {
        let prompt = classify_remote_failure(
            RemoteOp::Fetch,
            &raw(Some(128), "", "fatal: could not read Username for 'https://github.com': terminal prompts disabled"),
        );
        assert_eq!(prompt.code, ErrorCode::AuthFailed);

        let ssh = classify_remote_failure(
            RemoteOp::Push,
            &raw(Some(128), "", "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."),
        );
        assert_eq!(ssh.code, ErrorCode::AuthFailed);

        let perm = classify_remote_failure(
            RemoteOp::Push,
            &raw(Some(128), "", "remote: Permission to a/b.git denied to c.\nfatal: unable to access 'https://github.com/a/b.git/': The requested URL returned error: 403"),
        );
        assert_eq!(perm.code, ErrorCode::PermissionDenied);

        let dns = classify_remote_failure(
            RemoteOp::Fetch,
            &raw(Some(128), "", "fatal: unable to access 'https://x/': Could not resolve host: x"),
        );
        assert_eq!(dns.code, ErrorCode::NetworkError);

        let tls = classify_remote_failure(
            RemoteOp::Fetch,
            &raw(Some(128), "", "fatal: unable to access 'https://x/': SSL certificate problem: self signed certificate"),
        );
        assert_eq!(tls.code, ErrorCode::TlsError);

        let missing = classify_remote_failure(
            RemoteOp::Fetch,
            &raw(Some(128), "", "fatal: '/x/y.git' does not appear to be a git repository\nfatal: Could not read from remote repository."),
        );
        assert_eq!(missing.code, ErrorCode::RemoteNotFound);

        let branch = classify_remote_failure(
            RemoteOp::Pull,
            &raw(Some(1), "", "fatal: couldn't find remote ref naoexiste"),
        );
        assert_eq!(branch.code, ErrorCode::RemoteBranchMissing);
    }

    #[test]
    fn classifies_push_rejections_from_porcelain_evidence() {
        let non_ff = classify_remote_failure(
            RemoteOp::Push,
            &raw(Some(1), "To r\n!\trefs/heads/a:refs/heads/a\t[rejected] (fetch first)\nDone\n", "error: failed to push some refs"),
        );
        assert_eq!(non_ff.code, ErrorCode::PushRejected);
        assert!(non_ff.message.contains("fetch"));

        let protected = classify_remote_failure(
            RemoteOp::Push,
            &raw(Some(1), "To r\n!\trefs/heads/a:refs/heads/a\t[remote rejected] (protected branch hook declined)\nDone\n", "remote: error: GH006: Protected branch update failed"),
        );
        assert_eq!(protected.code, ErrorCode::PushRejected);
        assert!(protected.message.contains("protegida"));
    }

    #[test]
    fn classifies_pull_local_blockers_and_unknown_failures() {
        let overwrite = classify_remote_failure(
            RemoteOp::Pull,
            &raw(Some(1), "", "error: The following untracked working tree files would be overwritten by merge:\n\ta.txt\nAborting"),
        );
        assert_eq!(overwrite.code, ErrorCode::OperationBlocked);

        let diverged = classify_remote_failure(
            RemoteOp::Pull,
            &raw(Some(128), "", "fatal: Not possible to fast-forward, aborting."),
        );
        assert_eq!(diverged.code, ErrorCode::OperationBlocked);

        let unknown = classify_remote_failure(RemoteOp::Fetch, &raw(Some(2), "", "algo estranho"));
        assert_eq!(unknown.code, ErrorCode::CommandFailed);
        assert!(unknown.details.is_some());

        let killed = classify_remote_failure(RemoteOp::Push, &raw(None, "", ""));
        assert_eq!(killed.code, ErrorCode::CommandFailed);
        assert!(killed.message.contains("NÃO"));
    }

    #[test]
    fn errors_never_expose_credentials_in_details() {
        let err = classify_remote_failure(
            RemoteOp::Fetch,
            &raw(Some(128), "", "fatal: unable to access 'https://user:topsecret@host/r.git/': Could not resolve host: host"),
        );
        let details = err.details.unwrap();
        assert!(!details.contains("topsecret"));
        assert!(!details.contains("user:"));
    }

    /// Programa que ignora os argumentos e demora; simula uma operação de rede travada.
    #[cfg(windows)]
    fn slow_program(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("lento.cmd");
        fs::write(&path, "@echo off\r\nping -n 8 127.0.0.1 >nul\r\n").unwrap();
        path
    }

    #[cfg(unix)]
    fn slow_program(dir: &Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("lento.sh");
        fs::write(&path, "#!/bin/sh\nsleep 8\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn network_timeout_is_reported_as_unconfirmed() {
        let dir = tempfile::tempdir().unwrap();
        let git = Git::with_program(slow_program(dir.path()));
        let started = std::time::Instant::now();
        let err = run_with_timeout(&git, dir.path(), &["fetch", "origin"], Duration::from_millis(300))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Timeout);
        assert!(err.message.contains("NÃO foi confirmado"));
        assert!(started.elapsed() < Duration::from_secs(6), "o processo deveria ser encerrado");
    }

    #[test]
    fn remote_name_must_be_known_and_safe() {
        let remotes = vec![RemoteInfo {
            name: "origin".into(),
            fetch_url: None,
            push_url: None,
        }];
        assert!(require_known_remote(&remotes, "origin").is_ok());
        assert!(require_known_remote(&remotes, "").is_err());
        assert!(require_known_remote(&remotes, "--upload-pack=calc").is_err());
        assert!(require_known_remote(&remotes, "https://evil/x.git").is_err());
        assert!(require_known_remote(&remotes, "outro").is_err());
    }
}
