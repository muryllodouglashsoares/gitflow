//! Testes de integração da Fase 3: diagnóstico de sincronização, fetch, pull e push.
//!
//! Usam o Git real contra repositórios temporários e **remotos locais** (repositórios bare em
//! pastas temporárias): não dependem de internet, de credenciais nem de repositórios reais.
//! A configuração global/de sistema do Git é isolada para o processo de teste.

use gitflow_git::{
    fetch_remote, get_status, get_sync_status, lock::RepoGuard, open_repository, pull_branch,
    push_branch, ErrorCode, FetchRequest, Git, PullOutcome, PullRequest, PullStrategy,
    PushRequest, SyncState, SyncTarget,
};
use std::{fs, path::Path, process::Command, sync::Once};
use tempfile::TempDir;

// ----------------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------------

fn isolate_git_config() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" });
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    });
}

fn git_cmd(dir: &Path, args: &[&str]) -> std::process::Output {
    isolate_git_config();
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git precisa estar instalado para os testes")
}

fn git(dir: &Path, args: &[&str]) {
    let out = git_cmd(dir, args);
    assert!(
        out.status.success(),
        "git {:?} falhou: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = git_cmd(dir, args);
    assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn configure_identity(dir: &Path) {
    git(dir, &["config", "user.name", "Teste"]);
    git(dir, &["config", "user.email", "teste@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn new_repo() -> TempDir {
    isolate_git_config();
    let dir = tempfile::Builder::new().prefix("gitflow-fase3-").tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    configure_identity(dir.path());
    dir
}

fn bare_remote() -> TempDir {
    isolate_git_config();
    let dir = tempfile::Builder::new().prefix("gitflow-fase3-remoto-").tempdir().unwrap();
    git(dir.path(), &["init", "-q", "--bare", "-b", "main"]);
    dir
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

fn commit_file(dir: &Path, rel: &str, content: &str, message: &str) {
    write(dir, rel, content);
    git(dir, &["add", "--", rel]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn clone_of(remote: &Path) -> TempDir {
    isolate_git_config();
    let dir = tempfile::Builder::new().prefix("gitflow-fase3-clone-").tempdir().unwrap();
    git(dir.path(), &["clone", "-q", remote.to_str().unwrap(), "."]);
    configure_identity(dir.path());
    dir
}

struct Pair {
    remote: TempDir,
    local: TempDir,
}

/// `local` tem a branch `main` com um commit já enviado e acompanhando `origin/main`.
fn pair() -> Pair {
    let remote = bare_remote();
    let local = new_repo();
    commit_file(local.path(), "base.txt", "base\n", "base");
    git(local.path(), &["remote", "add", "origin", remote.path().to_str().unwrap()]);
    git(local.path(), &["push", "-q", "-u", "origin", "main"]);
    Pair { remote, local }
}

/// Igual a `pair`, mas com a branch de trabalho `feature` (não protegida) já publicada.
fn feature_pair() -> Pair {
    let pair = pair();
    git(pair.local.path(), &["checkout", "-q", "-b", "feature"]);
    git(pair.local.path(), &["push", "-q", "-u", "origin", "feature"]);
    pair
}

fn git_handle() -> Git {
    isolate_git_config();
    Git::locate().expect("git precisa estar instalado para os testes")
}

fn repo_path(git: &Git, dir: &TempDir) -> String {
    open_repository(git, dir.path().to_str().unwrap()).unwrap().path
}

fn target(branch: &str, remote_branch: &str) -> SyncTarget {
    SyncTarget {
        branch: branch.into(),
        remote: "origin".into(),
        remote_branch: remote_branch.into(),
    }
}

fn fetch_req(path: &str, remote: &str) -> FetchRequest {
    FetchRequest { repo_path: path.into(), remote: remote.into() }
}

fn pull_req(path: &str, branch: &str, remote_branch: &str) -> PullRequest {
    PullRequest { repo_path: path.into(), target: target(branch, remote_branch) }
}

fn push_req(path: &str, branch: &str, remote_branch: &str, confirm: bool) -> PushRequest {
    PushRequest {
        repo_path: path.into(),
        target: target(branch, remote_branch),
        confirm_protected_branch: confirm,
    }
}

// ----------------------------------------------------------------------------
// Diagnóstico
// ----------------------------------------------------------------------------

#[test]
fn repository_without_remote_is_reported_and_remote_operations_are_blocked() {
    let git = git_handle();
    let dir = new_repo();
    commit_file(dir.path(), "a.txt", "1\n", "c1");
    let path = repo_path(&git, &dir);

    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::NoRemote);
    assert!(sync.remotes.is_empty());
    assert_eq!(sync.branch.as_deref(), Some("main"));
    assert_eq!(sync.ahead, None);

    let err = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    let err = push_branch(&git, &push_req(&path, "main", "main", true)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
}

#[test]
fn remote_without_upstream_is_distinguished_and_blocks_pull_and_push() {
    let git = git_handle();
    let remote = bare_remote();
    let dir = new_repo();
    commit_file(dir.path(), "a.txt", "1\n", "c1");
    git(dir.path(), &["remote", "add", "origin", remote.path().to_str().unwrap()]);
    let path = repo_path(&git, &dir);

    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::NoUpstream);
    assert_eq!(sync.remotes.len(), 1);
    assert_eq!(sync.remotes[0].name, "origin");
    assert!(sync.upstream.is_none());

    let err = push_branch(&git, &push_req(&path, "main", "main", true)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("upstream"));
    // O aplicativo não cria o upstream por conta própria.
    assert_eq!(get_sync_status(&git, &path).unwrap().state, SyncState::NoUpstream);
    assert!(git_out(remote.path(), &["branch", "--list"]).is_empty());
}

#[test]
fn upstream_configured_but_never_fetched_is_not_reported_as_synced() {
    let git = git_handle();
    let remote = bare_remote();
    let dir = new_repo();
    commit_file(dir.path(), "a.txt", "1\n", "c1");
    git(dir.path(), &["remote", "add", "origin", remote.path().to_str().unwrap()]);
    git(dir.path(), &["config", "branch.main.remote", "origin"]);
    git(dir.path(), &["config", "branch.main.merge", "refs/heads/main"]);
    let path = repo_path(&git, &dir);

    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::UpstreamNotFetched);
    let upstream = sync.upstream.unwrap();
    assert_eq!(upstream.display, "origin/main");
    assert!(!upstream.tracking_ref_exists);
    assert_eq!((sync.ahead, sync.behind), (None, None));

    let err = push_branch(&git, &push_req(&path, "main", "main", true)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("fetch"));
}

#[test]
fn synced_ahead_behind_and_diverged_states_follow_the_real_refs() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);

    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::Synced);
    assert_eq!((sync.ahead, sync.behind), (Some(0), Some(0)));
    assert_eq!(sync.upstream.as_ref().unwrap().remote, "origin");
    assert_eq!(sync.upstream.as_ref().unwrap().remote_branch, "main");

    // Adiantada.
    commit_file(pair.local.path(), "local.txt", "l\n", "local");
    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::Ahead);
    assert_eq!((sync.ahead, sync.behind), (Some(1), Some(0)));

    // Outra pessoa envia ao remoto: SEM fetch o aplicativo ainda não sabe (não inventa).
    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "remoto.txt", "r\n", "remoto");
    git(other.path(), &["push", "-q", "origin", "main"]);
    assert_eq!(get_sync_status(&git, &path).unwrap().state, SyncState::Ahead);

    // Depois do fetch, a divergência aparece.
    fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();
    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::Diverged);
    assert_eq!((sync.ahead, sync.behind), (Some(1), Some(1)));
}

