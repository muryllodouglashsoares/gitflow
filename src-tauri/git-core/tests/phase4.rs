//! Testes de integração da Fase 4: inspeção de ZIP, plano, branch de trabalho, importação,
//! reversão, commit isolado e push.
//!
//! Usam o Git real contra repositórios temporários e **remotos locais** (repositórios bare em
//! pastas temporárias): não dependem de internet, de credenciais nem de repositórios reais.
//! Os ZIPs são montados em memória com o crate `zip`; os casos adulterados (duplicatas, CRC)
//! são produzidos alterando bytes do arquivo gerado.

use gitflow_git::{
    apply_import, create_import_commit, find_import_operation, finish_import, get_status,
    import::{apply_import_with, inspect_zip_with, ApplyOptions, ImportLimits},
    inspect_zip,
    lock::RepoGuard,
    open_repository, prepare_commit_review, prepare_import_branch, push_import_branch,
    rollback_import, ApplyImportRequest, EntryAction, ErrorCode, Git, ImportCommitRequest,
    ImportOperationRequest, ImportPlan, ImportResult, ImportState, InspectRequest,
    PrepareBranchRequest, PushImportRequest, ReviewRequest, SyncTarget,
};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::Once,
};
use tempfile::TempDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

// ----------------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------------

fn isolate_git_config() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        );
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

fn run_git(dir: &Path, args: &[&str]) {
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
    assert!(
        out.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn configure_identity(dir: &Path) {
    run_git(dir, &["config", "user.name", "Teste"]);
    run_git(dir, &["config", "user.email", "teste@example.com"]);
    run_git(dir, &["config", "commit.gpgsign", "false"]);
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
    run_git(dir, &["add", "-A"]);
    run_git(dir, &["commit", "-q", "-m", message]);
}

struct Env {
    git: Git,
    repo: TempDir,
    zips: TempDir,
    path: String,
}

/// Repositório com a branch `main` limpa, contendo `base.txt`, `src/app.txt` e `keep.txt`.
fn env() -> Env {
    isolate_git_config();
    let git = Git::locate().expect("git precisa estar instalado para os testes");
    let repo = tempfile::Builder::new()
        .prefix("gitflow-fase4-")
        .tempdir()
        .unwrap();
    run_git(repo.path(), &["init", "-q", "-b", "main"]);
    configure_identity(repo.path());
    write(repo.path(), "base.txt", "base\n");
    write(repo.path(), "src/app.txt", "app v1\n");
    write(repo.path(), "keep.txt", "fica no repositório\n");
    commit_all(repo.path(), "inicial");
    let path = open_repository(&git, repo.path().to_str().unwrap())
        .unwrap()
        .path;
    let zips = tempfile::Builder::new()
        .prefix("gitflow-fase4-zips-")
        .tempdir()
        .unwrap();
    Env {
        git,
        repo,
        zips,
        path,
    }
}

fn opts(method: CompressionMethod) -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(method)
}

fn zip_bytes_to(
    dir: &Path,
    name: &str,
    entries: &[(&str, &[u8])],
    method: CompressionMethod,
) -> String {
    let path = dir.join(name);
    let mut writer = ZipWriter::new(File::create(&path).unwrap());
    for (entry, content) in entries {
        if entry.ends_with('/') {
            writer.add_directory(*entry, opts(method)).unwrap();
        } else {
            writer.start_file(*entry, opts(method)).unwrap();
            writer.write_all(content).unwrap();
        }
    }
    writer.finish().unwrap();
    path.to_string_lossy().into_owned()
}

fn make_zip(env: &Env, name: &str, entries: &[(&str, &str)]) -> String {
    let owned: Vec<(&str, &[u8])> = entries.iter().map(|(n, c)| (*n, c.as_bytes())).collect();
    zip_bytes_to(env.zips.path(), name, &owned, CompressionMethod::Deflated)
}

fn replace_bytes(path: &str, from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let mut data = fs::read(path).unwrap();
    let mut count = 0;
    let mut i = 0;
    while i + from.len() <= data.len() {
        if &data[i..i + from.len()] == from {
            data[i..i + from.len()].copy_from_slice(to);
            i += from.len();
            count += 1;
        } else {
            i += 1;
        }
    }
    assert!(count > 0, "padrão não encontrado no ZIP de teste");
    fs::write(path, data).unwrap();
}

fn inspect_req(env: &Env, zip: &str, prefix: &str) -> InspectRequest {
    InspectRequest {
        repo_path: env.path.clone(),
        zip_path: zip.to_string(),
        source_prefix: prefix.to_string(),
        base_branch: "main".into(),
    }
}

fn plan_of(env: &Env, zip: &str) -> ImportPlan {
    inspect_zip(&env.git, &inspect_req(env, zip, "")).expect("inspeção deveria funcionar")
}

fn entry<'a>(plan: &'a ImportPlan, path: &str) -> &'a gitflow_git::PlanEntry {
    plan.entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| {
            panic!(
                "entrada {path} ausente do plano: {:?}",
                plan.entries.iter().map(|e| &e.path).collect::<Vec<_>>()
            )
        })
}

fn prepare_req(env: &Env, zip: &str, plan: &ImportPlan, work: &str) -> PrepareBranchRequest {
    PrepareBranchRequest {
        repo_path: env.path.clone(),
        zip_path: zip.to_string(),
        source_prefix: plan.source_prefix.clone(),
        base_branch: "main".into(),
        work_branch: work.into(),
        plan_id: plan.plan_id.clone(),
    }
}

fn apply_req(env: &Env, zip: &str, plan: &ImportPlan, work: &str) -> ApplyImportRequest {
    ApplyImportRequest {
        repo_path: env.path.clone(),
        zip_path: zip.to_string(),
        source_prefix: plan.source_prefix.clone(),
        base_branch: "main".into(),
        work_branch: work.into(),
        plan_id: plan.plan_id.clone(),
        excluded_paths: vec![],
        confirm_overwrites: true,
    }
}

/// Plano → branch de trabalho → importação (com confirmação de sobrescrita).
fn import_all(env: &Env, zip: &str, work: &str) -> (ImportPlan, ImportResult) {
    let plan = plan_of(env, zip);
    assert!(
        plan.can_apply,
        "plano deveria ser aplicável: {:?} {:?}",
        plan.blockers, plan.entries
    );
    prepare_import_branch(&env.git, &prepare_req(env, zip, &plan, work)).unwrap();
    let result = apply_import(&env.git, &apply_req(env, zip, &plan, work)).unwrap();
    (plan, result)
}

