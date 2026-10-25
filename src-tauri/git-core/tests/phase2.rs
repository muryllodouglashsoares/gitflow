//! Testes de integração da Fase 2: branches locais, seleção explícita, revisão e commit seletivo.
//! Usam o Git real contra repositórios temporários descartáveis (nunca o repositório de desenvolvimento).

use gitflow_git::{
    create_branch, create_commit, get_file_diff, get_status, list_branches, prepare_commit_review,
    switch_branch, CommitRequest, CreateBranchRequest, DiffRequest, DiffTarget, ErrorCode, Git,
    ReviewRequest, SwitchBranchRequest,
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

/// Repositório com identidade configurada localmente (o app usa o Git sem variáveis de autor).
fn init_repo() -> TempDir {
    let dir = tempfile::Builder::new().prefix("gitflow-fase2-").tempdir().unwrap();
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

fn head(dir: &Path) -> String {
    git_out(dir, &["rev-parse", "HEAD"])
}

fn review_request(dir: &TempDir, files: &[&str]) -> ReviewRequest {
    ReviewRequest {
        repo_path: root_of(dir),
        files: files.iter().map(|s| s.to_string()).collect(),
    }
}

/// Revisão + commit, como o aplicativo faz.
fn commit_files(dir: &TempDir, files: &[&str], message: &str) -> gitflow_git::Result<gitflow_git::CommitResult> {
    let git = locate();
    let review = prepare_commit_review(&git, &review_request(dir, files))?;
    create_commit(
        &git,
        &CommitRequest {
            repo_path: root_of(dir),
            files: files.iter().map(|s| s.to_string()).collect(),
            message: message.to_string(),
            expected_branch: review.branch.clone(),
            fingerprint: review.fingerprint.clone(),
        },
    )
}

fn base_repo() -> TempDir {
    let dir = init_repo();
    write(dir.path(), "a.txt", "a\n");
    write(dir.path(), "b.txt", "b\n");
    write(dir.path(), "c.txt", "c\n");
    commit_all(dir.path(), "base");
    dir
}

// ----------------------------------------------------------------------------
// Branches (cenários 1–8)
// ----------------------------------------------------------------------------

#[test]
fn lists_only_local_branches_and_marks_the_current_one() {
    let dir = base_repo();
    git(dir.path(), &["branch", "feature/x"]);
    // Uma referência remota não pode aparecer como branch local.
    git(dir.path(), &["update-ref", "refs/remotes/origin/remota", "HEAD"]);

    let list = list_branches(&locate(), &root_of(&dir)).unwrap();
    let names: Vec<&str> = list.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, vec!["feature/x", "main"]);
    assert_eq!(list.current.as_deref(), Some("main"));
    assert!(list.branches.iter().find(|b| b.name == "main").unwrap().current);
    assert!(!list.branches.iter().find(|b| b.name == "feature/x").unwrap().current);
    assert!(!list.detached && !list.unborn);
}

#[test]
fn reports_unborn_branch_and_detached_head() {
    let dir = init_repo();
    let list = list_branches(&locate(), &root_of(&dir)).unwrap();
    assert!(list.unborn);
    assert!(list.branches.is_empty());
    assert_eq!(list.current.as_deref(), Some("main"));

    write(dir.path(), "a.txt", "a\n");
    commit_all(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "--detach"]);
    let list = list_branches(&locate(), &root_of(&dir)).unwrap();
    assert!(list.detached);
    assert_eq!(list.current, None);
}

fn create_req(dir: &TempDir, name: &str, base: &str) -> CreateBranchRequest {
    CreateBranchRequest {
        repo_path: root_of(dir),
        name: name.to_string(),
        base: base.to_string(),
    }
}

#[test]
fn creates_branch_without_switching() {
    let dir = base_repo();
    let created = create_branch(&locate(), &create_req(&dir, "feature/nova", "main")).unwrap();
    assert_eq!(created.name, "feature/nova");
    assert_eq!(created.oid, head(dir.path()));
    // A branch atual não muda silenciosamente.
    assert_eq!(git_out(dir.path(), &["symbolic-ref", "--short", "HEAD"]), "main");
    assert!(!git_out(dir.path(), &["branch", "--list", "feature/nova"]).is_empty());
}