#[test]
fn behind_state_appears_only_after_fetch() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);

    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "novo.txt", "n\n", "novo");
    git(other.path(), &["push", "-q", "origin", "main"]);

    assert_eq!(get_sync_status(&git, &path).unwrap().state, SyncState::Synced);
    let result = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();
    assert!(result.updated_refs >= 1);
    assert_eq!(result.sync.state, SyncState::Behind);
    assert_eq!(result.sync.behind, Some(1));
    assert!(result.sync.last_fetch_at.is_some());
}

#[test]
fn credentials_in_remote_urls_are_never_exposed() {
    let git = git_handle();
    let dir = new_repo();
    commit_file(dir.path(), "a.txt", "1\n", "c1");
    git(dir.path(), &["remote", "add", "origin", "https://usuario:segredo123@127.0.0.1:1/x.git"]);
    let path = repo_path(&git, &dir);

    let sync = get_sync_status(&git, &path).unwrap();
    let remote = &sync.remotes[0];
    assert_eq!(remote.fetch_url.as_deref(), Some("https://127.0.0.1:1/x.git"));
    assert!(!format!("{sync:?}").contains("segredo123"));

    // Falha de conexão (porta recusada, sem depender da internet): erro de rede, sem segredo.
    let err = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap_err();
    assert_eq!(err.code, ErrorCode::NetworkError, "{err:?}");
    let shown = format!("{err:?}");
    assert!(!shown.contains("segredo123"), "{shown}");
}