fn current_branch(env: &Env) -> String {
    git_out(env.repo.path(), &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn commit_count(env: &Env) -> usize {
    git_out(env.repo.path(), &["rev-list", "--count", "--all"])
        .parse()
        .unwrap()
}

fn commit_import(
    env: &Env,
    op_id: &str,
    files: &[&str],
    work: &str,
) -> gitflow_git::Result<gitflow_git::CommitResult> {
    let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    let review = prepare_commit_review(
        &env.git,
        &ReviewRequest {
            repo_path: env.path.clone(),
            files: files.clone(),
        },
    )?;
    create_import_commit(
        &env.git,
        &ImportCommitRequest {
            op_id: op_id.into(),
            repo_path: env.path.clone(),
            files,
            message: "Importa atualização do ZIP".into(),
            expected_branch: work.into(),
            fingerprint: review.fingerprint,
        },
    )
}

fn op_req(env: &Env, op_id: &str) -> ImportOperationRequest {
    ImportOperationRequest {
        repo_path: env.path.clone(),
        op_id: op_id.into(),
    }
}

fn blocked_paths(plan: &ImportPlan) -> Vec<&str> {
    plan.entries
        .iter()
        .filter(|e| e.action == EntryAction::Blocked)
        .map(|e| e.path.as_str())
        .collect()
}

fn no_files_written(env: &Env, paths: &[&str]) {
    for p in paths {
        assert!(!env.repo.path().join(p).exists(), "{p} não deveria existir");
    }
}

// ----------------------------------------------------------------------------
// Inspeção
// ----------------------------------------------------------------------------

#[test]
fn inspection_classifies_new_replaced_and_identical_files_without_extracting_anything() {
    let env = env();
    let zip = make_zip(
        &env,
        "pacote.zip",
        &[
            ("novo.txt", "novo\n"),
            ("src/app.txt", "app v2\n"),
            ("base.txt", "base\n"),
            ("docs/Relatório final.md", "acentos e espaços\n"),
            (".gitignore", "target/\n"),
            (".github/workflows/ci.yml", "name: ci\n"),
        ],
    );
    let plan = plan_of(&env, &zip);

    assert_eq!(entry(&plan, "novo.txt").action, EntryAction::Add);
    assert_eq!(entry(&plan, "src/app.txt").action, EntryAction::Replace);
    assert_eq!(entry(&plan, "base.txt").action, EntryAction::Identical);
    assert_eq!(
        entry(&plan, "docs/Relatório final.md").action,
        EntryAction::Add
    );
    // Arquivos ocultos legítimos não são descartados.
    assert_eq!(entry(&plan, ".gitignore").action, EntryAction::Add);
    assert_eq!(
        entry(&plan, ".github/workflows/ci.yml").action,
        EntryAction::Add
    );
    assert_eq!(plan.summary.add, 4);
    assert_eq!(plan.summary.replace, 1);
    assert_eq!(plan.summary.identical, 1);
    assert_eq!(plan.summary.blocked, 0);
    assert!(plan.can_apply, "{:?}", plan.blockers);
    assert!(!plan.plan_id.is_empty());
    assert_eq!(plan.base_branch, "main");

    // A inspeção não extrai nada.
    no_files_written(&env, &["novo.txt", "docs", ".github", ".gitignore"]);
    assert_eq!(read(env.repo.path(), "src/app.txt"), "app v1\n");
    assert!(get_status(&env.git, &env.path).unwrap().files.is_empty());
}

#[test]
fn zips_that_are_corrupt_empty_or_not_zips_are_rejected() {
    let env = env();

    let good = make_zip(
        &env,
        "ok.zip",
        &[("a.txt", "conteúdo suficiente para truncar\n")],
    );
    let bytes = fs::read(&good).unwrap();
    let truncated = env.zips.path().join("truncado.zip");
    fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    let err = inspect_zip(
        &env.git,
        &inspect_req(&env, truncated.to_str().unwrap(), ""),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArchive);

    let fake = env.zips.path().join("falso.zip");
    fs::write(&fake, "isto é só texto, não um ZIP").unwrap();
    let err = inspect_zip(&env.git, &inspect_req(&env, fake.to_str().unwrap(), "")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArchive);

    let zero = env.zips.path().join("zero.zip");
    fs::write(&zero, "").unwrap();
    let err = inspect_zip(&env.git, &inspect_req(&env, zero.to_str().unwrap(), "")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArchive);

    // ZIP válido, porém sem nenhuma entrada.
    let empty = env.zips.path().join("vazio.zip");
    ZipWriter::new(File::create(&empty).unwrap())
        .finish()
        .unwrap();
    let err = inspect_zip(&env.git, &inspect_req(&env, empty.to_str().unwrap(), "")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArchive);

    // Extensão errada e arquivo inexistente.
    let txt = env.zips.path().join("pacote.txt");
    fs::write(&txt, "x").unwrap();
    assert_eq!(
        inspect_zip(&env.git, &inspect_req(&env, txt.to_str().unwrap(), ""))
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    let missing = env.zips.path().join("nao-existe.zip");
    assert_eq!(
        inspect_zip(&env.git, &inspect_req(&env, missing.to_str().unwrap(), ""))
            .unwrap_err()
            .code,
        ErrorCode::PathNotFound
    );
    // Caminho relativo nunca é aceito.
    assert_eq!(
        inspect_zip(&env.git, &inspect_req(&env, "pacote.zip", ""))
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
}

#[test]
fn corrupted_entry_content_is_detected_and_blocks_the_plan() {
    let env = env();
    let path = zip_bytes_to(
        env.zips.path(),
        "crc.zip",
        &[("a.txt", b"AAAAAAAAAAAAAAAA"), ("b.txt", b"ok")],
        CompressionMethod::Stored,
    );
    replace_bytes(&path, b"AAAAAAAAAAAAAAAA", b"BBBBBBBBBBBBBBBB");
    let plan = plan_of(&env, &path);
    assert!(!plan.can_apply);
    assert!(
        blocked_paths(&plan).contains(&"a.txt"),
        "{:?}",
        plan.entries
    );
}

#[test]
fn duplicate_entries_are_blocked() {
    let env = env();
    let path = zip_bytes_to(
        env.zips.path(),
        "dup.zip",
        &[("a.txt", b"um"), ("b.txt", b"dois")],
        CompressionMethod::Stored,
    );
    replace_bytes(&path, b"b.txt", b"a.txt");
    let plan = plan_of(&env, &path);
    assert!(!plan.can_apply);
    assert!(plan.summary.blocked >= 1);
    assert!(
        plan.entries.iter().any(|e| e.action == EntryAction::Blocked
            && e.reason
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("duplicad")),
        "{:?}",
        plan.entries
    );
}

#[test]
fn forged_zip64_markers_are_rejected_instead_of_hiding_entries() {
    let env = env();
    let path = zip_bytes_to(
        env.zips.path(),
        "zip64.zip",
        &[("a.txt", b"um")],
        CompressionMethod::Stored,
    );
    // Marca o total de entradas do registro de fim como 0xFFFF ("veja o registro ZIP64").
    let mut data = fs::read(&path).unwrap();
    let eocd = data
        .windows(4)
        .rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])
        .expect("registro de fim do diretório central");
    data[eocd + 10] = 0xFF;
    data[eocd + 11] = 0xFF;
    fs::write(&path, data).unwrap();
    let err = inspect_zip(&env.git, &inspect_req(&env, &path, "")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArchive);
    assert!(err.message.contains("ZIP64"), "{}", err.message);
}

#[test]
fn dangerous_paths_are_blocked_and_nothing_is_written_outside_the_repository() {
    let env = env();
    let parent = env.repo.path().parent().unwrap().to_path_buf();
    let zip = zip_bytes_to(
        env.zips.path(),
        "perigoso.zip",
        &[
            ("ok.txt", b"ok"),
            ("../fora.txt", b"x"),
            ("a/../../fora2.txt", b"x"),
            ("/etc/absoluto.txt", b"x"),
            ("C:/Windows/x.txt", b"x"),
            ("dir\\barra.txt", b"x"),
            ("src/CON.txt", b"x"),
            ("src/a:b.txt", b"x"),
            ("src/ponto.", b"x"),
            (".git/config", b"x"),
            ("x/.GIT/hooks/pre-commit", b"x"),
        ],
        CompressionMethod::Deflated,
    );
    let plan = plan_of(&env, &zip);
    assert!(!plan.can_apply);
    assert_eq!(plan.summary.blocked, 10, "{:?}", blocked_paths(&plan));
    assert_eq!(entry(&plan, "ok.txt").action, EntryAction::Add);

    // Mesmo tentando importar, o backend recusa e nada é gravado.
    let err = prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/x"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(current_branch(&env), "main");
    assert!(!parent.join("fora.txt").exists());
    assert!(!parent.join("fora2.txt").exists());
    assert!(!env.repo.path().join("ok.txt").exists());
}

#[test]
fn windows_name_collisions_are_blocked() {
    let env = env();
    let zip = zip_bytes_to(
        env.zips.path(),
        "colisao.zip",
        &[
            ("Leia.md", b"1"),
            ("leia.MD", b"2"),
            ("caf\u{e9}.txt", b"3"),
            ("cafe\u{301}.txt", b"4"),
            ("pasta", b"arquivo"),
            ("pasta/dentro.txt", b"5"),
            ("normal.txt", b"6"),
        ],
        CompressionMethod::Deflated,
    );
    let plan = plan_of(&env, &zip);
    assert!(!plan.can_apply);
    assert_eq!(entry(&plan, "normal.txt").action, EntryAction::Add);
    let blocked = blocked_paths(&plan);
    for expected in ["Leia.md", "leia.MD", "café.txt", "cafe\u{301}.txt", "pasta"] {
        assert!(
            blocked.contains(&expected),
            "{expected:?} deveria estar bloqueado: {blocked:?}"
        );
    }
}

#[test]
fn symlink_entries_are_blocked_not_materialized() {
    let env = env();
    let path = env.zips.path().join("link.zip");
    let mut writer = ZipWriter::new(File::create(&path).unwrap());
    writer
        .start_file("normal.txt", opts(CompressionMethod::Deflated))
        .unwrap();
    writer.write_all(b"ok").unwrap();
    writer
        .add_symlink("atalho", "/etc/passwd", opts(CompressionMethod::Stored))
        .unwrap();
    writer.finish().unwrap();
    let plan = plan_of(&env, path.to_str().unwrap());
    assert!(!plan.can_apply);
    assert_eq!(entry(&plan, "atalho").action, EntryAction::Blocked);
    assert!(entry(&plan, "atalho")
        .reason
        .as_deref()
        .unwrap_or("")
        .to_lowercase()
        .contains("simbólico"));
}

#[test]
fn size_entry_count_and_compression_ratio_limits_are_enforced() {
    let env = env();
    let big = vec![b'a'; 20_000];
    let zip = zip_bytes_to(
        env.zips.path(),
        "limites.zip",
        &[("pequeno.txt", b"ok"), ("grande.txt", &big)],
        CompressionMethod::Deflated,
    );
    let req = inspect_req(&env, &zip, "");

    // Arquivo maior que o limite por arquivo.
    let limits = ImportLimits {
        max_file_bytes: 10_000,
        ..ImportLimits::default()
    };
    let plan = inspect_zip_with(&env.git, &req, &limits).unwrap();
    assert_eq!(entry(&plan, "grande.txt").action, EntryAction::Blocked);
    assert!(!plan.can_apply);

    // Total descomprimido acima do limite: o ZIP é recusado antes de qualquer plano.
    let limits = ImportLimits {
        max_total_bytes: 10_000,
        ..ImportLimits::default()
    };
    assert_eq!(
        inspect_zip_with(&env.git, &req, &limits).unwrap_err().code,
        ErrorCode::InvalidArchive
    );

    // Muitas entradas.
    let limits = ImportLimits {
        max_entries: 1,
        ..ImportLimits::default()
    };
    assert_eq!(
        inspect_zip_with(&env.git, &req, &limits).unwrap_err().code,
        ErrorCode::InvalidArchive
    );

    // ZIP maior que o limite em disco.
    let limits = ImportLimits {
        max_zip_bytes: 10,
        ..ImportLimits::default()
    };
    assert_eq!(
        inspect_zip_with(&env.git, &req, &limits).unwrap_err().code,
        ErrorCode::InvalidArchive
    );

    // Alta taxa de compressão (20 KB de 'a' comprimem para poucas dezenas de bytes).
    let limits = ImportLimits {
        max_ratio: 5,
        ratio_min_bytes: 1_000,
        ..ImportLimits::default()
    };
    let plan = inspect_zip_with(&env.git, &req, &limits).unwrap();
    assert_eq!(entry(&plan, "grande.txt").action, EntryAction::Blocked);
    assert_eq!(entry(&plan, "pequeno.txt").action, EntryAction::Add);
}

#[test]
fn wrapper_folder_is_suggested_but_never_removed_silently() {
    let env = env();
    let zip = make_zip(
        &env,
        "wrapper.zip",
        &[
            ("projeto/src/app.txt", "app v2\n"),
            ("projeto/novo.txt", "n\n"),
            ("projeto/.env.example", "A=1\n"),
        ],
    );
    let plan = plan_of(&env, &zip);
    // Sem escolher o prefixo, os arquivos entram com o caminho completo (dentro de projeto/).
    assert_eq!(plan.source_prefix, "");
    assert_eq!(entry(&plan, "projeto/novo.txt").action, EntryAction::Add);
    assert_eq!(plan.layout.suggested_prefix.as_deref(), Some("projeto"));
    assert!(plan.layout.directories.iter().any(|d| d == "projeto"));
    assert!(
        plan.warnings.iter().any(|w| w.contains("projeto")),
        "{:?}",
        plan.warnings
    );

    // Com o prefixo escolhido pelo usuário, o nível é removido de forma explícita.
    let plan = inspect_zip(&env.git, &inspect_req(&env, &zip, "projeto")).unwrap();
    assert_eq!(plan.source_prefix, "projeto");
    assert_eq!(entry(&plan, "src/app.txt").action, EntryAction::Replace);
    assert_eq!(entry(&plan, "novo.txt").action, EntryAction::Add);
    assert_eq!(entry(&plan, ".env.example").action, EntryAction::Add);

    // Prefixo malicioso é recusado.
    assert!(inspect_zip(&env.git, &inspect_req(&env, &zip, "../x")).is_err());
}

#[test]
fn files_outside_the_selected_folder_are_ignored() {
    let env = env();
    let zip = make_zip(
        &env,
        "escopo.zip",
        &[
            ("projeto/novo.txt", "n\n"),
            ("outro/lixo.txt", "l\n"),
            ("raiz.txt", "r\n"),
        ],
    );
    let plan = inspect_zip(&env.git, &inspect_req(&env, &zip, "projeto/")).unwrap();
    assert_eq!(plan.summary.add, 1);
    assert!(plan.summary.ignored >= 2);
    assert!(plan.entries.iter().all(|e| e.path != "lixo.txt"));
}

#[test]
fn files_ignored_by_gitignore_are_blocked_because_git_would_not_see_them() {
    let env = env();
    write(env.repo.path(), ".gitignore", "*.log\n");
    commit_all(env.repo.path(), "gitignore");
    let zip = make_zip(&env, "ign.zip", &[("debug.log", "x\n"), ("ok.txt", "x\n")]);
    let plan = plan_of(&env, &zip);
    assert_eq!(entry(&plan, "debug.log").action, EntryAction::Blocked);
    assert!(!plan.can_apply);
    assert_eq!(entry(&plan, "ok.txt").action, EntryAction::Add);
}

#[cfg(unix)]
#[test]
fn destination_that_crosses_a_symlink_is_blocked() {
    let env = env();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), env.repo.path().join("atalho")).unwrap();
    commit_all(env.repo.path(), "link");
    let zip = make_zip(
        &env,
        "viaLink.zip",
        &[("atalho/x.txt", "x\n"), ("ok.txt", "x\n")],
    );
    let plan = plan_of(&env, &zip);
    assert_eq!(entry(&plan, "atalho/x.txt").action, EntryAction::Blocked);
    assert!(!plan.can_apply);
    assert!(!outside.path().join("x.txt").exists());
}