#[test]
fn rejects_invalid_branch_names_without_touching_git_refs() {
    let dir = base_repo();
    let before = git_out(dir.path(), &["for-each-ref"]);
    for name in ["", "  ", "com espaço", "-opcao", "--force", "a..b", "x.lock", "a//b", "HEAD", "@{-1}", "a~b", "a:b", "a\\b"] {
        let err = create_branch(&locate(), &create_req(&dir, name, "main")).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "nome {name:?}");
    }
    assert_eq!(git_out(dir.path(), &["for-each-ref"]), before);
}

#[test]
fn rejects_existing_conflicting_and_unknown_base() {
    let dir = base_repo();
    git(dir.path(), &["branch", "Feature"]);
    for name in ["main", "Feature", "feature", "main/sub", "Feature/sub"] {
        let err = create_branch(&locate(), &create_req(&dir, name, "main")).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "nome {name:?}");
    }
    let err = create_branch(&locate(), &create_req(&dir, "ok", "nao-existe")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[test]
fn refuses_to_create_branches_before_the_first_commit() {
    let dir = init_repo();
    let err = create_branch(&locate(), &create_req(&dir, "x", "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
}

fn switch_req(dir: &TempDir, name: &str) -> SwitchBranchRequest {
    SwitchBranchRequest {
        repo_path: root_of(dir),
        name: name.to_string(),
    }
}

#[test]
fn switches_branch_in_clean_repository_and_reports_real_result() {
    let dir = base_repo();
    git(dir.path(), &["branch", "outra"]);
    let result = switch_branch(&locate(), &switch_req(&dir, "outra")).unwrap();
    assert_eq!(result.current, "outra");
    assert_eq!(result.previous.as_deref(), Some("main"));
    assert_eq!(git_out(dir.path(), &["symbolic-ref", "--short", "HEAD"]), "outra");
}

#[test]
fn blocks_switch_when_tracked_changes_exist_and_preserves_them() {
    let dir = base_repo();
    git(dir.path(), &["branch", "outra"]);
    write(dir.path(), "a.txt", "alterado, não preparado\n");
    write(dir.path(), "b.txt", "alterado e preparado\n");
    git(dir.path(), &["add", "b.txt"]);

    let err = switch_branch(&locate(), &switch_req(&dir, "outra")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(git_out(dir.path(), &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(read(dir.path(), "a.txt"), "alterado, não preparado\n");
    assert_eq!(read(dir.path(), "b.txt"), "alterado e preparado\n");
    assert!(git_out(dir.path(), &["diff", "--cached", "--name-only"]).contains("b.txt"));
}

#[test]
fn lets_git_refuse_a_switch_that_would_overwrite_untracked_files() {
    let dir = base_repo();
    git(dir.path(), &["switch", "-q", "-c", "outra"]);
    write(dir.path(), "novo.txt", "da outra branch\n");
    commit_all(dir.path(), "novo na outra");
    git(dir.path(), &["switch", "-q", "main"]);
    write(dir.path(), "novo.txt", "arquivo local não rastreado\n");

    let err = switch_branch(&locate(), &switch_req(&dir, "outra")).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(read(dir.path(), "novo.txt"), "arquivo local não rastreado\n");
    assert_eq!(git_out(dir.path(), &["symbolic-ref", "--short", "HEAD"]), "main");
}

#[test]
fn rejects_missing_target_and_same_branch() {
    let dir = base_repo();
    let err = switch_branch(&locate(), &switch_req(&dir, "inexistente")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = switch_branch(&locate(), &switch_req(&dir, "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = switch_branch(&locate(), &switch_req(&dir, "--force")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(git_out(dir.path(), &["symbolic-ref", "--short", "HEAD"]), "main");
}

// ----------------------------------------------------------------------------
// Seleção e staging (cenários 9–16)
// ----------------------------------------------------------------------------

#[test]
fn commits_only_the_selected_file_and_leaves_the_others_untouched() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "a alterado\n");
    write(dir.path(), "b.txt", "b alterado\n");
    write(dir.path(), "c.txt", "c alterado\n");

    let result = commit_files(&dir, &["a.txt"], "só o a").unwrap();
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].path, "a.txt");

    // Os não selecionados continuam alterados e fora do índice, com o conteúdo intacto.
    assert_eq!(read(dir.path(), "b.txt"), "b alterado\n");
    assert_eq!(read(dir.path(), "c.txt"), "c alterado\n");
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    let paths: Vec<&str> = status.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["b.txt", "c.txt"]);
    assert!(status.files.iter().all(|f| f.index.is_none() && f.worktree.is_some()));
}

#[test]
fn preserves_previously_staged_work_that_was_not_selected() {
    let dir = base_repo();
    write(dir.path(), "b.txt", "b preparado antes\n");
    git(dir.path(), &["add", "b.txt"]);
    write(dir.path(), "a.txt", "a alterado\n");

    let review = prepare_commit_review(&locate(), &review_request(&dir, &["a.txt"])).unwrap();
    assert_eq!(review.preserved_staged, vec!["b.txt".to_string()]);

    commit_files(&dir, &["a.txt"], "só o a").unwrap();

    // b.txt NÃO entrou no commit e continua preparado, com o mesmo conteúdo.
    assert_eq!(git_out(dir.path(), &["show", "--name-only", "--format=", "HEAD"]), "a.txt");
    assert_eq!(git_out(dir.path(), &["diff", "--cached", "--name-only"]), "b.txt");
    assert_eq!(read(dir.path(), "b.txt"), "b preparado antes\n");
}

#[test]
fn handles_untracked_files_including_hostile_names() {
    let dir = base_repo();
    let names = ["novo arquivo.txt", "-começa-com-hífen.txt", "colchetes [1] *.txt", "ação ç.txt", "dir com espaço/aninhado/x.txt"];
    for n in names {
        write(dir.path(), n, "novo\n");
    }
    write(dir.path(), "fora.txt", "não selecionado\n");

    let result = commit_files(&dir, &names, "arquivos novos").unwrap();
    assert_eq!(result.files.len(), names.len());
    for n in names {
        let tracked = git_out(
            dir.path(),
            &["--literal-pathspecs", "-c", "core.quotepath=off", "ls-files", "--error-unmatch", "--", n],
        );
        assert_eq!(tracked, n);
    }
    // O não selecionado continua não rastreado.
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert_eq!(status.files.len(), 1);
    assert!(status.files[0].untracked && status.files[0].path == "fora.txt");
}

#[test]
fn partially_staged_file_is_reviewed_and_committed_as_its_current_content() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "a\nlinha preparada\n");
    git(dir.path(), &["add", "a.txt"]);
    write(dir.path(), "a.txt", "a\nlinha preparada\nlinha não preparada\n");

    let review = prepare_commit_review(&locate(), &review_request(&dir, &["a.txt"])).unwrap();
    assert!(review.files[0].has_staged && review.files[0].has_unstaged);
    assert_eq!(review.files[0].diff_target, DiffTarget::Head);

    // O diff da revisão mostra as DUAS linhas: é exatamente o que será registrado.
    let diff = get_file_diff(
        &locate(),
        &DiffRequest {
            repo_path: root_of(&dir),
            path: "a.txt".into(),
            orig_path: None,
            target: DiffTarget::Head,
        },
    )
    .unwrap();
    assert_eq!(diff.additions, 2);

    commit_files(&dir, &["a.txt"], "parcial").unwrap();
    assert_eq!(git_out(dir.path(), &["show", "HEAD:a.txt"]), "a\nlinha preparada\nlinha não preparada");
    assert!(git_out(dir.path(), &["status", "--porcelain"]).is_empty());
}

#[test]
fn renamed_files_are_committed_with_both_paths() {
    let dir = base_repo();
    git(dir.path(), &["mv", "a.txt", "renomeado.txt"]);
    commit_files(&dir, &["renomeado.txt"], "renomeia").unwrap();
    assert!(git_out(dir.path(), &["ls-tree", "--name-only", "HEAD"]).contains("renomeado.txt"));
    assert!(!git_out(dir.path(), &["ls-tree", "--name-only", "HEAD"]).lines().any(|l| l == "a.txt"));
}

#[test]
fn deleted_files_can_be_selected() {
    let dir = base_repo();
    fs::remove_file(dir.path().join("a.txt")).unwrap();
    commit_files(&dir, &["a.txt"], "remove a").unwrap();
    assert!(!git_out(dir.path(), &["ls-tree", "--name-only", "HEAD"]).lines().any(|l| l == "a.txt"));
}

#[test]
fn first_commit_in_an_empty_repository_works() {
    let dir = init_repo();
    write(dir.path(), "um.txt", "1\n");
    write(dir.path(), "dois.txt", "2\n");
    let result = commit_files(&dir, &["um.txt"], "primeiro").unwrap();
    assert_eq!(result.oid, head(dir.path()));
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert_eq!(status.files.len(), 1);
    assert_eq!(status.files[0].path, "dois.txt");
}

#[test]
fn conflicted_files_and_unrelated_paths_are_rejected() {
    let dir = base_repo();
    git(dir.path(), &["switch", "-q", "-c", "outra"]);
    write(dir.path(), "a.txt", "versão da outra\n");
    commit_all(dir.path(), "outra");
    git(dir.path(), &["switch", "-q", "main"]);
    write(dir.path(), "a.txt", "versão da main\n");
    commit_all(dir.path(), "main");
    let _ = git_cmd(dir.path(), &["merge", "outra"]); // gera conflito de propósito

    let err = prepare_commit_review(&locate(), &review_request(&dir, &["a.txt"])).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    let err = create_commit(
        &locate(),
        &CommitRequest {
            repo_path: root_of(&dir),
            files: vec!["a.txt".into()],
            message: "x".into(),
            expected_branch: "main".into(),
            fingerprint: "0".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
}

#[test]
fn rejects_unsafe_or_unknown_selections() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "x\n");
    for bad in ["../fora.txt", "/etc/passwd", "nao-alterado.txt", "b.txt"] {
        let err = prepare_commit_review(&locate(), &review_request(&dir, &[bad])).unwrap_err();
        assert!(
            matches!(err.code, ErrorCode::InvalidArgument | ErrorCode::StaleReview),
            "{bad:?} -> {:?}",
            err.code
        );
    }
    let err = prepare_commit_review(&locate(), &review_request(&dir, &[])).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = prepare_commit_review(&locate(), &review_request(&dir, &["a.txt", "a.txt"])).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

// ----------------------------------------------------------------------------
// Diff, mensagem e resultado (cenários 17–24)
// ----------------------------------------------------------------------------

#[test]
fn review_content_matches_committed_content_and_stale_review_is_refused() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "primeira versão revisada\n");
    let git = locate();
    let review = prepare_commit_review(&git, &review_request(&dir, &["a.txt"])).unwrap();

    // O arquivo muda DEPOIS da revisão: o commit precisa ser recusado e nada é registrado.
    write(dir.path(), "a.txt", "segunda versão, não revisada\n");
    let before = head(dir.path());
    let err = create_commit(
        &git,
        &CommitRequest {
            repo_path: root_of(&dir),
            files: vec!["a.txt".into()],
            message: "x".into(),
            expected_branch: review.branch.clone(),
            fingerprint: review.fingerprint.clone(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    assert_eq!(head(dir.path()), before);
    assert_eq!(read(dir.path(), "a.txt"), "segunda versão, não revisada\n");

    // Nova revisão -> novo fingerprint -> commit aceito com o conteúdo revisado.
    let review2 = prepare_commit_review(&git, &review_request(&dir, &["a.txt"])).unwrap();
    assert_ne!(review.fingerprint, review2.fingerprint);
    commit_files(&dir, &["a.txt"], "ok").unwrap();
    assert_eq!(git_out(dir.path(), &["show", "HEAD:a.txt"]), "segunda versão, não revisada");
}

#[test]
fn review_diff_updates_when_selection_changes() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "a2\n");
    write(dir.path(), "b.txt", "b2\n");
    let git = locate();
    let one = prepare_commit_review(&git, &review_request(&dir, &["a.txt"])).unwrap();
    let two = prepare_commit_review(&git, &review_request(&dir, &["a.txt", "b.txt"])).unwrap();
    assert_eq!(one.files.len(), 1);
    assert_eq!(two.files.len(), 2);
    assert_ne!(one.fingerprint, two.fingerprint);
}

#[test]
fn rejects_empty_message_and_does_not_commit() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "x\n");
    let before = head(dir.path());
    for msg in ["", "   ", "\n\t \n"] {
        let err = commit_files(&dir, &["a.txt"], msg).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "mensagem {msg:?}");
    }
    assert_eq!(head(dir.path()), before);
}

