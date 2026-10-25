//! Testes de integração: usam o Git real contra repositórios temporários.
//! Nenhum projeto real é tocado; tudo acontece em pastas criadas por `tempfile`.

use gitflow_git::{
    diff::LineKind,
    get_file_diff, get_status,
    status::{ChangeKind, Operation},
    DiffContent, DiffRequest, DiffTarget, ErrorCode, FileDiff, Git, RepositoryStatus,
};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::Path,
    process::Command,
    time::{Duration, Instant, SystemTime},
};
use tempfile::TempDir;

// ----------------------------------------------------------------------------
// Utilitários de fixture (usam o Git real, isolado da configuração do usuário)
// ----------------------------------------------------------------------------

fn git_cmd(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Teste")
        .env("GIT_AUTHOR_EMAIL", "teste@example.com")
        .env("GIT_COMMITTER_NAME", "Teste")
        .env("GIT_COMMITTER_EMAIL", "teste@example.com")
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

fn init_repo_named(prefix: &str, branch: &str) -> TempDir {
    let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", branch]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

fn init_repo() -> TempDir {
    init_repo_named("gitflow-test-", "main")
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn root_of(dir: &TempDir) -> String {
    fs::canonicalize(dir.path()).unwrap().to_string_lossy().into_owned()
}

fn git_available() -> Git {
    Git::locate().expect("Git não localizado")
}

fn status_of(dir: &TempDir) -> RepositoryStatus {
    get_status(&git_available(), &root_of(dir)).expect("status deveria funcionar")
}

fn diff_of(dir: &TempDir, path: &str, orig: Option<&str>, target: DiffTarget) -> FileDiff {
    let request = DiffRequest {
        repo_path: root_of(dir),
        path: path.to_string(),
        orig_path: orig.map(str::to_string),
        target,
    };
    get_file_diff(&git_available(), &request).expect("diff deveria funcionar")
}

fn entry<'a>(status: &'a RepositoryStatus, path: &str) -> &'a gitflow_git::status::FileEntry {
    status
        .files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path:?} não encontrado em {:?}", status.files))
}

/// Impressão digital de toda a árvore (incluindo `.git`): conteúdo e data de modificação.
fn snapshot(dir: &Path) -> Vec<(String, u64, Option<SystemTime>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, u64, Option<SystemTime>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                walk(base, &path, out);
            } else if meta.is_file() {
                let mut h = DefaultHasher::new();
                fs::read(&path).unwrap().hash(&mut h);
                out.push((rel, h.finish(), meta.modified().ok()));
            } else {
                out.push((rel, 0, None));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

// ----------------------------------------------------------------------------
// Seleção e validação de repositórios
// ----------------------------------------------------------------------------

#[test]
fn folder_that_is_not_a_repository_is_identified() {
    let dir = tempfile::tempdir().unwrap();
    let err = gitflow_git::open_repository(&git_available(), &root_of(&dir)).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotARepository);
    assert_eq!(err.message, "A pasta selecionada não é um repositório Git.");
}

#[test]
fn missing_folder_and_regular_file_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nao-existe");
    let git = git_available();
    let err = gitflow_git::open_repository(&git, &missing.to_string_lossy()).unwrap_err();
    assert_eq!(err.code, ErrorCode::PathNotFound);

    write(dir.path(), "arquivo.txt", "x");
    let file = fs::canonicalize(dir.path().join("arquivo.txt")).unwrap();
    let err = gitflow_git::open_repository(&git, &file.to_string_lossy()).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotADirectory);
}

#[test]
fn missing_git_produces_an_understandable_error() {
    let dir = init_repo();
    let ghost = Git::with_program("/caminho/inexistente/git-que-nao-existe");
    let err = gitflow_git::open_repository(&ghost, &root_of(&dir)).unwrap_err();
    assert_eq!(err.code, ErrorCode::GitNotFound);
    assert!(err.message.contains("Git"), "{}", err.message);
    assert!(err.message.contains("git-scm.com"));
}