// ----------------------------------------------------------------------------
// Fetch
// ----------------------------------------------------------------------------

#[test]
fn fetch_updates_remote_refs_without_touching_the_work_tree_or_branch() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);
    let head_before = git_out(pair.local.path(), &["rev-parse", "HEAD"]);

    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "novo.txt", "n\n", "novo");
    git(other.path(), &["push", "-q", "origin", "main"]);

    let result = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();
    assert_eq!(result.remote, "origin");
    assert_eq!(git_out(pair.local.path(), &["rev-parse", "HEAD"]), head_before);
    assert!(!pair.local.path().join("novo.txt").exists(), "fetch não integra nada");
    assert!(get_status(&git, &path).unwrap().is_clean);

    // Um segundo fetch não acha novidades.
    let again = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();
    assert_eq!(again.updated_refs, 0);
}

#[test]
fn fetch_only_accepts_remotes_that_exist_and_never_arguments() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);

    for bad in ["", "naoexiste", "--upload-pack=calc", "-v", "https://evil.example/x.git", "origin extra"] {
        let err = fetch_remote(&git, &fetch_req(&path, bad)).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "remoto {bad:?}");
    }
}

#[test]
fn fetch_failure_is_reported_and_a_retry_works_after_fixing_the_remote() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);
    let good_url = pair.remote.path().to_str().unwrap().to_string();

    git(pair.local.path(), &["remote", "set-url", "origin", "/caminho/que/nao/existe/remoto.git"]);
    let err = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap_err();
    assert_eq!(err.code, ErrorCode::RemoteNotFound, "{err:?}");
    assert!(err.details.is_some());

    // A trava foi liberada e uma nova tentativa funciona.
    git(pair.local.path(), &["remote", "set-url", "origin", &good_url]);
    let ok = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();
    assert_eq!(ok.sync.state, SyncState::Synced);
}

#[test]
fn two_incompatible_operations_on_the_same_repository_do_not_run_together() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);

    let guard = RepoGuard::acquire(Path::new(&path)).unwrap();
    let err = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap_err();
    assert_eq!(err.code, ErrorCode::RepositoryBusy);
    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::RepositoryBusy);
    let err = push_branch(&git, &push_req(&path, "main", "main", true)).unwrap_err();
    assert_eq!(err.code, ErrorCode::RepositoryBusy);

    drop(guard);
    assert!(fetch_remote(&git, &fetch_req(&path, "origin")).is_ok());
}

// ----------------------------------------------------------------------------
// Pull
// ----------------------------------------------------------------------------

#[test]
fn pull_fast_forwards_and_reports_the_real_outcome() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);

    // Nada novo: já atualizado.
    let same = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap();
    assert_eq!(same.outcome, PullOutcome::AlreadyUpToDate);

    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "novo.txt", "conteudo remoto\n", "novo");
    git(other.path(), &["push", "-q", "origin", "main"]);

    let result = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap();
    assert_eq!(result.outcome, PullOutcome::Updated);
    assert!(!result.merge_commit);
    assert_ne!(result.head_before, result.head_after);
    assert_eq!(read(pair.local.path(), "novo.txt"), "conteudo remoto\n");
    assert_eq!(result.sync.state, SyncState::Synced);
}

#[test]
fn pull_is_blocked_by_local_changes_and_never_discards_them() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);
    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "novo.txt", "n\n", "novo");
    git(other.path(), &["push", "-q", "origin", "main"]);

    // Não preparada.
    write(pair.local.path(), "base.txt", "editado localmente\n");
    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.details.as_deref().unwrap_or("").contains("base.txt"));
    assert_eq!(read(pair.local.path(), "base.txt"), "editado localmente\n");

    // Preparada.
    git(pair.local.path(), &["add", "base.txt"]);
    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(git_out(pair.local.path(), &["diff", "--cached", "--name-only"]), "base.txt");

    // Nada de stash, reset ou limpeza silenciosa.
    assert!(git_out(pair.local.path(), &["stash", "list"]).is_empty());
    assert!(!pair.local.path().join("novo.txt").exists());
}

