//! Testes de integração da Fase 5: merge manual, proteção da `main` e tratamento de conflitos.
//! Usam o Git real contra repositórios temporários descartáveis (nunca o repositório de desenvolvimento).

use gitflow_git::{
    get_status, lock::RepoGuard, merge_branches, preview_merge, BlockerKind, ErrorCode,
    ForecastState, Git, MergeOutcome, MergePreview, MergePreviewRequest, MergeRelation,
    MergeRequest, MergeStrategy, Operation,
};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

// ----------------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------------

fn git_cmd(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_CONFIG_NOSYSTEM", "1")
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

fn init_repo() -> TempDir {
    let dir = tempfile::Builder::new().prefix("gitflow-fase5-").tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Teste"]);
    git(dir.path(), &["config", "user.email", "teste@example.com"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
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

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn root_of(dir: &TempDir) -> String {
    fs::canonicalize(dir.path()).unwrap().to_string_lossy().into_owned()
}

fn locate() -> Git {
    Git::locate().expect("Git não localizado")
}

/// `git merge-tree --write-tree` (previsão de conflitos) existe a partir do Git 2.38.
fn git_supports_merge_tree() -> bool {
    let version = locate().version().unwrap();
    let mut parts = version.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    (major, minor) >= (2, 38)
}

fn oid(dir: &Path, rev: &str) -> String {
    git_out(dir, &["rev-parse", rev])
}

/// `main` com um commit base e `feature/login` à frente (avanço rápido possível). Termina em `main`.
fn setup_fast_forward() -> TempDir {
    let dir = init_repo();
    let p = dir.path();
    write(p, "base.txt", "base\n");
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "feature/login"]);
    write(p, "login.txt", "login\n");
    commit_all(p, "adiciona login");
    write(p, "login2.txt", "login 2\n");
    commit_all(p, "adiciona login 2");
    git(p, &["checkout", "-q", "main"]);
    dir
}

/// As duas branches têm commits próprios, em arquivos diferentes (sem conflito). Termina em `main`.
fn setup_diverged() -> TempDir {
    let dir = init_repo();
    let p = dir.path();
    write(p, "base.txt", "base\n");
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "feature/login"]);
    write(p, "login.txt", "login\n");
    commit_all(p, "adiciona login");
    git(p, &["checkout", "-q", "main"]);
    write(p, "main-only.txt", "só na main\n");
    commit_all(p, "commit só na main");
    dir
}

/// As duas branches alteram a mesma linha de `shared.txt`. Termina em `main`.
fn setup_conflict() -> TempDir {
    let dir = init_repo();
    let p = dir.path();
    write(p, "shared.txt", "a\nb\nc\n");
    write(p, "outro.txt", "outro\n");
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "feature/login"]);
    write(p, "shared.txt", "a\nFEATURE\nc\n");
    commit_all(p, "feature altera shared");
    git(p, &["checkout", "-q", "main"]);
    write(p, "shared.txt", "a\nMAIN\nc\n");
    commit_all(p, "main altera shared");
    dir
}

fn preview(dir: &TempDir, source: &str, target: &str) -> gitflow_git::Result<MergePreview> {
    preview_merge(
        &locate(),
        &MergePreviewRequest {
            repo_path: root_of(dir),
            source: source.into(),
            target: target.into(),
        },
    )
}

fn request(dir: &TempDir, p: &MergePreview, confirmed: bool, confirm_target: Option<&str>) -> MergeRequest {
    MergeRequest {
        repo_path: root_of(dir),
        source: p.source.name.clone(),
        target: p.target.name.clone(),
        token: p.token.clone(),
        confirmed,
        confirm_target: confirm_target.map(str::to_string),
    }
}

/// Executa o merge confirmando como um usuário faria (inclusive digitando o nome da branch protegida).
fn run_merge(dir: &TempDir, p: &MergePreview) -> gitflow_git::Result<gitflow_git::MergeResult> {
    let typed = p.required_confirmation.clone();
    merge_branches(&locate(), &request(dir, p, true, typed.as_deref()))
}

fn has_blocker(p: &MergePreview, kind: BlockerKind) -> bool {
    p.blockers.iter().any(|b| b.kind == kind)
}

// ----------------------------------------------------------------------------
// Prévia
// ----------------------------------------------------------------------------