#[test]
fn valid_repository_is_identified_with_real_branch_name() {
    let dir = init_repo_named("gitflow-test-", "trunk");
    write(dir.path(), "a.txt", "um\n");
    commit_all(dir.path(), "inicial");

    let info = gitflow_git::open_repository(&git_available(), &root_of(&dir)).unwrap();
    assert_eq!(info.path, root_of(&dir));
    assert_eq!(
        info.name,
        fs::canonicalize(dir.path()).unwrap().file_name().unwrap().to_string_lossy()
    );
    assert!(!info.resolved_from_subfolder);

    let status = status_of(&dir);
    assert_eq!(status.branch.name.as_deref(), Some("trunk")); // não presume "main"
    assert!(!status.branch.unborn && !status.branch.detached);
    assert!(status.branch.head_oid.is_some());
}

#[test]
fn subfolder_resolves_to_repository_root_but_commands_require_the_root() {
    let dir = init_repo();
    write(dir.path(), "src/lib/a.txt", "x\n");
    commit_all(dir.path(), "inicial");
    let sub = fs::canonicalize(dir.path().join("src/lib")).unwrap();

    let git = git_available();
    let info = gitflow_git::open_repository(&git, &sub.to_string_lossy()).unwrap();
    assert_eq!(info.path, root_of(&dir));
    assert!(info.resolved_from_subfolder);
    assert_eq!(info.selected_path, sub.to_string_lossy());

    // Os comandos de leitura não aceitam caminhos arbitrários: só a raiz.
    let err = get_status(&git, &sub.to_string_lossy()).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[test]
fn bare_repository_is_rejected_with_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "--bare"]);
    let err = gitflow_git::open_repository(&git_available(), &root_of(&dir)).unwrap_err();
    assert_eq!(err.code, ErrorCode::BareRepository);
}

// ----------------------------------------------------------------------------
// Estado do repositório
// ----------------------------------------------------------------------------

#[test]
fn clean_repository_is_reported_as_clean() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "um\n");
    commit_all(dir.path(), "inicial");

    let status = status_of(&dir);
    assert!(status.is_clean);
    assert!(status.files.is_empty());
    assert_eq!(status.summary.total, 0);
    assert_eq!(status.operation, None);
}

#[test]
fn repository_without_commits_is_reported_as_unborn() {
    let dir = init_repo_named("gitflow-test-", "nova");
    write(dir.path(), "a.txt", "um\n");

    let status = status_of(&dir);
    assert!(status.branch.unborn);
    assert_eq!(status.branch.name.as_deref(), Some("nova"));
    assert!(entry(&status, "a.txt").untracked);

    // diff de arquivo preparado também funciona sem HEAD
    git(dir.path(), &["add", "a.txt"]);
    let diff = diff_of(&dir, "a.txt", None, DiffTarget::Staged);
    assert_eq!(diff.content, DiffContent::Text);
    assert!(diff.is_new);
}

#[test]
fn detects_modified_added_deleted_renamed_and_untracked_files() {
    let dir = init_repo();
    let p = dir.path();
    for f in ["a.txt", "b.txt", "c.txt"] {
        write(p, f, &format!("conteúdo de {f}\nlinha 2\nlinha 3\n"));
    }
    commit_all(p, "inicial");

    write(p, "a.txt", "conteúdo de a.txt\nlinha 2\nlinha 3\nlinha 4\n"); // modificado
    fs::remove_file(p.join("b.txt")).unwrap(); // removido
    git(p, &["mv", "c.txt", "c2.txt"]); // renomeado (staged)
    write(p, "d.txt", "novo\n");
    git(p, &["add", "d.txt"]); // adicionado (staged)
    write(p, "f.txt", "v1\n");
    git(p, &["add", "f.txt"]);
    write(p, "f.txt", "v1\nv2\n"); // staged + não preparado
    write(p, "e.txt", "solto\n"); // não rastreado

    let status = status_of(&dir);
    assert!(!status.is_clean);

    let a = entry(&status, "a.txt");
    assert_eq!((a.index, a.worktree), (None, Some(ChangeKind::Modified)));
    let b = entry(&status, "b.txt");
    assert_eq!((b.index, b.worktree), (None, Some(ChangeKind::Deleted)));
    let c2 = entry(&status, "c2.txt");
    assert_eq!(c2.index, Some(ChangeKind::Renamed));
    assert_eq!(c2.orig_path.as_deref(), Some("c.txt"));
    let d = entry(&status, "d.txt");
    assert_eq!((d.index, d.worktree), (Some(ChangeKind::Added), None));
    let f = entry(&status, "f.txt");
    assert_eq!((f.index, f.worktree), (Some(ChangeKind::Added), Some(ChangeKind::Modified)));
    let e = entry(&status, "e.txt");
    assert!(e.untracked && e.index.is_none() && e.worktree.is_none());

    assert_eq!(status.summary.total, 6);
    assert_eq!(status.summary.staged, 3); // c2, d, f
    assert_eq!(status.summary.unstaged, 3); // a, b, f
    assert_eq!(status.summary.untracked, 1);
    assert_eq!(status.summary.conflicted, 0);
}