// ----------------------------------------------------------------------------
// Estado do repositório
// ----------------------------------------------------------------------------

#[test]
fn local_changes_block_the_plan_and_the_branch_creation_and_are_never_touched() {
    let env = env();
    write(env.repo.path(), "keep.txt", "edição local não commitada\n");
    write(env.repo.path(), "rascunho.txt", "não rastreado\n");
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);
    assert!(!plan.can_apply);
    assert!(!plan.blockers.is_empty());

    let err = prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/a"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(current_branch(&env), "main");
    assert_eq!(
        read(env.repo.path(), "keep.txt"),
        "edição local não commitada\n"
    );
    assert_eq!(read(env.repo.path(), "rascunho.txt"), "não rastreado\n");
    assert!(git_out(env.repo.path(), &["branch", "--list", "importacao/a"]).is_empty());
}

#[test]
fn base_must_exist_be_current_and_the_repository_valid() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);

    let mut req = inspect_req(&env, &zip, "");
    req.base_branch = "inexistente".into();
    assert!(inspect_zip(&env.git, &req).is_err());

    // Base existente, mas diferente da branch atual: o plano é bloqueado (é preciso trocar antes).
    run_git(env.repo.path(), &["branch", "outra"]);
    let mut req = inspect_req(&env, &zip, "");
    req.base_branch = "outra".into();
    let plan = inspect_zip(&env.git, &req).unwrap();
    assert!(!plan.can_apply);
    assert!(
        plan.blockers
            .iter()
            .any(|b| b.contains("main") && b.contains("outra")),
        "{:?}",
        plan.blockers
    );

    // Pasta que não é repositório.
    let not_repo = tempfile::tempdir().unwrap();
    let mut req = inspect_req(&env, &zip, "");
    req.repo_path = not_repo.path().to_string_lossy().into_owned();
    assert_eq!(
        inspect_zip(&env.git, &req).unwrap_err().code,
        ErrorCode::NotARepository
    );
}