#[test]
fn preview_is_read_only_and_reports_fast_forward() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let head_before = oid(p, "HEAD");
    let main_before = oid(p, "main");
    let feature_before = oid(p, "feature/login");
    let refs_before = git_out(p, &["for-each-ref"]);

    let pv = preview(&dir, "feature/login", "main").unwrap();

    assert_eq!(pv.relation, MergeRelation::FastForward);
    assert_eq!(pv.strategy, Some(MergeStrategy::FastForward));
    assert_eq!((pv.source_only, pv.target_only), (2, 0));
    assert_eq!(pv.source_commits.len(), 2);
    assert_eq!(pv.source_commits[0].subject, "adiciona login 2");
    assert!(pv.target_is_current && pv.target_protected);
    assert_eq!(pv.required_confirmation.as_deref(), Some("main"));
    assert!(pv.blockers.is_empty(), "{:?}", pv.blockers);
    assert!(pv.can_execute);
    assert_eq!(pv.forecast.state, ForecastState::NotApplicable);

    // Nada mudou: nem referências, nem HEAD, nem arquivos.
    assert_eq!(oid(p, "HEAD"), head_before);
    assert_eq!(oid(p, "main"), main_before);
    assert_eq!(oid(p, "feature/login"), feature_before);
    assert_eq!(git_out(p, &["for-each-ref"]), refs_before);
    assert!(!p.join("login.txt").exists());
    assert!(get_status(&locate(), &root_of(&dir)).unwrap().is_clean);
}

#[test]
fn preview_reports_divergence_and_clean_forecast() {
    let dir = setup_diverged();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert_eq!(pv.relation, MergeRelation::Diverged);
    assert_eq!(pv.strategy, Some(MergeStrategy::MergeCommit));
    assert_eq!((pv.source_only, pv.target_only), (1, 1));
    assert!(pv.merge_base.is_some());
    assert!(pv.can_execute);
    // Git 2.38+ simula a integração; versões mais antigas devem declarar que não sabem.
    if git_supports_merge_tree() {
        assert_eq!(pv.forecast.state, ForecastState::Clean);
    } else {
        assert_eq!(pv.forecast.state, ForecastState::Unavailable);
    }
    assert!(pv.forecast.files.is_empty());
}

#[test]
fn preview_forecasts_conflicting_files() {
    let dir = setup_conflict();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert_eq!(pv.relation, MergeRelation::Diverged);
    if git_supports_merge_tree() {
        assert_eq!(pv.forecast.state, ForecastState::Conflicts);
        assert_eq!(pv.forecast.files, vec!["shared.txt".to_string()]);
    } else {
        assert_eq!(pv.forecast.state, ForecastState::Unavailable);
    }
    // A simulação não deixa rastros no repositório.
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert!(status.is_clean && status.operation.is_none());
}

#[test]
fn identical_and_already_integrated_branches_have_nothing_to_merge() {
    let dir = setup_fast_forward();
    let p = dir.path();

    // Já integradas: a origem (main) está contida no destino (feature/login).
    git(p, &["checkout", "-q", "feature/login"]);
    let pv = preview(&dir, "main", "feature/login").unwrap();
    assert_eq!(pv.relation, MergeRelation::AlreadyIntegrated);
    assert!(!pv.can_execute && pv.blockers.is_empty());
    let before = oid(p, "HEAD");
    let err = run_merge(&dir, &pv).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(oid(p, "HEAD"), before);

    // Idênticas.
    git(p, &["branch", "copia", "feature/login"]);
    let pv = preview(&dir, "copia", "feature/login").unwrap();
    assert_eq!(pv.relation, MergeRelation::Identical);
    assert!(!pv.can_execute);
    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::OperationBlocked);
}

// ----------------------------------------------------------------------------
// Sucesso
// ----------------------------------------------------------------------------

#[test]
fn fast_forward_merge_integrates_source_into_target() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let source_oid = oid(p, "feature/login");
    let main_before = oid(p, "main");

    let pv = preview(&dir, "feature/login", "main").unwrap();
    let result = run_merge(&dir, &pv).unwrap();

    assert_eq!(result.outcome, MergeOutcome::FastForward);
    assert!(!result.merge_commit);
    assert_eq!(result.head_before.as_deref(), Some(main_before.as_str()));
    assert_eq!(result.head_after.as_deref(), Some(source_oid.as_str()));
    assert_eq!(result.integrated_commits, 2);
    assert!(result.conflicts.is_empty());
    assert_eq!(result.exit_code, Some(0));
    assert!(result.status.is_clean && result.status.operation.is_none());

    // A branch de destino é a que mudou; a origem permanece intacta.
    assert_eq!(oid(p, "main"), source_oid);
    assert_eq!(oid(p, "feature/login"), source_oid);
    assert_eq!(read(p, "login.txt"), "login\n");
    assert_eq!(git_out(p, &["rev-list", "--count", "HEAD"]), "3");
    assert!(git_out(p, &["stash", "list"]).is_empty());
}