#[test]
fn pull_does_not_block_on_unrelated_untracked_files_but_never_overwrites_them() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);
    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "novo.txt", "remoto\n", "novo");
    git(other.path(), &["push", "-q", "origin", "main"]);

    // Arquivo não rastreado e sem relação: o pull funciona.
    write(pair.local.path(), "anotacoes.txt", "minhas notas\n");
    let ok = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap();
    assert_eq!(ok.outcome, PullOutcome::Updated);
    assert_eq!(read(pair.local.path(), "anotacoes.txt"), "minhas notas\n");

    // Arquivo não rastreado que seria sobrescrito: o Git recusa e o conteúdo fica intacto.
    commit_file(other.path(), "colide.txt", "do remoto\n", "colide");
    git(other.path(), &["push", "-q", "origin", "main"]);
    write(pair.local.path(), "colide.txt", "meu arquivo local\n");
    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked, "{err:?}");
    assert_eq!(read(pair.local.path(), "colide.txt"), "meu arquivo local\n");
}

#[test]
fn divergent_pull_without_configured_strategy_is_blocked_before_running_git() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);
    commit_file(pair.local.path(), "local.txt", "l\n", "local");
    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "remoto.txt", "r\n", "remoto");
    git(other.path(), &["push", "-q", "origin", "main"]);
    fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();

    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::Diverged);
    assert_eq!(sync.pull_strategy, PullStrategy::Unspecified);

    let head = git_out(pair.local.path(), &["rev-parse", "HEAD"]);
    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("pull.rebase"));
    assert_eq!(git_out(pair.local.path(), &["rev-parse", "HEAD"]), head);
    assert!(get_status(&git, &path).unwrap().operation.is_none());
}

#[test]
fn divergent_pull_with_merge_strategy_integrates_without_conflicts() {
    let git = git_handle();
    let pair = pair();
    git(pair.local.path(), &["config", "pull.rebase", "false"]);
    let path = repo_path(&git, &pair.local);
    commit_file(pair.local.path(), "local.txt", "l\n", "local");
    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "remoto.txt", "r\n", "remoto");
    git(other.path(), &["push", "-q", "origin", "main"]);

    let result = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap();
    assert_eq!(result.outcome, PullOutcome::Updated);
    assert!(result.merge_commit);
    assert_eq!(result.sync.state, SyncState::Ahead); // o merge ainda não foi enviado
    assert!(pair.local.path().join("remoto.txt").exists());
    assert!(pair.local.path().join("local.txt").exists());
}

#[test]
fn pull_conflict_is_reported_not_resolved_and_leaves_the_repository_for_manual_resolution() {
    let git = git_handle();
    let pair = pair();
    git(pair.local.path(), &["config", "pull.rebase", "false"]);
    let path = repo_path(&git, &pair.local);

    commit_file(pair.local.path(), "base.txt", "versao local\n", "local");
    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "base.txt", "versao remota\n", "remoto");
    git(other.path(), &["push", "-q", "origin", "main"]);

    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::MergeConflict, "{err:?}");
    assert!(err.details.as_deref().unwrap_or("").contains("CONFLICT"));

    let status = get_status(&git, &path).unwrap();
    assert_eq!(status.summary.conflicted, 1);
    assert!(status.operation.is_some());
    // Nenhuma resolução automática: os marcadores continuam no arquivo.
    assert!(read(pair.local.path(), "base.txt").contains("<<<<<<<"));

    // Enquanto houver conflito, pull e push ficam bloqueados.
    let again = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(again.code, ErrorCode::OperationBlocked);
    let push = push_branch(&git, &push_req(&path, "main", "main", true)).unwrap_err();
    assert_eq!(push.code, ErrorCode::OperationBlocked);
}

#[test]
fn stale_target_is_refused_without_running_anything() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);

    let err = pull_branch(&git, &pull_req(&path, "main", "outra")).unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    let err = push_branch(&git, &push_req(&path, "outra-branch", "main", true)).unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
}

// ----------------------------------------------------------------------------
// Push
// ----------------------------------------------------------------------------