#[test]
fn untracked_files_inside_new_folders_are_listed_individually() {
    let dir = init_repo();
    write(dir.path(), "x.txt", "x\n");
    commit_all(dir.path(), "inicial");
    write(dir.path(), "nova/pasta/um.txt", "1\n");
    write(dir.path(), "nova/pasta/dois.txt", "2\n");

    let status = status_of(&dir);
    assert!(entry(&status, "nova/pasta/um.txt").untracked);
    assert!(entry(&status, "nova/pasta/dois.txt").untracked);
}

#[test]
fn gitignored_files_are_not_listed() {
    let dir = init_repo();
    write(dir.path(), ".gitignore", "*.log\n");
    commit_all(dir.path(), "inicial");
    write(dir.path(), "debug.log", "ruído\n");

    let status = status_of(&dir);
    assert!(status.is_clean, "{:?}", status.files);
}

#[test]
fn paths_with_spaces_accents_and_special_characters() {
    // A própria pasta do repositório também tem espaços e acentos.
    let dir = init_repo_named("repo com espaço ç ", "main");
    let p = dir.path();
    let mut names = vec![
        "arquivo com espaços.txt".to_string(),
        "pasta com espaço/arquivo ç ã [1] (2) #3 &4.txt".to_string(),
        "-comeca-com-traco.txt".to_string(),
        "*.txt".to_string(),
        "emoji 🚀.txt".to_string(),
    ];
    if cfg!(unix) {
        names.push("com\"aspas'.txt".to_string());
        names.push("com\ttab.txt".to_string());
    }
    for n in &names {
        write(p, n, "original\n");
    }
    commit_all(p, "inicial");
    for n in &names {
        write(p, n, "original\nmodificado\n");
    }
    write(p, "novo arquivo ç.txt", "novo\n");

    let status = status_of(&dir);
    for n in &names {
        assert_eq!(entry(&status, n).worktree, Some(ChangeKind::Modified), "{n}");
        let diff = diff_of(&dir, n, None, DiffTarget::Unstaged);
        assert_eq!(diff.content, DiffContent::Text, "{n}");
        assert_eq!(diff.additions, 1, "{n}");
    }
    assert!(entry(&status, "novo arquivo ç.txt").untracked);
    let diff = diff_of(&dir, "novo arquivo ç.txt", None, DiffTarget::Untracked);
    assert_eq!(diff.content, DiffContent::Text);
    assert!(diff.is_new);
    assert_eq!(diff.hunks[0].lines[0].text, "novo");
}

#[test]
fn upstream_ahead_and_behind_are_reported() {
    let origin = init_repo();
    write(origin.path(), "a.txt", "1\n");
    commit_all(origin.path(), "inicial");

    let clone_parent = tempfile::tempdir().unwrap();
    let out = git_cmd(
        clone_parent.path(),
        &["clone", "-q", &root_of(&origin), "clone"],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let clone = clone_parent.path().join("clone");
    git(&clone, &["config", "commit.gpgsign", "false"]);

    write(&clone, "b.txt", "local\n");
    commit_all(&clone, "local");
    write(origin.path(), "c.txt", "remoto\n");
    commit_all(origin.path(), "remoto");
    git(&clone, &["fetch", "-q"]);

    let status = get_status(&git_available(), &fs::canonicalize(&clone).unwrap().to_string_lossy()).unwrap();
    assert_eq!(status.branch.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.branch.ahead, status.branch.behind), (Some(1), Some(1)));
}