#[test]
fn diverged_merge_creates_merge_commit_with_correct_parents() {
    let dir = setup_diverged();
    let p = dir.path();
    let main_before = oid(p, "main");
    let feature_before = oid(p, "feature/login");

    let pv = preview(&dir, "feature/login", "main").unwrap();
    let result = run_merge(&dir, &pv).unwrap();

    assert_eq!(result.outcome, MergeOutcome::MergeCommit);
    assert!(result.merge_commit);
    let head = oid(p, "HEAD");
    assert_eq!(result.head_after.as_deref(), Some(head.as_str()));
    assert_ne!(head, main_before);
    // Pais: 1º = destino anterior, 2º = origem.
    let parents = git_out(p, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents, format!("{head} {main_before} {feature_before}"));
    // Os dois lados estão presentes e a origem não foi alterada.
    assert_eq!(read(p, "login.txt"), "login\n");
    assert_eq!(read(p, "main-only.txt"), "só na main\n");
    assert_eq!(oid(p, "feature/login"), feature_before);
    assert!(result.status.is_clean);
}

#[test]
fn merge_direction_updates_only_the_target_branch() {
    // Origem = main, destino = feature/login (a branch atual): quem muda é a feature.
    let dir = setup_diverged();
    let p = dir.path();
    git(p, &["checkout", "-q", "feature/login"]);
    let main_before = oid(p, "main");
    let feature_before = oid(p, "feature/login");

    let pv = preview(&dir, "main", "feature/login").unwrap();
    assert!(!pv.target_protected, "destino comum não exige confirmação específica");
    assert_eq!(pv.required_confirmation, None);
    let result = run_merge(&dir, &pv).unwrap();

    assert_eq!(result.source, "main");
    assert_eq!(result.target, "feature/login");
    assert_eq!(oid(p, "main"), main_before, "a origem não pode mudar");
    assert_ne!(oid(p, "feature/login"), feature_before);
    assert_eq!(read(p, "main-only.txt"), "só na main\n");
}

#[test]
fn unrelated_untracked_files_do_not_block_and_are_preserved() {
    let dir = setup_diverged();
    let p = dir.path();
    write(p, "rascunho com espaço.txt", "meu rascunho\n");

    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert!(pv.blockers.is_empty(), "{:?}", pv.blockers);
    assert!(pv.notes.iter().any(|n| n.contains("não rastreados")));
    run_merge(&dir, &pv).unwrap();
    assert_eq!(read(p, "rascunho com espaço.txt"), "meu rascunho\n");
}

#[test]
fn works_with_accents_in_branch_and_file_names() {
    let dir = init_repo();
    let p = dir.path();
    write(p, "base.txt", "base\n");
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "feature/ação-nova"]);
    write(p, "pasta com espaço/arquivo ç.txt", "conteúdo\n");
    commit_all(p, "novo");
    git(p, &["checkout", "-q", "main"]);

    let pv = preview(&dir, "feature/ação-nova", "main").unwrap();
    let result = run_merge(&dir, &pv).unwrap();
    assert_eq!(result.outcome, MergeOutcome::FastForward);
    assert_eq!(read(p, "pasta com espaço/arquivo ç.txt"), "conteúdo\n");
}

// ----------------------------------------------------------------------------
// Bloqueios
// ----------------------------------------------------------------------------

#[test]
fn missing_branches_and_same_branch_are_rejected() {
    let dir = setup_fast_forward();
    let e = preview(&dir, "nao-existe", "main").unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert!(e.message.contains("origem"));
    let e = preview(&dir, "feature/login", "nao-existe").unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert!(e.message.contains("destino"));
    let e = preview(&dir, "main", "main").unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert!(e.message.contains("mesma branch"));
    // Nomes que seriam interpretados como opção ou referência especial são recusados.
    assert_eq!(preview(&dir, "--help", "main").unwrap_err().code, ErrorCode::InvalidArgument);
    assert_eq!(preview(&dir, "HEAD", "main").unwrap_err().code, ErrorCode::InvalidArgument);
    // Uma tag com o nome de uma branch inexistente não é aceita como branch.
    git(dir.path(), &["tag", "so-tag"]);
    assert_eq!(preview(&dir, "so-tag", "main").unwrap_err().code, ErrorCode::InvalidArgument);
}