#[test]
fn push_sends_only_the_current_branch_to_its_upstream_and_confirms_the_result() {
    let git = git_handle();
    let pair = feature_pair();
    let path = repo_path(&git, &pair.local);

    // Nada a enviar: explicado, sem executar o push.
    let err = push_branch(&git, &push_req(&path, "feature", "feature", false)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("Não há commits locais"));

    commit_file(pair.local.path(), "f.txt", "f\n", "feature work");
    // Uma segunda branch local, que NÃO deve ser enviada.
    git(pair.local.path(), &["branch", "nao-enviar"]);
    let local_head = git_out(pair.local.path(), &["rev-parse", "HEAD"]);

    let result = push_branch(&git, &push_req(&path, "feature", "feature", false)).unwrap();
    assert_eq!(result.pushed_commits, 1);
    assert_eq!(result.local_oid, local_head);
    assert_eq!(result.sync.state, SyncState::Synced);
    assert_eq!(git_out(pair.remote.path(), &["rev-parse", "feature"]), local_head);
    assert!(git_out(pair.remote.path(), &["branch", "--list", "nao-enviar"]).is_empty());
}

#[test]
fn push_to_main_or_master_needs_explicit_extra_confirmation() {
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);
    commit_file(pair.local.path(), "f.txt", "f\n", "work");
    let local_head = git_out(pair.local.path(), &["rev-parse", "HEAD"]);
    let remote_before = git_out(pair.remote.path(), &["rev-parse", "main"]);

    let err = push_branch(&git, &push_req(&path, "main", "main", false)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(git_out(pair.remote.path(), &["rev-parse", "main"]), remote_before);

    let ok = push_branch(&git, &push_req(&path, "main", "main", true)).unwrap();
    assert_eq!(ok.sync.state, SyncState::Synced);
    assert_eq!(git_out(pair.remote.path(), &["rev-parse", "main"]), local_head);
}

#[test]
fn rejected_push_is_reported_as_failure_and_the_remote_is_left_intact() {
    let git = git_handle();
    let pair = feature_pair();
    let path = repo_path(&git, &pair.local);
    commit_file(pair.local.path(), "meu.txt", "meu\n", "meu commit");

    // Outra pessoa envia antes; o aplicativo ainda não sabe (sem fetch), então o Git é quem recusa.
    let other = clone_of(pair.remote.path());
    git(other.path(), &["checkout", "-q", "feature"]);
    commit_file(other.path(), "dela.txt", "dela\n", "commit dela");
    git(other.path(), &["push", "-q", "origin", "feature"]);
    let remote_head = git_out(pair.remote.path(), &["rev-parse", "feature"]);

    let err = push_branch(&git, &push_req(&path, "feature", "feature", false)).unwrap_err();
    assert_eq!(err.code, ErrorCode::PushRejected, "{err:?}");
    assert!(err.message.contains("Nada foi enviado"));
    assert_eq!(git_out(pair.remote.path(), &["rev-parse", "feature"]), remote_head);

    // Depois do fetch o aplicativo passa a bloquear antes de chamar o Git.
    fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();
    let blocked = push_branch(&git, &push_req(&path, "feature", "feature", false)).unwrap_err();
    assert_eq!(blocked.code, ErrorCode::OperationBlocked);
    assert!(blocked.message.contains("divergiu"));
}