#[test]
fn merge_in_progress_with_conflicts_blocks_the_plan() {
    let env = env();
    run_git(env.repo.path(), &["checkout", "-q", "-b", "a"]);
    write(env.repo.path(), "base.txt", "linha A\n");
    commit_all(env.repo.path(), "a");
    run_git(env.repo.path(), &["checkout", "-q", "main"]);
    write(env.repo.path(), "base.txt", "linha B\n");
    commit_all(env.repo.path(), "b");
    let merge = git_cmd(env.repo.path(), &["merge", "a"]);
    assert!(!merge.status.success(), "o merge deveria gerar conflito");
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);
    assert!(!plan.can_apply);
    assert!(!plan.blockers.is_empty());
    let err = prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/m"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
}

// ----------------------------------------------------------------------------
// Branch de trabalho
// ----------------------------------------------------------------------------

#[test]
fn work_branch_is_created_from_the_base_and_checked_out_explicitly() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);
    let prepared =
        prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/ok")).unwrap();
    assert_eq!(prepared.work_branch, "importacao/ok");
    assert_eq!(current_branch(&env), "importacao/ok");
    assert_eq!(
        git_out(env.repo.path(), &["rev-parse", "importacao/ok"]),
        git_out(env.repo.path(), &["rev-parse", "main"])
    );
    // Nenhum arquivo foi importado ainda.
    no_files_written(&env, &["novo.txt"]);
}

#[test]
fn invalid_existing_or_equal_branch_names_are_refused_before_anything_changes() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);

    for bad in ["nome inválido", "a..b", "-x", "feature.lock", "x~1", ""] {
        let err =
            prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, bad)).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad:?}");
    }
    run_git(env.repo.path(), &["branch", "ja-existe"]);
    let err =
        prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "ja-existe")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    let err = prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "main")).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(current_branch(&env), "main");
}

#[test]
fn a_stale_plan_is_refused_before_creating_the_branch() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[("novo.txt", "n\n"), ("src/app.txt", "app v2\n")],
    );
    let plan = plan_of(&env, &zip);

    // O destino mudou depois da prévia (arquivo que seria substituído foi alterado e commitado).
    write(env.repo.path(), "src/app.txt", "app v1.5\n");
    commit_all(env.repo.path(), "mudança depois da prévia");
    let err = prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/s"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    assert!(git_out(env.repo.path(), &["branch", "--list", "importacao/s"]).is_empty());
}

#[test]
fn a_zip_changed_after_inspection_is_refused() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "versão 1\n")]);
    let plan = plan_of(&env, &zip);
    // Mesmo caminho, conteúdo diferente.
    make_zip(&env, "p.zip", &[("novo.txt", "versão 2 (trocada)\n")]);
    let err = prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/z"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    assert_eq!(current_branch(&env), "main");
}

// ----------------------------------------------------------------------------
// Importação
// ----------------------------------------------------------------------------

#[test]
fn import_writes_exactly_the_confirmed_plan_and_never_deletes_missing_files() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[
            ("novo.txt", "novo\n"),
            ("fundo/a/b/profundo.txt", "profundo\n"),
            ("src/app.txt", "app v2\n"),
            ("base.txt", "base\n"),
        ],
    );
    let (plan, result) = import_all(&env, &zip, "importacao/ok");
    assert_eq!(result.state, ImportState::Applied);
    assert!(result.failure.is_none());
    assert_eq!(result.identical_skipped, 1);
    let mut applied: Vec<&str> = result.applied.iter().map(|f| f.path.as_str()).collect();
    applied.sort();
    assert_eq!(
        applied,
        vec!["fundo/a/b/profundo.txt", "novo.txt", "src/app.txt"]
    );
    assert_eq!(plan.summary.add + plan.summary.replace, applied.len());
    assert!(result.applied.iter().all(|f| f.visible_in_git));

    assert_eq!(read(env.repo.path(), "novo.txt"), "novo\n");
    assert_eq!(
        read(env.repo.path(), "fundo/a/b/profundo.txt"),
        "profundo\n"
    );
    assert_eq!(read(env.repo.path(), "src/app.txt"), "app v2\n");
    // Ausente do ZIP ≠ excluir.
    assert_eq!(read(env.repo.path(), "keep.txt"), "fica no repositório\n");
    // O Git enxerga exatamente as alterações reais (o idêntico não aparece).
    let status = get_status(&env.git, &env.path).unwrap();
    let mut changed: Vec<String> = status.files.iter().map(|f| f.path.clone()).collect();
    changed.sort();
    assert_eq!(
        changed,
        vec!["fundo/a/b/profundo.txt", "novo.txt", "src/app.txt"]
    );
    // Nada de commit nem de arquivos temporários no repositório.
    assert_eq!(commit_count(&env), 1);
    assert!(!changed.iter().any(|p| p.contains(".gitflow-import")));
}

#[test]
fn overwrite_needs_explicit_confirmation_and_unconfirmed_apply_writes_nothing() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[("src/app.txt", "app v2\n"), ("novo.txt", "n\n")],
    );
    let plan = plan_of(&env, &zip);
    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/c")).unwrap();
    let mut req = apply_req(&env, &zip, &plan, "importacao/c");
    req.confirm_overwrites = false;
    let err = apply_import(&env.git, &req).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(read(env.repo.path(), "src/app.txt"), "app v1\n");
    no_files_written(&env, &["novo.txt"]);
    // Com a confirmação, funciona.
    req.confirm_overwrites = true;
    assert_eq!(
        apply_import(&env.git, &req).unwrap().state,
        ImportState::Applied
    );
}