#[test]
fn invalid_repository_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let err = preview_merge(
        &locate(),
        &MergePreviewRequest {
            repo_path: fs::canonicalize(dir.path()).unwrap().to_string_lossy().into_owned(),
            source: "a".into(),
            target: "b".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotARepository);
    // E a execução também: nada é criado na pasta.
    let err = merge_branches(
        &locate(),
        &MergeRequest {
            repo_path: fs::canonicalize(dir.path()).unwrap().to_string_lossy().into_owned(),
            source: "a".into(),
            target: "b".into(),
            token: "x".into(),
            confirmed: true,
            confirm_target: None,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotARepository);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn staged_changes_block_and_are_preserved() {
    let dir = setup_fast_forward();
    let p = dir.path();
    write(p, "base.txt", "base alterado e preparado\n");
    git(p, &["add", "base.txt"]);

    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert!(!pv.can_execute);
    let b = pv.blockers.iter().find(|b| b.kind == BlockerKind::StagedChanges).unwrap();
    assert_eq!(b.files, vec!["base.txt".to_string()]);

    let head = oid(p, "HEAD");
    let err = run_merge(&dir, &pv).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.details.as_deref().unwrap_or("").contains("base.txt"));
    assert_eq!(oid(p, "HEAD"), head);
    assert_eq!(read(p, "base.txt"), "base alterado e preparado\n");
    assert!(git_out(p, &["diff", "--cached", "--name-only"]).contains("base.txt"));
    assert!(git_out(p, &["stash", "list"]).is_empty());
}

#[test]
fn unstaged_changes_block_and_are_preserved() {
    let dir = setup_diverged();
    let p = dir.path();
    write(p, "base.txt", "alteração local\n");

    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert!(has_blocker(&pv, BlockerKind::UnstagedChanges));
    let head = oid(p, "HEAD");
    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::OperationBlocked);
    assert_eq!(oid(p, "HEAD"), head);
    assert_eq!(read(p, "base.txt"), "alteração local\n");
    assert!(git_out(p, &["stash", "list"]).is_empty());
}

#[test]
fn untracked_file_that_would_be_overwritten_blocks() {
    let dir = setup_fast_forward();
    let p = dir.path();
    // `login.txt` ainda não existe na main, mas a feature o cria.
    write(p, "login.txt", "meu arquivo local, ainda não rastreado\n");

    let pv = preview(&dir, "feature/login", "main").unwrap();
    let b = pv.blockers.iter().find(|b| b.kind == BlockerKind::UntrackedOverwrite).unwrap();
    assert_eq!(b.files, vec!["login.txt".to_string()]);
    assert!(!pv.can_execute);

    let head = oid(p, "HEAD");
    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::OperationBlocked);
    assert_eq!(oid(p, "HEAD"), head);
    assert_eq!(read(p, "login.txt"), "meu arquivo local, ainda não rastreado\n");
}

#[test]
fn target_must_be_the_current_branch_and_no_silent_switch_happens() {
    let dir = setup_fast_forward();
    let p = dir.path();
    git(p, &["checkout", "-q", "feature/login"]);
    let main_before = oid(p, "main");

    // Destino = main, mas a branch atual é feature/login.
    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert!(!pv.target_is_current);
    assert!(has_blocker(&pv, BlockerKind::TargetNotCurrent));
    assert!(!pv.can_execute);

    let err = run_merge(&dir, &pv).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(git_out(p, &["symbolic-ref", "--short", "HEAD"]), "feature/login", "sem troca silenciosa");
    assert_eq!(oid(p, "main"), main_before);
}

#[test]
fn detached_head_blocks() {
    let dir = setup_fast_forward();
    let p = dir.path();
    git(p, &["checkout", "-q", "--detach", "main"]);

    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert!(has_blocker(&pv, BlockerKind::DetachedHead));
    assert!(has_blocker(&pv, BlockerKind::TargetNotCurrent));
    assert!(!pv.can_execute);
    let head = oid(p, "HEAD");
    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::OperationBlocked);
    assert_eq!(oid(p, "HEAD"), head);
    assert_eq!(oid(p, "main"), head);
}