#[cfg(unix)]
#[test]
fn push_rejected_by_a_server_side_hook_is_a_push_failure_not_a_success() {
    use std::os::unix::fs::PermissionsExt;

    let git = git_handle();
    let pair = feature_pair();
    let path = repo_path(&git, &pair.local);
    commit_file(pair.local.path(), "f.txt", "f\n", "work");

    // Política do remoto: um hook pre-receive que recusa tudo.
    let hook = pair.remote.path().join("hooks").join("pre-receive");
    fs::write(&hook, "#!/bin/sh\necho 'politica do servidor' >&2\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let remote_before = git_out(pair.remote.path(), &["rev-parse", "feature"]);

    let err = push_branch(&git, &push_req(&path, "feature", "feature", false)).unwrap_err();
    assert_eq!(err.code, ErrorCode::PushRejected, "{err:?}");
    assert_eq!(git_out(pair.remote.path(), &["rev-parse", "feature"]), remote_before);
    assert_eq!(get_sync_status(&git, &path).unwrap().state, SyncState::Ahead);
}

#[test]
fn push_is_blocked_when_behind_or_without_a_branch() {
    let git = git_handle();
    let pair = feature_pair();
    let path = repo_path(&git, &pair.local);

    let other = clone_of(pair.remote.path());
    git(other.path(), &["checkout", "-q", "feature"]);
    commit_file(other.path(), "dela.txt", "dela\n", "dela");
    git(other.path(), &["push", "-q", "origin", "feature"]);
    fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();

    let err = push_branch(&git, &push_req(&path, "feature", "feature", false)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);

    // HEAD desanexado: sem destino inequívoco.
    git(pair.local.path(), &["checkout", "-q", "--detach"]);
    let sync = get_sync_status(&git, &path).unwrap();
    assert_eq!(sync.state, SyncState::DetachedHead);
    let err = push_branch(&git, &push_req(&path, "feature", "feature", false)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("desanexado"));
}

// ----------------------------------------------------------------------------
// Caminhos especiais e garantias gerais
// ----------------------------------------------------------------------------

#[test]
fn whole_flow_works_with_spaces_and_accents_in_paths() {
    isolate_git_config();
    let git = git_handle();
    let root = tempfile::Builder::new().prefix("gitflow fase3 ação ").tempdir().unwrap();
    let remote = root.path().join("remoto com espaço.git");
    let local = root.path().join("repositório local");
    fs::create_dir_all(&remote).unwrap();
    fs::create_dir_all(&local).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    git(&local, &["init", "-q", "-b", "main"]);
    configure_identity(&local);
    commit_file(&local, "base.txt", "b\n", "base");
    git(&local, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&local, &["push", "-q", "-u", "origin", "main"]);
    let path = open_repository(&git, local.to_str().unwrap()).unwrap().path;

    commit_file(&local, "ação.txt", "ok\n", "ação");
    let pushed = push_branch(&git, &push_req(&path, "main", "main", true)).unwrap();
    assert_eq!(pushed.sync.state, SyncState::Synced);

    let other = root.path().join("outra cópia");
    fs::create_dir_all(&other).unwrap();
    git(&other, &["clone", "-q", remote.to_str().unwrap(), "."]);
    configure_identity(&other);
    commit_file(&other, "novo é bom.txt", "n\n", "novo");
    git(&other, &["push", "-q", "origin", "main"]);

    let fetched = fetch_remote(&git, &fetch_req(&path, "origin")).unwrap();
    assert_eq!(fetched.sync.state, SyncState::Behind);
    let pulled = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap();
    assert_eq!(pulled.outcome, PullOutcome::Updated);
    assert!(local.join("novo é bom.txt").exists());
}

#[test]
fn failed_operations_never_fall_back_to_destructive_commands() {
    // Garantia estática: o módulo remoto não contém argumentos destrutivos nem de envio amplo.
    let source = include_str!("../src/remote.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();
    for forbidden in [
        "\"--force\"",
        "\"--force-with-lease\"",
        "\"-f\"",
        "\"--mirror\"",
        "\"--all\"",
        "\"--tags\"",
        "\"--hard\"",
        "\"-u\"",
        "\"--set-upstream\"",
        "\"reset\"",
        "\"clean\"",
        "\"stash\"",
        "\"checkout\"",
        "\"restore\"",
        "\"--no-verify\"",
        "\"--autostash\"",
    ] {
        assert!(!production.contains(forbidden), "argumento proibido encontrado: {forbidden}");
    }

    // Garantia dinâmica: depois de falhas de pull, o histórico e os arquivos continuam como estavam.
    let git = git_handle();
    let pair = pair();
    let path = repo_path(&git, &pair.local);
    commit_file(pair.local.path(), "local.txt", "l\n", "local");
    let other = clone_of(pair.remote.path());
    commit_file(other.path(), "remoto.txt", "r\n", "remoto");
    git(other.path(), &["push", "-q", "origin", "main"]);
    let head = git_out(pair.local.path(), &["rev-parse", "HEAD"]);
    let reflog = git_out(pair.local.path(), &["reflog", "--format=%H"]);

    // Sem fetch, o aplicativo ainda vê a branch apenas "adiantada" e deixa o Git decidir. Sem
    // estratégia configurada o pull só aceita avanço rápido; como há divergência, o Git recusa e
    // nada é alterado (nenhum merge, rebase, reset ou stash para "dar um jeito").
    let err = pull_branch(&git, &pull_req(&path, "main", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked, "{err:?}");
    assert_eq!(git_out(pair.local.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(git_out(pair.local.path(), &["reflog", "--format=%H"]), reflog);
    assert!(git_out(pair.local.path(), &["stash", "list"]).is_empty());
    assert_eq!(read(pair.local.path(), "local.txt"), "l\n");
    assert!(get_status(&git, &path).unwrap().operation.is_none());
}