#[test]
fn detached_head_is_reported() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "1\n");
    commit_all(dir.path(), "inicial");
    git(dir.path(), &["checkout", "-q", "--detach"]);

    let status = status_of(&dir);
    assert!(status.branch.detached);
    assert_eq!(status.branch.name, None);
    assert!(status.branch.head_oid.is_some());
}

#[test]
fn nested_repository_entry_has_no_textual_diff() {
    let dir = init_repo();
    write(dir.path(), "x.txt", "x\n");
    commit_all(dir.path(), "inicial");
    let nested = dir.path().join("aninhado");
    fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init", "-q", "-b", "main"]);
    write(&nested, "dentro.txt", "x\n");

    let status = status_of(&dir);
    let nested_entry = status.files.iter().find(|f| f.path.starts_with("aninhado")).unwrap();
    assert!(nested_entry.untracked);
    if nested_entry.path.ends_with('/') {
        let diff = diff_of(&dir, &nested_entry.path, None, DiffTarget::Untracked);
        assert_eq!(diff.content, DiffContent::Unsupported);
        assert!(diff.notice.is_some());
    }
}

// ----------------------------------------------------------------------------
// Conflitos
// ----------------------------------------------------------------------------

#[test]
fn merge_conflict_is_identified_and_diffable() {
    let dir = init_repo();
    let p = dir.path();
    write(p, "f.txt", "base\n");
    commit_all(p, "base");
    git(p, &["checkout", "-q", "-b", "outra"]);
    write(p, "f.txt", "da outra branch\n");
    commit_all(p, "outra");
    git(p, &["checkout", "-q", "main"]);
    write(p, "f.txt", "da main\n");
    commit_all(p, "main");
    let merge = git_cmd(p, &["merge", "outra"]);
    assert!(!merge.status.success(), "o merge deveria gerar conflito");

    let status = status_of(&dir);
    assert_eq!(status.operation, Some(Operation::Merge));
    let f = entry(&status, "f.txt");
    assert!(f.conflicted);
    assert_eq!(f.conflict_code.as_deref(), Some("UU"));
    assert_eq!(status.summary.conflicted, 1);

    let diff = diff_of(&dir, "f.txt", None, DiffTarget::Conflicted);
    assert_eq!(diff.content, DiffContent::Text);
    let all: Vec<&str> = diff
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .map(|l| l.text.as_str())
        .collect();
    assert!(all.iter().any(|t| t.starts_with("<<<<<<<")), "{all:?}");
    assert!(all.contains(&"da outra branch") && all.contains(&"da main"), "{all:?}");
}

// ----------------------------------------------------------------------------
// Diff
// ----------------------------------------------------------------------------

#[test]
fn unstaged_diff_distinguishes_added_and_removed_lines() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "um\ndois\ntrês\n");
    commit_all(dir.path(), "inicial");
    write(dir.path(), "a.txt", "um\nDOIS\ntrês\n");

    let diff = diff_of(&dir, "a.txt", None, DiffTarget::Unstaged);
    assert_eq!(diff.content, DiffContent::Text);
    assert_eq!((diff.additions, diff.deletions), (1, 1));
    assert!(!diff.is_new && !diff.is_deleted);
    let lines = &diff.hunks[0].lines;
    let removed = lines.iter().find(|l| l.kind == LineKind::Removed).unwrap();
    let added = lines.iter().find(|l| l.kind == LineKind::Added).unwrap();
    assert_eq!((removed.text.as_str(), removed.old_no, removed.new_no), ("dois", Some(2), None));
    assert_eq!((added.text.as_str(), added.old_no, added.new_no), ("DOIS", None, Some(2)));
}

#[test]
fn staged_diff_only_shows_staged_changes() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "um\n");
    commit_all(dir.path(), "inicial");
    write(dir.path(), "a.txt", "um\nstaged\n");
    git(dir.path(), &["add", "a.txt"]);
    write(dir.path(), "a.txt", "um\nstaged\nnão preparado\n");

    let staged = diff_of(&dir, "a.txt", None, DiffTarget::Staged);
    let texts: Vec<_> = staged.hunks[0].lines.iter().map(|l| l.text.as_str()).collect();
    assert!(texts.contains(&"staged") && !texts.contains(&"não preparado"));

    let unstaged = diff_of(&dir, "a.txt", None, DiffTarget::Unstaged);
    assert_eq!((unstaged.additions, unstaged.deletions), (1, 0));
    let added: Vec<_> = unstaged
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .filter(|l| l.kind == LineKind::Added)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(added, vec!["não preparado"]);
}