#[test]
fn rebase_in_progress_blocks() {
    let dir = setup_conflict();
    let p = dir.path();
    // Um rebase real, interrompido por conflito.
    let out = git_cmd(p, &["rebase", "feature/login"]);
    assert!(!out.status.success(), "o rebase deveria parar por conflito");

    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert_eq!(status.operation, Some(Operation::Rebase));

    git(p, &["branch", "outra", "feature/login"]);
    // O destino (HEAD) está desanexado durante o rebase; a prévia precisa bloquear pela operação.
    let pv = preview(&dir, "outra", "main").unwrap();
    assert!(has_blocker(&pv, BlockerKind::OperationInProgress));
    assert!(!pv.can_execute);
    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::OperationBlocked);
    // O rebase continua exatamente como estava.
    let after = get_status(&locate(), &root_of(&dir)).unwrap();
    assert_eq!(after.operation, Some(Operation::Rebase));
}

// ----------------------------------------------------------------------------
// Conflitos
// ----------------------------------------------------------------------------

#[test]
fn conflicting_merge_reports_files_and_is_never_a_success() {
    let dir = setup_conflict();
    let p = dir.path();
    let main_before = oid(p, "main");
    let feature_oid = oid(p, "feature/login");

    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert!(pv.can_execute, "conflito possível não impede a tentativa: {:?}", pv.blockers);
    let result = run_merge(&dir, &pv).unwrap();

    assert_eq!(result.outcome, MergeOutcome::Conflicts);
    assert!(!result.merge_commit);
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].path, "shared.txt");
    assert_eq!(result.conflicts[0].code.as_deref(), Some("UU"));
    assert_ne!(result.exit_code, Some(0));
    assert_eq!(result.status.operation, Some(Operation::Merge));
    assert_eq!(result.status.summary.conflicted, 1);
    assert!(!result.output.is_empty(), "a saída real do Git precisa ser devolvida");

    // Nada foi descartado ou resolvido sozinho: HEAD igual, marcadores de conflito no arquivo.
    assert_eq!(oid(p, "HEAD"), main_before);
    assert_eq!(oid(p, "feature/login"), feature_oid);
    let content = read(p, "shared.txt");
    assert!(content.contains("<<<<<<<") && content.contains("MAIN") && content.contains("FEATURE"));
    assert_eq!(read(p, "outro.txt"), "outro\n");
    assert!(git_out(p, &["stash", "list"]).is_empty());
}

#[test]
fn merge_in_progress_and_previous_conflicts_block_new_merges() {
    let dir = setup_conflict();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert_eq!(run_merge(&dir, &pv).unwrap().outcome, MergeOutcome::Conflicts);

    git(p, &["branch", "outra", "feature/login"]);
    let again = preview(&dir, "outra", "main").unwrap();
    assert!(has_blocker(&again, BlockerKind::OperationInProgress));
    assert!(has_blocker(&again, BlockerKind::Conflicts));
    let conflict_blocker = again.blockers.iter().find(|b| b.kind == BlockerKind::Conflicts).unwrap();
    assert_eq!(conflict_blocker.files, vec!["shared.txt".to_string()]);
    assert!(!again.can_execute);

    let err = run_merge(&dir, &again).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    // O merge original continua em andamento, com o conflito intacto.
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert_eq!(status.operation, Some(Operation::Merge));
    assert!(read(p, "shared.txt").contains("<<<<<<<"));
}

#[test]
fn conflicts_resolved_manually_allow_new_merges_again() {
    let dir = setup_conflict();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    assert_eq!(run_merge(&dir, &pv).unwrap().outcome, MergeOutcome::Conflicts);

    // O usuário resolve manualmente (fora do aplicativo) e conclui.
    write(p, "shared.txt", "a\nRESOLVIDO\nc\n");
    git(p, &["add", "shared.txt"]);
    git(p, &["commit", "-q", "--no-edit"]);

    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert!(status.operation.is_none() && status.is_clean);
    assert_eq!(git_out(p, &["rev-list", "--parents", "-n", "1", "HEAD"]).split(' ').count(), 3);
}