#[test]
fn successful_commit_reports_the_real_hash_branch_and_files() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "novo\n");
    let before = head(dir.path());
    let result = commit_files(&dir, &["a.txt"], "-mensagem com hífen\n\ncorpo").unwrap();

    assert_ne!(result.oid, before);
    assert_eq!(result.oid, head(dir.path()));
    assert!(result.oid.starts_with(&result.short_oid));
    assert_eq!(result.branch, "main");
    assert_eq!(result.subject, "-mensagem com hífen");
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].status, "M");
    assert!(result.summary.is_some());
    assert_eq!(git_out(dir.path(), &["log", "-1", "--format=%B"]), "-mensagem com hífen\n\ncorpo");

    // Estado depois do commit: limpo.
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    assert!(status.is_clean);
}

#[test]
fn failed_commit_keeps_user_data_and_restores_untracked_state() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "alterado\n");
    write(dir.path(), "novo.txt", "novo\n");

    // Um hook que recusa o commit simula uma falha real do Git.
    let hook = dir.path().join(".git").join("hooks").join("pre-commit");
    fs::write(&hook, "#!/bin/sh\necho recusado >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    if cfg!(windows) {
        return; // hooks de shell dependem do Git Bash; o cenário é validado em Unix.
    }

    let before = head(dir.path());
    let err = commit_files(&dir, &["a.txt", "novo.txt"], "vai falhar").unwrap_err();
    assert_eq!(err.code, ErrorCode::CommandFailed);
    assert!(err.details.unwrap_or_default().contains("recusado"));

    assert_eq!(head(dir.path()), before);
    assert_eq!(read(dir.path(), "a.txt"), "alterado\n");
    assert_eq!(read(dir.path(), "novo.txt"), "novo\n");
    // `novo.txt` voltou a ser não rastreado (o intent-to-add foi desfeito).
    let status = get_status(&locate(), &root_of(&dir)).unwrap();
    let novo = status.files.iter().find(|f| f.path == "novo.txt").unwrap();
    assert!(novo.untracked && novo.index.is_none());
}

#[test]
fn never_pushes_and_never_touches_remotes() {
    let dir = base_repo();
    let remote = tempfile::Builder::new().prefix("gitflow-remoto-").tempdir().unwrap();
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
    let remote_path = remote.path().to_string_lossy().into_owned();
    git(dir.path(), &["remote", "add", "origin", &remote_path]);
    git(dir.path(), &["push", "-q", "origin", "main"]);
    let remote_head = git_out(remote.path(), &["rev-parse", "main"]);

    write(dir.path(), "a.txt", "mudou\n");
    commit_files(&dir, &["a.txt"], "local").unwrap();

    assert_eq!(git_out(remote.path(), &["rev-parse", "main"]), remote_head);
    assert_ne!(head(dir.path()), remote_head);
}

#[test]
fn blocks_commit_during_detached_head() {
    let dir = base_repo();
    write(dir.path(), "a.txt", "x\n");
    git(dir.path(), &["checkout", "-q", "--detach"]);
    let err = prepare_commit_review(&locate(), &review_request(&dir, &["a.txt"])).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
}