#[test]
fn deleted_and_new_files_are_flagged() {
    let dir = init_repo();
    write(dir.path(), "velho.txt", "adeus\n");
    commit_all(dir.path(), "inicial");
    fs::remove_file(dir.path().join("velho.txt")).unwrap();
    write(dir.path(), "novo.txt", "oi\nmundo\n");

    let deleted = diff_of(&dir, "velho.txt", None, DiffTarget::Unstaged);
    assert!(deleted.is_deleted && !deleted.is_new);
    assert_eq!((deleted.additions, deleted.deletions), (0, 1));

    let new = diff_of(&dir, "novo.txt", None, DiffTarget::Untracked);
    assert!(new.is_new && !new.is_deleted);
    assert_eq!((new.additions, new.deletions), (2, 0));
}

#[test]
fn renamed_file_diff_reports_origin() {
    let dir = init_repo();
    write(dir.path(), "antigo.txt", "a\nb\nc\nd\ne\n");
    commit_all(dir.path(), "inicial");
    git(dir.path(), &["mv", "antigo.txt", "novo.txt"]);

    let pure = diff_of(&dir, "novo.txt", Some("antigo.txt"), DiffTarget::Staged);
    assert_eq!(pure.renamed_from.as_deref(), Some("antigo.txt"));
    assert_eq!(pure.content, DiffContent::Empty);
    assert!(pure.notice.unwrap().contains("renomeado"));

    write(dir.path(), "novo.txt", "a\nb\nc\nd\nE\n");
    git(dir.path(), &["add", "novo.txt"]);
    let edited = diff_of(&dir, "novo.txt", Some("antigo.txt"), DiffTarget::Staged);
    assert_eq!(edited.renamed_from.as_deref(), Some("antigo.txt"));
    assert_eq!(edited.content, DiffContent::Text);
    assert_eq!((edited.additions, edited.deletions), (1, 1));
}

#[test]
fn binary_files_have_no_textual_diff() {
    let dir = init_repo();
    fs::write(dir.path().join("img.bin"), [0u8, 1, 2, 3, 0, 255]).unwrap();
    commit_all(dir.path(), "inicial");
    fs::write(dir.path().join("img.bin"), [0u8, 9, 9, 9, 0, 255]).unwrap();
    fs::write(dir.path().join("novo.bin"), [0u8, 7, 7]).unwrap();

    for (path, target) in [("img.bin", DiffTarget::Unstaged), ("novo.bin", DiffTarget::Untracked)] {
        let diff = diff_of(&dir, path, None, target);
        assert_eq!(diff.content, DiffContent::Binary, "{path}");
        assert!(diff.hunks.is_empty());
        assert!(diff.notice.is_some());
    }
}

#[test]
fn empty_untracked_file_and_unchanged_file_report_no_differences() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "um\n");
    commit_all(dir.path(), "inicial");
    write(dir.path(), "vazio.txt", "");

    let empty = diff_of(&dir, "vazio.txt", None, DiffTarget::Untracked);
    assert_eq!(empty.content, DiffContent::Empty);
    assert!(empty.notice.is_some());

    let unchanged = diff_of(&dir, "a.txt", None, DiffTarget::Unstaged);
    assert_eq!(unchanged.content, DiffContent::Empty);
    assert!(unchanged.notice.is_some());
}

#[test]
fn very_large_diff_is_truncated_not_failed() {
    let dir = init_repo();
    write(dir.path(), "x.txt", "x\n");
    commit_all(dir.path(), "inicial");
    let big: String = (0..400_000).map(|i| format!("linha número {i}\n")).collect();
    write(dir.path(), "grande.txt", &big);

    let diff = diff_of(&dir, "grande.txt", None, DiffTarget::Untracked);
    assert!(diff.truncated);
    assert_eq!(diff.content, DiffContent::Text);
    assert!(diff.additions > 1000);
}