#[test]
fn excluded_files_are_not_imported_and_unknown_exclusions_are_refused() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[
            ("src/app.txt", "app v2\n"),
            ("novo.txt", "n\n"),
            ("outro.txt", "o\n"),
        ],
    );
    let plan = plan_of(&env, &zip);
    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/e")).unwrap();

    let mut req = apply_req(&env, &zip, &plan, "importacao/e");
    req.excluded_paths = vec!["nao-esta-no-plano.txt".into()];
    assert_eq!(
        apply_import(&env.git, &req).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    no_files_written(&env, &["novo.txt", "outro.txt"]);

    req.excluded_paths = vec!["src/app.txt".into(), "outro.txt".into()];
    let result = apply_import(&env.git, &req).unwrap();
    assert_eq!(result.state, ImportState::Applied);
    assert_eq!(result.applied.len(), 1);
    assert_eq!(read(env.repo.path(), "novo.txt"), "n\n");
    assert_eq!(read(env.repo.path(), "src/app.txt"), "app v1\n");
    no_files_written(&env, &["outro.txt"]);
}

#[test]
fn apply_requires_the_work_branch_and_a_matching_plan() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);

    // Chamada direta, sem preparar a branch de trabalho: recusada (sem pular etapas).
    let err = apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/x")).unwrap_err();
    assert!(
        matches!(
            err.code,
            ErrorCode::OperationBlocked | ErrorCode::StaleReview | ErrorCode::InvalidArgument
        ),
        "{err:?}"
    );
    no_files_written(&env, &["novo.txt"]);

    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/x")).unwrap();
    // plan_id adulterado.
    let mut req = apply_req(&env, &zip, &plan, "importacao/x");
    req.plan_id = "0".repeat(64);
    assert_eq!(
        apply_import(&env.git, &req).unwrap_err().code,
        ErrorCode::StaleReview
    );
    no_files_written(&env, &["novo.txt"]);

    // O destino mudou depois da confirmação: recusado.
    write(env.repo.path(), "novo.txt", "apareceu depois\n");
    let err = apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/x")).unwrap_err();
    assert!(
        matches!(
            err.code,
            ErrorCode::StaleReview | ErrorCode::OperationBlocked
        ),
        "{err:?}"
    );
    assert_eq!(read(env.repo.path(), "novo.txt"), "apareceu depois\n");
}

#[test]
fn apply_never_overwrites_a_file_that_changed_after_the_preview() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("src/app.txt", "app v2\n")]);
    let plan = plan_of(&env, &zip);
    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/m")).unwrap();
    write(env.repo.path(), "src/app.txt", "editado por fora\n");
    let err = apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/m")).unwrap_err();
    assert!(
        matches!(
            err.code,
            ErrorCode::StaleReview | ErrorCode::OperationBlocked
        ),
        "{err:?}"
    );
    assert_eq!(read(env.repo.path(), "src/app.txt"), "editado por fora\n");
}

#[test]
fn concurrent_operation_blocks_the_import_and_the_lock_is_released_after_failures() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);
    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/l")).unwrap();
    let root = Path::new(&env.path).to_path_buf();
    {
        let _guard = RepoGuard::acquire(&root).unwrap();
        let err =
            apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/l")).unwrap_err();
        assert_eq!(err.code, ErrorCode::RepositoryBusy);
    }
    // Falha de validação não deixa o repositório travado: a nova tentativa funciona.
    let mut bad = apply_req(&env, &zip, &plan, "importacao/l");
    bad.plan_id = "x".into();
    assert!(apply_import(&env.git, &bad).is_err());
    assert_eq!(
        apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/l"))
            .unwrap()
            .state,
        ImportState::Applied
    );
}

// ----------------------------------------------------------------------------
// Falha parcial e reversão
// ----------------------------------------------------------------------------

fn partial_import(env: &Env, zip: &str, work: &str, fail_after: usize) -> ImportResult {
    let plan = plan_of(env, zip);
    prepare_import_branch(&env.git, &prepare_req(env, zip, &plan, work)).unwrap();
    let options = ApplyOptions {
        fail_after_files: Some(fail_after),
        ..ApplyOptions::default()
    };
    apply_import_with(&env.git, &apply_req(env, zip, &plan, work), &options).unwrap()
}

#[test]
fn a_write_failure_midway_is_reported_as_partial_and_does_not_continue() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[
            ("a1.txt", "1\n"),
            ("a2.txt", "2\n"),
            ("a3.txt", "3\n"),
            ("src/app.txt", "app v2\n"),
        ],
    );
    let result = partial_import(&env, &zip, "importacao/p", 2);
    assert_eq!(result.state, ImportState::Partial);
    assert!(result.failure.is_some());
    assert_eq!(result.applied.len(), 2);
    // 4 arquivos: 2 gravados, 1 falhou (`failure`) e 1 nem foi tentado.
    assert_eq!(result.not_attempted.len(), 1);
    assert!(result
        .failure
        .as_ref()
        .is_some_and(|f| !f.path.is_empty() && !f.message.is_empty()));
    assert!(result.recovery.is_some());

    // A operação parcial é encontrada para retomada e não pode ser commitada nem encerrada.
    let op = find_import_operation(&env.git, &env.path)
        .unwrap()
        .expect("operação pendente");
    assert_eq!(op.op_id, result.op_id);
    assert_eq!(op.state, ImportState::Partial);
    let applied: Vec<&str> = result.applied.iter().map(|f| f.path.as_str()).collect();
    let err = commit_import(&env, &result.op_id, &applied, "importacao/p").unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(
        finish_import(&env.git, &op_req(&env, &result.op_id))
            .unwrap_err()
            .code,
        ErrorCode::OperationBlocked
    );
    assert_eq!(commit_count(&env), 1);
}

#[test]
fn rollback_restores_replaced_files_removes_new_ones_and_keeps_later_user_edits() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[
            ("a1.txt", "1\n"),
            ("fundo/x/a2.txt", "2\n"),
            ("src/app.txt", "app v2\n"),
        ],
    );
    let plan = plan_of(&env, &zip);
    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/r")).unwrap();
    let result = apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/r")).unwrap();
    assert_eq!(result.state, ImportState::Applied);

    // O usuário editou um dos arquivos importados depois: a reversão não pode destruir isso.
    write(env.repo.path(), "a1.txt", "edição do usuário\n");
    let rolled = rollback_import(&env.git, &op_req(&env, &result.op_id)).unwrap();
    assert_eq!(rolled.state, ImportState::RolledBack);
    assert_eq!(rolled.kept_modified, vec!["a1.txt".to_string()]);
    assert_eq!(rolled.restored, vec!["src/app.txt".to_string()]);
    assert_eq!(rolled.removed, vec!["fundo/x/a2.txt".to_string()]);
    assert_eq!(read(env.repo.path(), "a1.txt"), "edição do usuário\n");
    assert_eq!(read(env.repo.path(), "src/app.txt"), "app v1\n");
    no_files_written(&env, &["fundo"]);
    // Já desfeita: não há mais o que desfazer, e a operação deixa de ser "pendente".
    assert!(rollback_import(&env.git, &op_req(&env, &result.op_id)).is_err());
    assert!(find_import_operation(&env.git, &env.path)
        .unwrap()
        .is_none());
}