// ----------------------------------------------------------------------------
// Confirmação e proteção da main (no backend)
// ----------------------------------------------------------------------------

#[test]
fn nothing_is_integrated_without_explicit_confirmation() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    let before = oid(p, "main");

    // Mesmo com a confirmação específica da main, `confirmed: false` impede a execução.
    let err = merge_branches(&locate(), &request(&dir, &pv, false, Some("main"))).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("confirm"));
    assert_eq!(oid(p, "main"), before);
    assert!(!p.join("login.txt").exists());
}

#[test]
fn main_requires_a_specific_confirmation_in_the_backend() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    let before = oid(p, "main");

    for typed in [None, Some(""), Some("sim"), Some("MAIN"), Some("master"), Some("main ")] {
        let err = merge_branches(&locate(), &request(&dir, &pv, true, typed)).unwrap_err();
        assert_eq!(err.code, ErrorCode::OperationBlocked, "confirmação {typed:?}");
        assert_eq!(oid(p, "main"), before, "confirmação {typed:?}");
    }
    assert!(!p.join("login.txt").exists());

    // Com a confirmação correta, a integração acontece.
    let result = merge_branches(&locate(), &request(&dir, &pv, true, Some("main"))).unwrap();
    assert_eq!(result.outcome, MergeOutcome::FastForward);
    assert_ne!(oid(p, "main"), before);
}

#[test]
fn master_is_protected_like_main() {
    let dir = init_repo();
    let p = dir.path();
    git(p, &["checkout", "-q", "-b", "master"]);
    write(p, "base.txt", "base\n");
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "feat"]);
    write(p, "f.txt", "f\n");
    commit_all(p, "f");
    git(p, &["checkout", "-q", "master"]);

    let pv = preview(&dir, "feat", "master").unwrap();
    assert!(pv.target_protected);
    let before = oid(p, "master");
    let err = merge_branches(&locate(), &request(&dir, &pv, true, None)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(oid(p, "master"), before);
    assert!(merge_branches(&locate(), &request(&dir, &pv, true, Some("master"))).is_ok());
}

#[test]
fn non_protected_target_only_needs_the_explicit_confirmation() {
    let dir = setup_diverged();
    let p = dir.path();
    git(p, &["checkout", "-q", "feature/login"]);
    let pv = preview(&dir, "main", "feature/login").unwrap();
    let before = oid(p, "feature/login");
    assert_eq!(
        merge_branches(&locate(), &request(&dir, &pv, false, None)).unwrap_err().code,
        ErrorCode::OperationBlocked
    );
    assert_eq!(oid(p, "feature/login"), before);
    assert!(merge_branches(&locate(), &request(&dir, &pv, true, None)).is_ok());
}

// ----------------------------------------------------------------------------
// Atualidade da revisão (token)
// ----------------------------------------------------------------------------

#[test]
fn new_commit_on_source_after_preview_invalidates_the_review() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    let main_before = oid(p, "main");

    git(p, &["checkout", "-q", "feature/login"]);
    write(p, "surpresa.txt", "novo\n");
    commit_all(p, "commit depois da prévia");
    git(p, &["checkout", "-q", "main"]);

    let err = run_merge(&dir, &pv).unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    assert_eq!(oid(p, "main"), main_before);
    assert!(!p.join("surpresa.txt").exists());

    // Uma nova revisão é aceita.
    let fresh = preview(&dir, "feature/login", "main").unwrap();
    assert_ne!(fresh.token, pv.token);
    assert_eq!(fresh.source_only, 3);
    assert!(run_merge(&dir, &fresh).is_ok());
}

#[test]
fn new_commit_on_target_after_preview_invalidates_the_review() {
    let dir = setup_diverged();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    write(p, "outro-commit.txt", "x\n");
    commit_all(p, "main mudou depois da prévia");
    let before = oid(p, "main");

    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::StaleReview);
    assert_eq!(oid(p, "main"), before);
}

#[test]
fn switching_branch_after_preview_invalidates_the_review() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    git(p, &["checkout", "-q", "feature/login"]);
    let main_before = oid(p, "main");

    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::StaleReview);
    assert_eq!(oid(p, "main"), main_before);
}