// ----------------------------------------------------------------------------
// Segurança
// ----------------------------------------------------------------------------

#[test]
fn malicious_or_malformed_diff_paths_are_rejected() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "1\n");
    commit_all(dir.path(), "inicial");
    let git = git_available();
    for bad in ["", "../fora.txt", "a/../../fora", "/etc/passwd", "C:\\Windows\\win.ini", "a\0b"] {
        let request = DiffRequest {
            repo_path: root_of(&dir),
            path: bad.to_string(),
            orig_path: None,
            target: DiffTarget::Unstaged,
        };
        let err = get_file_diff(&git, &request).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad:?}");
    }
    let request = DiffRequest {
        repo_path: root_of(&dir),
        path: "a.txt".into(),
        orig_path: Some("../x".into()),
        target: DiffTarget::Staged,
    };
    assert_eq!(get_file_diff(&git, &request).unwrap_err().code, ErrorCode::InvalidArgument);
}

#[test]
fn repo_path_from_the_ui_must_be_a_real_repository_root() {
    let git = git_available();
    let plain = tempfile::tempdir().unwrap();
    let err = get_status(&git, &root_of(&plain)).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotARepository);
    assert_eq!(get_status(&git, "relativo/qualquer").unwrap_err().code, ErrorCode::InvalidPath);
    assert_eq!(get_status(&git, "").unwrap_err().code, ErrorCode::InvalidPath);
}

#[test]
fn pathspecs_are_literal_no_globs_or_magic() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "1\n");
    write(dir.path(), "b.txt", "1\n");
    commit_all(dir.path(), "inicial");
    write(dir.path(), "a.txt", "2\n");
    write(dir.path(), "b.txt", "2\n");

    for pattern in ["*.txt", ":(top)a.txt", ":!b.txt", "?.txt", "[ab].txt"] {
        let diff = diff_of(&dir, pattern, None, DiffTarget::Unstaged);
        assert_eq!(diff.content, DiffContent::Empty, "{pattern} deveria ser tratado literalmente");
    }
}

#[cfg(unix)]
#[test]
fn repository_config_cannot_execute_commands_through_fsmonitor() {
    use std::os::unix::fs::PermissionsExt;

    let dir = init_repo();
    write(dir.path(), "a.txt", "1\n");
    commit_all(dir.path(), "inicial");

    let outside = tempfile::tempdir().unwrap();
    let marker = outside.path().join("executado.marker");
    let hook = outside.path().join("hook.sh");
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display())).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    git(dir.path(), &["config", "core.fsmonitor", &hook.to_string_lossy()]);
    write(dir.path(), "a.txt", "2\n");

    // Prova de que o teste é significativo: o Git "puro" executa o hook da configuração.
    let _ = git_cmd(dir.path(), &["status", "--porcelain"]);
    let raw_git_runs_hook = marker.exists();
    let _ = fs::remove_file(&marker);

    let status = status_of(&dir);
    assert!(entry(&status, "a.txt").worktree.is_some());
    let _ = diff_of(&dir, "a.txt", None, DiffTarget::Unstaged);
    assert!(
        !marker.exists(),
        "o GitFlow executou o hook de fsmonitor do repositório (git puro executava: {raw_git_runs_hook})"
    );
    println!("git puro executa o hook fsmonitor neste ambiente: {raw_git_runs_hook}");
}

#[cfg(unix)]
#[test]
fn untracked_symlink_preview_does_not_leak_target_content() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "1\n");
    commit_all(dir.path(), "inicial");

    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("segredo.txt");
    fs::write(&secret, "CONTEUDO-SECRETO-FORA-DO-REPOSITORIO\n").unwrap();
    std::os::unix::fs::symlink(&secret, dir.path().join("atalho.txt")).unwrap();

    let status = status_of(&dir);
    assert!(entry(&status, "atalho.txt").untracked);
    let diff = diff_of(&dir, "atalho.txt", None, DiffTarget::Untracked);
    let dump = format!("{:?}", diff.hunks);
    assert!(!dump.contains("CONTEUDO-SECRETO"), "vazou: {dump}");
}