#[test]
fn rollback_is_refused_once_the_work_branch_has_commits_made_outside_the_flow() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[("novo.txt", "n\n"), ("src/app.txt", "app v2\n")],
    );
    let (_plan, result) = import_all(&env, &zip, "importacao/fora");
    // O usuário commita pela aba "Alterações" (fora do fluxo de importação).
    run_git(env.repo.path(), &["add", "-A"]);
    run_git(env.repo.path(), &["commit", "-q", "-m", "commit manual"]);
    let err = rollback_import(&env.git, &op_req(&env, &result.op_id)).unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("git revert"), "{}", err.message);
    // Nada foi removido nem revertido no disco.
    assert_eq!(read(env.repo.path(), "novo.txt"), "n\n");
    assert_eq!(read(env.repo.path(), "src/app.txt"), "app v2\n");
    assert!(get_status(&env.git, &env.path).unwrap().files.is_empty());
}

#[test]
fn rollback_after_a_partial_import_returns_to_the_original_state() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[
            ("a1.txt", "1\n"),
            ("src/app.txt", "app v2\n"),
            ("a3.txt", "3\n"),
        ],
    );
    let result = partial_import(&env, &zip, "importacao/rp", 2);
    assert_eq!(result.state, ImportState::Partial);
    let rolled = rollback_import(&env.git, &op_req(&env, &result.op_id)).unwrap();
    assert!(rolled.problems.is_empty(), "{:?}", rolled.problems);
    assert!(get_status(&env.git, &env.path).unwrap().files.is_empty());
    assert_eq!(read(env.repo.path(), "src/app.txt"), "app v1\n");
    no_files_written(&env, &["a1.txt", "a3.txt"]);
}

#[test]
fn a_new_import_is_not_started_automatically_over_a_partial_state() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[("a1.txt", "1\n"), ("a2.txt", "2\n"), ("a3.txt", "3\n")],
    );
    let _ = partial_import(&env, &zip, "importacao/np", 1);
    // O repositório está sujo (resultado parcial): qualquer novo plano/preparo é bloqueado.
    let plan = plan_of(&env, &zip);
    assert!(!plan.can_apply);
    assert!(!plan.blockers.is_empty());
}

// ----------------------------------------------------------------------------
// Commit isolado
// ----------------------------------------------------------------------------

#[test]
fn commit_contains_only_selected_imported_files_and_refuses_foreign_ones() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[
            ("novo.txt", "n\n"),
            ("src/app.txt", "app v2\n"),
            ("extra.txt", "e\n"),
        ],
    );
    let (_plan, result) = import_all(&env, &zip, "importacao/c");
    // Depois da importação, algo alheio à operação aparece no repositório.
    write(env.repo.path(), "keep.txt", "edição alheia\n");
    write(env.repo.path(), "alheio.txt", "x\n");

    // Arquivo que não pertence à importação não entra no commit.
    let err = commit_import(
        &env,
        &result.op_id,
        &["novo.txt", "keep.txt"],
        "importacao/c",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert_eq!(commit_count(&env), 1);

    // Seleção parcial: só `novo.txt` e `src/app.txt`; `extra.txt` continua fora.
    let commit = commit_import(
        &env,
        &result.op_id,
        &["novo.txt", "src/app.txt"],
        "importacao/c",
    )
    .unwrap();
    assert_eq!(commit_count(&env), 2);
    let files = git_out(
        env.repo.path(),
        &["show", "--name-only", "--format=", "HEAD"],
    );
    let mut listed: Vec<&str> = files.lines().collect();
    listed.sort();
    assert_eq!(listed, vec!["novo.txt", "src/app.txt"]);
    assert_eq!(git_out(env.repo.path(), &["rev-parse", "HEAD"]), commit.oid);
    // O que não foi autorizado continua intacto no disco.
    assert_eq!(read(env.repo.path(), "keep.txt"), "edição alheia\n");
    assert_eq!(read(env.repo.path(), "alheio.txt"), "x\n");
    assert_eq!(read(env.repo.path(), "extra.txt"), "e\n");

    let op = find_import_operation(&env.git, &env.path).unwrap().unwrap();
    assert_eq!(op.state, ImportState::Committed);
    assert_eq!(op.commit_oid.as_deref(), Some(commit.oid.as_str()));
}