#[test]
fn local_changes_after_preview_invalidate_the_review() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    write(p, "novo-depois.txt", "x\n"); // arquivo não rastreado que não colide: ainda muda a revisão
    let before = oid(p, "main");

    assert_eq!(run_merge(&dir, &pv).unwrap_err().code, ErrorCode::StaleReview);
    assert_eq!(oid(p, "main"), before);

    // Alteração em arquivo rastreado: também é recusada (stale, antes mesmo do bloqueio).
    fs::remove_file(p.join("novo-depois.txt")).unwrap();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    write(p, "base.txt", "mudou\n");
    let err = run_merge(&dir, &pv).unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    assert_eq!(read(p, "base.txt"), "mudou\n");
}

#[test]
fn a_token_from_another_selection_is_rejected() {
    let dir = setup_diverged();
    let p = dir.path();
    git(p, &["branch", "feature/outra", "feature/login"]);
    let pv = preview(&dir, "feature/outra", "main").unwrap();
    let mut req = request(&dir, &pv, true, Some("main"));
    req.source = "feature/login".into(); // mesma OID, mas outra branch: a revisão não vale
    let before = oid(p, "main");
    assert_eq!(merge_branches(&locate(), &req).unwrap_err().code, ErrorCode::StaleReview);
    assert_eq!(oid(p, "main"), before);

    let mut forged = request(&dir, &pv, true, Some("main"));
    forged.token = "token-inventado".into();
    assert_eq!(merge_branches(&locate(), &forged).unwrap_err().code, ErrorCode::StaleReview);
    assert_eq!(oid(p, "main"), before);
}

// ----------------------------------------------------------------------------
// Concorrência e falhas inesperadas
// ----------------------------------------------------------------------------

#[test]
fn concurrent_operation_on_the_same_repository_is_refused() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    let before = oid(p, "main");

    let guard = RepoGuard::acquire(std::path::Path::new(&root_of(&dir))).unwrap();
    let err = run_merge(&dir, &pv).unwrap_err();
    assert_eq!(err.code, ErrorCode::RepositoryBusy);
    assert_eq!(oid(p, "main"), before);
    drop(guard);

    // Liberada a trava, a mesma revisão ainda vale.
    assert!(run_merge(&dir, &pv).is_ok());
}

#[test]
fn missing_git_executable_fails_without_touching_the_repository() {
    let dir = setup_fast_forward();
    let p = dir.path();
    let pv = preview(&dir, "feature/login", "main").unwrap();
    let before = oid(p, "main");
    let broken = Git::with_program("git-que-nao-existe-gitflow");
    let err = merge_branches(&broken, &request(&dir, &pv, true, Some("main"))).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitNotFound);
    assert_eq!(oid(p, "main"), before);
}

#[cfg(unix)]
#[test]
fn hook_rejecting_the_merge_commit_is_reported_as_failure_not_success() {
    use std::os::unix::fs::PermissionsExt;
    let dir = setup_diverged();
    let p = dir.path();
    let hook = p.join(".git/hooks/pre-merge-commit");
    fs::write(&hook, "#!/bin/sh\necho recusado pelo hook >&2\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let main_before = oid(p, "main");

    let pv = preview(&dir, "feature/login", "main").unwrap();
    let err = run_merge(&dir, &pv).unwrap_err();
    assert_eq!(err.code, ErrorCode::CommandFailed);
    assert!(err.message.contains("em andamento"), "{}", err.message);
    assert!(err.details.as_deref().unwrap_or("").contains("recusado pelo hook"));
    // O repositório é descrito como está: merge pendente, HEAD ainda no commit anterior.
    assert_eq!(oid(p, "HEAD"), main_before);
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert_eq!(status.operation, Some(Operation::Merge));
    assert_eq!(status.summary.conflicted, 0);
}

#[test]
fn merge_never_uses_destructive_options() {
    // O merge com conflito não pode ter descartado nada nem guardado alterações em stash,
    // mesmo com `merge.autoStash`/`rerere` ligados na configuração do repositório.
    let dir = setup_conflict();
    let p = dir.path();
    git(p, &["config", "merge.autoStash", "true"]);
    git(p, &["config", "rerere.enabled", "true"]);
    git(p, &["config", "rerere.autoUpdate", "true"]);
    let pv = preview(&dir, "feature/login", "main").unwrap();
    let result = run_merge(&dir, &pv).unwrap();
    assert_eq!(result.outcome, MergeOutcome::Conflicts);
    assert!(git_out(p, &["stash", "list"]).is_empty());
    assert!(read(p, "shared.txt").contains("<<<<<<<"));
}