#[cfg(unix)]
#[test]
fn hung_git_is_cancelled_by_timeout() {
    use gitflow_git::runner::RunOptions;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("fake-git");
    fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

    let git = Git::with_program(&fake);
    let opts = RunOptions {
        timeout: Duration::from_millis(300),
        ..RunOptions::default()
    };
    let started = Instant::now();
    let err = git.run(None, &["status"], &opts).unwrap_err();
    assert_eq!(err.code, ErrorCode::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[test]
fn failing_git_reports_exit_code_and_stderr() {
    use gitflow_git::runner::RunOptions;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("fake-git");
    fs::write(&fake, "#!/bin/sh\necho 'fatal: algo deu errado' >&2\nexit 7\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

    let err = Git::with_program(&fake)
        .run(None, &["status"], &RunOptions::default())
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CommandFailed);
    assert!(err.message.contains('7'));
    assert!(err.details.unwrap().contains("algo deu errado"));
}

// ----------------------------------------------------------------------------
// Somente leitura e contrato com o frontend
// ----------------------------------------------------------------------------

#[test]
fn reading_never_modifies_the_repository() {
    let dir = init_repo();
    let p = dir.path();
    write(p, "a.txt", "um\ndois\n");
    write(p, "b.txt", "b\n");
    commit_all(p, "inicial");
    write(p, "a.txt", "um\nDOIS\n");
    git(p, &["add", "a.txt"]);
    write(p, "a.txt", "um\nDOIS\ntrês\n");
    fs::remove_file(p.join("b.txt")).unwrap();
    write(p, "novo.txt", "novo\n");
    // `git status` normalmente "refresca" o índice; garantimos que o nosso não o faz.
    let before = snapshot(p);

    for _ in 0..3 {
        let status = status_of(&dir);
        assert_eq!(status.summary.total, 3);
        let _ = diff_of(&dir, "a.txt", None, DiffTarget::Staged);
        let _ = diff_of(&dir, "a.txt", None, DiffTarget::Unstaged);
        let _ = diff_of(&dir, "b.txt", None, DiffTarget::Unstaged);
        let _ = diff_of(&dir, "novo.txt", None, DiffTarget::Untracked);
    }

    assert_eq!(before, snapshot(p), "a leitura alterou arquivos do repositório");
    assert!(!p.join(".git/index.lock").exists());
}

#[test]
fn missing_repository_folder_after_selection_is_reported() {
    let dir = init_repo();
    let path = root_of(&dir);
    drop(dir); // a pasta é apagada
    let err = get_status(&git_available(), &path).unwrap_err();
    assert_eq!(err.code, ErrorCode::PathNotFound);
}

#[test]
fn json_contract_matches_what_the_frontend_expects() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "1\n");
    commit_all(dir.path(), "inicial");
    write(dir.path(), "a.txt", "2\n");
    write(dir.path(), "u.txt", "u\n");

    let status = serde_json::to_value(status_of(&dir)).unwrap();
    assert!(status["repoPath"].is_string());
    assert_eq!(status["isClean"], false);
    assert_eq!(status["branch"]["name"], "main");
    assert_eq!(status["branch"]["detached"], false);
    assert_eq!(status["summary"]["untracked"], 1);
    let a = status["files"].as_array().unwrap().iter().find(|f| f["path"] == "a.txt").unwrap();
    assert_eq!(a["worktree"], "modified");
    assert!(a["index"].is_null());
    assert_eq!(a["untracked"], false);

    let diff = serde_json::to_value(diff_of(&dir, "a.txt", None, DiffTarget::Unstaged)).unwrap();
    assert_eq!(diff["content"], "text");
    assert_eq!(diff["target"], "unstaged");
    assert_eq!(diff["hunks"][0]["lines"][0]["kind"], "removed");
    assert!(diff["hunks"][0]["lines"][0]["oldNo"].is_number());

    let request: DiffRequest = serde_json::from_str(
        r#"{"repoPath":"/x","path":"a.txt","origPath":null,"target":"conflicted"}"#,
    )
    .unwrap();
    assert_eq!(request.target, DiffTarget::Conflicted);

    let err = get_status(&git_available(), "").unwrap_err();
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json["code"], "INVALID_PATH");
    assert!(json["message"].is_string());
}