#[test]
fn commit_is_never_duplicated_and_requires_the_work_branch() {
    let env = env();
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let (_plan, result) = import_all(&env, &zip, "importacao/d");

    // Branch esperada errada.
    let err = commit_import(&env, &result.op_id, &["novo.txt"], "main").unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    assert_eq!(commit_count(&env), 1);

    // Falha na criação (mensagem vazia): nada é commitado e a operação continua "applied".
    let review = prepare_commit_review(
        &env.git,
        &ReviewRequest {
            repo_path: env.path.clone(),
            files: vec!["novo.txt".into()],
        },
    )
    .unwrap();
    let err = create_import_commit(
        &env.git,
        &ImportCommitRequest {
            op_id: result.op_id.clone(),
            repo_path: env.path.clone(),
            files: vec!["novo.txt".into()],
            message: "   ".into(),
            expected_branch: "importacao/d".into(),
            fingerprint: review.fingerprint,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(commit_count(&env), 1);
    assert_eq!(
        find_import_operation(&env.git, &env.path)
            .unwrap()
            .unwrap()
            .state,
        ImportState::Applied
    );

    // Sucesso uma vez; a segunda tentativa não cria outro commit.
    commit_import(&env, &result.op_id, &["novo.txt"], "importacao/d").unwrap();
    assert_eq!(commit_count(&env), 2);

    // Retomada pela interface: a revisão já não encontra alterações pendentes.
    let err = commit_import(&env, &result.op_id, &["novo.txt"], "importacao/d").unwrap_err();
    assert_eq!(err.code, ErrorCode::StaleReview);
    // Chamada direta ao backend (sem revisão): a própria operação recusa um segundo commit.
    let err = create_import_commit(
        &env.git,
        &ImportCommitRequest {
            op_id: result.op_id.clone(),
            repo_path: env.path.clone(),
            files: vec!["novo.txt".into()],
            message: "de novo".into(),
            expected_branch: "importacao/d".into(),
            fingerprint: "qualquer".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
    assert!(err.message.contains("já foi criado"), "{}", err.message);
    assert_eq!(commit_count(&env), 2);
}

#[test]
fn malformed_operation_ids_are_refused_without_touching_the_filesystem() {
    let env = env();
    for bad in [
        "../../etc",
        "..",
        "a/b",
        "",
        "ABCDEF0123456789",
        "zzzzzzzzzzzzzzzz",
    ] {
        assert!(
            rollback_import(&env.git, &op_req(&env, bad)).is_err(),
            "{bad:?}"
        );
        assert!(
            finish_import(&env.git, &op_req(&env, bad)).is_err(),
            "{bad:?}"
        );
    }
    // Operação com formato válido, porém inexistente.
    assert!(rollback_import(&env.git, &op_req(&env, "0123456789abcdef")).is_err());
}

// ----------------------------------------------------------------------------
// Push (reutiliza a Fase 3)
// ----------------------------------------------------------------------------

struct Remote {
    bare: TempDir,
}

/// Cria um remoto bare com `main` publicada e a branch de trabalho já com upstream configurado.
fn with_remote(env: &Env, work: &str, upstream: bool) -> Remote {
    let bare = tempfile::Builder::new()
        .prefix("gitflow-fase4-remoto-")
        .tempdir()
        .unwrap();
    run_git(bare.path(), &["init", "-q", "--bare", "-b", "main"]);
    run_git(
        env.repo.path(),
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    );
    run_git(env.repo.path(), &["push", "-q", "-u", "origin", "main"]);
    if upstream {
        // Cria a branch de trabalho (no mesmo commit da base) e a publica ANTES da importação,
        // como o usuário faria para ter um destino explícito.
        run_git(
            env.repo.path(),
            &["push", "-q", "origin", &format!("main:refs/heads/{work}")],
        );
    }
    Remote { bare }
}

fn target(work: &str) -> SyncTarget {
    SyncTarget {
        branch: work.into(),
        remote: "origin".into(),
        remote_branch: work.into(),
    }
}

fn push_req(env: &Env, op_id: &str, work: &str) -> PushImportRequest {
    PushImportRequest {
        op_id: op_id.into(),
        repo_path: env.path.clone(),
        target: target(work),
        confirm_protected_branch: false,
    }
}

/// Importa, configura o upstream explicitamente e commita. Devolve o `op_id`.
fn committed_import(env: &Env, work: &str) -> String {
    let zip = make_zip(
        env,
        "p.zip",
        &[("novo.txt", "n\n"), ("src/app.txt", "app v2\n")],
    );
    let plan = plan_of(env, &zip);
    prepare_import_branch(&env.git, &prepare_req(env, &zip, &plan, work)).unwrap();
    run_git(
        env.repo.path(),
        &["branch", &format!("--set-upstream-to=origin/{work}")],
    );
    let result = apply_import(&env.git, &apply_req(env, &zip, &plan, work)).unwrap();
    commit_import(env, &result.op_id, &["novo.txt", "src/app.txt"], work).unwrap();
    result.op_id
}

#[test]
fn push_after_commit_uses_the_phase3_push_and_finishes_the_operation() {
    let env = env();
    let remote = with_remote(&env, "importacao/push", true);
    run_git(env.repo.path(), &["fetch", "-q", "origin"]);
    let op_id = committed_import(&env, "importacao/push");
    let local_head = git_out(env.repo.path(), &["rev-parse", "HEAD"]);

    let result = push_import_branch(&env.git, &push_req(&env, &op_id, "importacao/push")).unwrap();
    let _ = result;
    assert_eq!(
        git_out(
            remote.bare.path(),
            &["rev-parse", "refs/heads/importacao/push"]
        ),
        local_head
    );
    // Somente a branch de trabalho avançou; `main` do remoto permanece onde estava.
    let main_remote = git_out(remote.bare.path(), &["rev-parse", "refs/heads/main"]);
    assert_ne!(main_remote, local_head);
    // Operação encerrada.
    assert!(find_import_operation(&env.git, &env.path)
        .unwrap()
        .is_none());
}

#[test]
fn push_without_upstream_is_refused_and_no_destination_is_invented() {
    let env = env();
    let remote = with_remote(&env, "importacao/sem", false);
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);
    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/sem")).unwrap();
    let result = apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/sem")).unwrap();
    commit_import(&env, &result.op_id, &["novo.txt"], "importacao/sem").unwrap();

    let err =
        push_import_branch(&env.git, &push_req(&env, &result.op_id, "importacao/sem")).unwrap_err();
    assert!(
        matches!(
            err.code,
            ErrorCode::OperationBlocked
                | ErrorCode::RemoteBranchMissing
                | ErrorCode::InvalidArgument
                | ErrorCode::StaleReview
        ),
        "{err:?}"
    );
    // Nada foi criado no remoto e nenhum upstream foi configurado.
    assert!(git_out(remote.bare.path(), &["branch", "--list", "importacao/sem"]).is_empty());
    assert!(git_cmd(
        env.repo.path(),
        &["config", "--get", "branch.importacao/sem.remote"]
    )
    .stdout
    .is_empty());
    // O commit local foi preservado e a operação continua "committed".
    assert_eq!(
        find_import_operation(&env.git, &env.path)
            .unwrap()
            .unwrap()
            .state,
        ImportState::Committed
    );
    assert_eq!(commit_count(&env), 2);
}

#[test]
fn rejected_push_keeps_the_local_commit_and_does_not_retry_or_force() {
    let env = env();
    let remote = with_remote(&env, "importacao/rej", true);
    run_git(env.repo.path(), &["fetch", "-q", "origin"]);
    let op_id = committed_import(&env, "importacao/rej");

    // Outra pessoa avança a branch remota depois do commit local.
    let other = tempfile::tempdir().unwrap();
    run_git(
        other.path(),
        &[
            "clone",
            "-q",
            "-b",
            "importacao/rej",
            remote.bare.path().to_str().unwrap(),
            ".",
        ],
    );
    configure_identity(other.path());
    write(other.path(), "alheio.txt", "de outra pessoa\n");
    commit_all(other.path(), "outra pessoa");
    run_git(other.path(), &["push", "-q", "origin", "importacao/rej"]);
    let remote_head = git_out(
        remote.bare.path(),
        &["rev-parse", "refs/heads/importacao/rej"],
    );

    run_git(env.repo.path(), &["fetch", "-q", "origin"]);
    let err = push_import_branch(&env.git, &push_req(&env, &op_id, "importacao/rej")).unwrap_err();
    assert!(
        matches!(
            err.code,
            ErrorCode::PushRejected | ErrorCode::OperationBlocked | ErrorCode::StaleReview
        ),
        "{err:?}"
    );
    // O remoto não foi sobrescrito, o commit local existe e a operação segue em "committed".
    assert_eq!(
        git_out(
            remote.bare.path(),
            &["rev-parse", "refs/heads/importacao/rej"]
        ),
        remote_head
    );
    assert_eq!(
        commit_count(&env),
        3 /* inicial, commit local, commit da outra pessoa (remoto) */
    );
    assert_eq!(
        find_import_operation(&env.git, &env.path)
            .unwrap()
            .unwrap()
            .state,
        ImportState::Committed
    );
}

#[test]
fn push_only_accepts_the_work_branch_after_the_commit_and_with_the_import_head() {
    let env = env();
    let _remote = with_remote(&env, "importacao/alvo", true);
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let plan = plan_of(&env, &zip);
    prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/alvo")).unwrap();
    run_git(
        env.repo.path(),
        &["branch", "--set-upstream-to=origin/importacao/alvo"],
    );
    let result = apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/alvo")).unwrap();

    // Antes do commit: recusado.
    let err = push_import_branch(&env.git, &push_req(&env, &result.op_id, "importacao/alvo"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);

    commit_import(&env, &result.op_id, &["novo.txt"], "importacao/alvo").unwrap();
    // Alvo diferente da branch de trabalho (por exemplo, main): recusado.
    let mut wrong = push_req(&env, &result.op_id, "importacao/alvo");
    wrong.target = target("main");
    assert_eq!(
        push_import_branch(&env.git, &wrong).unwrap_err().code,
        ErrorCode::OperationBlocked
    );

    // Um commit extra depois do commit da importação: o fluxo recusa (revisão própria na aba Sincronização).
    write(env.repo.path(), "extra.txt", "x\n");
    commit_all(env.repo.path(), "commit fora do fluxo");
    let err = push_import_branch(&env.git, &push_req(&env, &result.op_id, "importacao/alvo"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OperationBlocked);
}

#[test]
fn finish_is_blocked_until_committed_and_cleans_temporary_data() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[("novo.txt", "n\n"), ("src/app.txt", "app v2\n")],
    );
    let (_plan, result) = import_all(&env, &zip, "importacao/f");
    assert_eq!(
        finish_import(&env.git, &op_req(&env, &result.op_id))
            .unwrap_err()
            .code,
        ErrorCode::OperationBlocked
    );

    let git_dir = env
        .repo
        .path()
        .join(".git")
        .join("gitflow-import")
        .join(&result.op_id);
    assert!(git_dir.join("journal.json").exists());
    assert!(
        git_dir.join("backup").exists(),
        "o backup do arquivo substituído deve existir até o encerramento"
    );

    commit_import(
        &env,
        &result.op_id,
        &["novo.txt", "src/app.txt"],
        "importacao/f",
    )
    .unwrap();
    let op = finish_import(&env.git, &op_req(&env, &result.op_id)).unwrap();
    assert_eq!(op.state, ImportState::Finished);
    assert!(!git_dir.join("backup").exists());
    assert!(!git_dir.join("staged").exists());
    assert!(find_import_operation(&env.git, &env.path)
        .unwrap()
        .is_none());
    // Nenhum temporário ficou no repositório.
    assert!(get_status(&env.git, &env.path).unwrap().files.is_empty());
}

#[test]
fn pending_operation_can_be_found_after_restart_and_points_at_the_right_state() {
    let env = env();
    assert!(find_import_operation(&env.git, &env.path)
        .unwrap()
        .is_none());
    let zip = make_zip(&env, "p.zip", &[("novo.txt", "n\n")]);
    let (_plan, result) = import_all(&env, &zip, "importacao/t");
    let op = find_import_operation(&env.git, &env.path).unwrap().unwrap();
    assert_eq!(op.op_id, result.op_id);
    assert_eq!(op.state, ImportState::Applied);
    assert_eq!(op.work_branch, "importacao/t");
    assert_eq!(op.base_branch, "main");
    assert_eq!(op.files.len(), 1);
    assert_eq!(op.files[0].path, "novo.txt");
}

#[allow(dead_code)]
fn _unused(_: PathBuf) {}

// ----------------------------------------------------------------------------
// Contrato JSON com o frontend (src/types/git.ts)
// ----------------------------------------------------------------------------

fn keys(value: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = value
        .as_object()
        .expect("objeto JSON")
        .keys()
        .cloned()
        .collect();
    k.sort();
    k
}

fn sorted(list: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn json_contract_matches_the_frontend_types() {
    let env = env();
    let zip = make_zip(
        &env,
        "p.zip",
        &[("novo.txt", "n\n"), ("src/app.txt", "app v2\n")],
    );
    let plan = plan_of(&env, &zip);
    let plan_json = serde_json::to_value(&plan).unwrap();
    assert_eq!(
        keys(&plan_json),
        sorted(&[
            "planId",
            "repoPath",
            "baseBranch",
            "baseOid",
            "zipPath",
            "zipSize",
            "zipSha256",
            "sourcePrefix",
            "layout",
            "entries",
            "summary",
            "blockers",
            "warnings",
            "canApply"
        ])
    );
    assert_eq!(
        keys(&plan_json["layout"]),
        sorted(&[
            "topLevelDirs",
            "hasRootFiles",
            "suggestedPrefix",
            "directories"
        ])
    );
    assert_eq!(
        keys(&plan_json["summary"]),
        sorted(&[
            "totalEntries",
            "directories",
            "filesInScope",
            "add",
            "replace",
            "identical",
            "blocked",
            "ignored",
            "bytesToWrite",
            "uncompressedBytes"
        ])
    );
    assert_eq!(
        keys(&plan_json["entries"][0]),
        sorted(&[
            "path",
            "action",
            "size",
            "reason",
            "newSha256",
            "oldSha256",
            "zipIndex",
            "zipName"
        ])
    );
    assert_eq!(plan_json["entries"][0]["action"], "add");

    let prepared =
        prepare_import_branch(&env.git, &prepare_req(&env, &zip, &plan, "importacao/j")).unwrap();
    assert_eq!(
        keys(&serde_json::to_value(&prepared).unwrap()),
        sorted(&["repoPath", "workBranch", "baseBranch", "baseOid"])
    );

    let result = apply_import(&env.git, &apply_req(&env, &zip, &plan, "importacao/j")).unwrap();
    let result_json = serde_json::to_value(&result).unwrap();
    assert_eq!(
        keys(&result_json),
        sorted(&[
            "opId",
            "state",
            "repoPath",
            "workBranch",
            "applied",
            "failure",
            "notAttempted",
            "identicalSkipped",
            "recovery"
        ])
    );
    assert_eq!(result_json["state"], "applied");
    assert_eq!(
        keys(&result_json["applied"][0]),
        sorted(&["path", "action", "visibleInGit"])
    );

    let op = find_import_operation(&env.git, &env.path).unwrap().unwrap();
    let op_json = serde_json::to_value(&op).unwrap();
    assert_eq!(
        keys(&op_json),
        sorted(&[
            "opId",
            "state",
            "repoPath",
            "workBranch",
            "baseBranch",
            "baseOid",
            "zipPath",
            "sourcePrefix",
            "files",
            "commitOid",
            "createdAt"
        ])
    );
    assert_eq!(keys(&op_json["files"][0]), sorted(&["path", "action"]));

    let rolled = rollback_import(&env.git, &op_req(&env, &result.op_id)).unwrap();
    let rolled_json = serde_json::to_value(&rolled).unwrap();
    assert_eq!(
        keys(&rolled_json),
        sorted(&[
            "opId",
            "state",
            "restored",
            "removed",
            "keptModified",
            "problems"
        ])
    );
    assert_eq!(rolled_json["state"], "rolledBack");
}

#[test]
fn requests_deserialize_from_the_exact_json_the_frontend_sends() {
    use serde_json::json;
    let inspect: InspectRequest = serde_json::from_value(json!({
        "repoPath": "/r", "zipPath": "/z.zip", "sourcePrefix": "", "baseBranch": "main"
    }))
    .unwrap();
    assert_eq!(inspect.base_branch, "main");

    let prepare: PrepareBranchRequest = serde_json::from_value(json!({
        "repoPath": "/r", "zipPath": "/z.zip", "sourcePrefix": "projeto", "baseBranch": "main",
        "workBranch": "importacao/x", "planId": "abc"
    }))
    .unwrap();
    assert_eq!(prepare.work_branch, "importacao/x");

    let apply: ApplyImportRequest = serde_json::from_value(json!({
        "repoPath": "/r", "zipPath": "/z.zip", "sourcePrefix": "", "baseBranch": "main",
        "workBranch": "importacao/x", "planId": "abc",
        "excludedPaths": ["a.txt"], "confirmOverwrites": true
    }))
    .unwrap();
    assert!(apply.confirm_overwrites);
    assert_eq!(apply.excluded_paths, vec!["a.txt".to_string()]);

    let op: ImportOperationRequest =
        serde_json::from_value(json!({ "repoPath": "/r", "opId": "0123456789abcdef" })).unwrap();
    assert_eq!(op.op_id, "0123456789abcdef");

    let commit: ImportCommitRequest = serde_json::from_value(json!({
        "opId": "0123456789abcdef", "repoPath": "/r", "files": ["a.txt"], "message": "m",
        "expectedBranch": "importacao/x", "fingerprint": "f"
    }))
    .unwrap();
    assert_eq!(commit.expected_branch, "importacao/x");

    let push: PushImportRequest = serde_json::from_value(json!({
        "opId": "0123456789abcdef", "repoPath": "/r",
        "target": { "branch": "importacao/x", "remote": "origin", "remoteBranch": "importacao/x" },
        "confirmProtectedBranch": false
    }))
    .unwrap();
    assert_eq!(push.target.remote_branch, "importacao/x");
}
