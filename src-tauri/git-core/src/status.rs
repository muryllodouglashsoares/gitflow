//! Estado do repositório: branch, arquivos alterados e operação em andamento.
//!
//! Usa `git status --porcelain=v2 -z --branch --untracked-files=all`, formato estável
//! pensado para programas: registros separados por NUL, sem aspas e sem escapes, o que
//! torna seguros nomes com espaços, acentos ou caracteres especiais.

use crate::{
    error::{ErrorCode, GitError, Result},
    repository::require_repository_root,
    runner::{Git, RunOptions},
};
use serde::Serialize;
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// Caminho relativo à raiz do repositório, com `/` como separador.
    pub path: String,
    /// Caminho original (apenas em renomeações/cópias).
    pub orig_path: Option<String>,
    /// Alteração preparada (staged), se houver.
    pub index: Option<ChangeKind>,
    /// Alteração ainda não preparada no diretório de trabalho, se houver.
    pub worktree: Option<ChangeKind>,
    pub untracked: bool,
    pub conflicted: bool,
    /// Código de conflito do Git (`UU`, `AA`, `DU`...), quando `conflicted`.
    pub conflict_code: Option<String>,
    pub submodule: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    /// Nome da branch; `None` com HEAD desanexado.
    pub name: Option<String>,
    pub detached: bool,
    /// Branch ainda sem commits.
    pub unborn: bool,
    pub head_oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusSummary {
    pub total: usize,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStatus {
    /// Raiz do repositório consultado (permite à interface descartar respostas de outra seleção).
    pub repo_path: String,
    pub branch: BranchInfo,
    pub files: Vec<FileEntry>,
    pub summary: StatusSummary,
    pub is_clean: bool,
    pub operation: Option<Operation>,
}

pub fn get_status(git: &Git, repo_path: &str) -> Result<RepositoryStatus> {
    let root = require_repository_root(git, repo_path)?;

    let opts = RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 16 * 1024 * 1024,
        ..RunOptions::default()
    };
    let out = git.run(
        Some(&root),
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
        ],
        &opts,
    )?;
    if out.stdout_truncated {
        return Err(GitError::new(
            ErrorCode::OutputTooLarge,
            "Há alterações demais para listar. Reduza o número de arquivos não rastreados \
             (por exemplo, adicionando pastas geradas ao .gitignore).",
        ));
    }

    let (branch, files) = parse_porcelain_v2(&out.stdout)?;
    let summary = summarize(&files);
    let operation = detect_operation(git, &root);

    Ok(RepositoryStatus {
        repo_path: root.to_string_lossy().into_owned(),
        is_clean: files.is_empty(),
        branch,
        files,
        summary,
        operation,
    })
}

fn summarize(files: &[FileEntry]) -> StatusSummary {
    StatusSummary {
        total: files.len(),
        staged: files.iter().filter(|f| f.index.is_some()).count(),
        unstaged: files.iter().filter(|f| f.worktree.is_some()).count(),
        untracked: files.iter().filter(|f| f.untracked).count(),
        conflicted: files.iter().filter(|f| f.conflicted).count(),
    }
}

/// Interpreta a saída de `git status --porcelain=v2 -z --branch`.
pub fn parse_porcelain_v2(output: &[u8]) -> Result<(BranchInfo, Vec<FileEntry>)> {
    let mut branch = BranchInfo::default();
    let mut files = Vec::new();

    let mut tokens = output
        .split(|b| *b == 0)
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .filter(|t| !t.is_empty());

    while let Some(token) = tokens.next() {
        if let Some(header) = token.strip_prefix("# ") {
            parse_branch_header(header, &mut branch);
            continue;
        }

        match token.as_bytes().first() {
            Some(b'1') => files.push(parse_ordinary(&token)?),
            Some(b'2') => {
                // Renomeação/cópia: o caminho original vem no registro seguinte (por causa do -z).
                let orig = tokens.next().ok_or_else(|| {
                    unexpected("registro de renomeação sem o caminho original")
                })?;
                files.push(parse_renamed(&token, orig)?);
            }
            Some(b'u') => files.push(parse_unmerged(&token)?),
            Some(b'?') => {
                let path = token
                    .strip_prefix("? ")
                    .ok_or_else(|| unexpected("registro de arquivo não rastreado inválido"))?;
                files.push(FileEntry {
                    path: path.to_string(),
                    orig_path: None,
                    index: None,
                    worktree: None,
                    untracked: true,
                    conflicted: false,
                    conflict_code: None,
                    submodule: false,
                });
            }
            Some(b'!') => {} // ignorados: não solicitados, mas inofensivos
            _ => return Err(unexpected("tipo de registro desconhecido")),
        }
    }

    Ok((branch, files))
}

fn parse_branch_header(header: &str, branch: &mut BranchInfo) {
    if let Some(oid) = header.strip_prefix("branch.oid ") {
        if oid == "(initial)" {
            branch.unborn = true;
        } else {
            branch.head_oid = Some(oid.to_string());
        }
    } else if let Some(head) = header.strip_prefix("branch.head ") {
        if head == "(detached)" {
            branch.detached = true;
        } else {
            branch.name = Some(head.to_string());
        }
    } else if let Some(upstream) = header.strip_prefix("branch.upstream ") {
        branch.upstream = Some(upstream.to_string());
    } else if let Some(ab) = header.strip_prefix("branch.ab ") {
        for part in ab.split_whitespace() {
            if let Some(n) = part.strip_prefix('+') {
                branch.ahead = n.parse().ok();
            } else if let Some(n) = part.strip_prefix('-') {
                branch.behind = n.parse().ok();
            }
        }
    }
}

/// `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
fn parse_ordinary(token: &str) -> Result<FileEntry> {
    let fields: Vec<&str> = token.splitn(9, ' ').collect();
    if fields.len() != 9 {
        return Err(unexpected("registro de alteração incompleto"));
    }
    let (index, worktree) = parse_xy(fields[1])?;
    Ok(FileEntry {
        path: fields[8].to_string(),
        orig_path: None,
        index,
        worktree,
        untracked: false,
        conflicted: false,
        conflict_code: None,
        submodule: fields[2].starts_with('S'),
    })
}

/// `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>` + registro com o caminho original
fn parse_renamed(token: &str, orig: String) -> Result<FileEntry> {
    let fields: Vec<&str> = token.splitn(10, ' ').collect();
    if fields.len() != 10 {
        return Err(unexpected("registro de renomeação incompleto"));
    }
    let (index, worktree) = parse_xy(fields[1])?;
    Ok(FileEntry {
        path: fields[9].to_string(),
        orig_path: Some(orig),
        index,
        worktree,
        untracked: false,
        conflicted: false,
        conflict_code: None,
        submodule: fields[2].starts_with('S'),
    })
}

/// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
fn parse_unmerged(token: &str) -> Result<FileEntry> {
    let fields: Vec<&str> = token.splitn(11, ' ').collect();
    if fields.len() != 11 {
        return Err(unexpected("registro de conflito incompleto"));
    }
    Ok(FileEntry {
        path: fields[10].to_string(),
        orig_path: None,
        index: None,
        worktree: None,
        untracked: false,
        conflicted: true,
        conflict_code: Some(fields[1].to_string()),
        submodule: fields[2].starts_with('S'),
    })
}

fn parse_xy(xy: &str) -> Result<(Option<ChangeKind>, Option<ChangeKind>)> {
    let mut chars = xy.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(x), Some(y), None) => Ok((parse_kind(x)?, parse_kind(y)?)),
        _ => Err(unexpected("código de status inválido")),
    }
}

fn parse_kind(c: char) -> Result<Option<ChangeKind>> {
    Ok(match c {
        '.' => None,
        'M' => Some(ChangeKind::Modified),
        'T' => Some(ChangeKind::TypeChanged),
        'A' => Some(ChangeKind::Added),
        'D' => Some(ChangeKind::Deleted),
        'R' => Some(ChangeKind::Renamed),
        'C' => Some(ChangeKind::Copied),
        _ => return Err(unexpected("código de alteração desconhecido")),
    })
}

fn unexpected(what: &str) -> GitError {
    GitError::new(
        ErrorCode::UnexpectedOutput,
        "Não foi possível interpretar a resposta do Git.",
    )
    .with_details(what)
}

/// Detecta merge/rebase/etc. em andamento lendo marcadores no diretório `.git` (somente leitura).
fn detect_operation(git: &Git, root: &Path) -> Option<Operation> {
    let opts = RunOptions {
        timeout: Duration::from_secs(10),
        max_stdout: 16 * 1024,
        ..RunOptions::default()
    };
    let out = git
        .run(Some(root), &["rev-parse", "--absolute-git-dir"], &opts)
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let git_dir = Path::new(text.trim_end_matches(['\n', '\r']));

    let exists = |name: &str| git_dir.join(name).exists();
    if exists("rebase-merge") || exists("rebase-apply") {
        Some(Operation::Rebase)
    } else if exists("MERGE_HEAD") {
        Some(Operation::Merge)
    } else if exists("CHERRY_PICK_HEAD") {
        Some(Operation::CherryPick)
    } else if exists("REVERT_HEAD") {
        Some(Operation::Revert)
    } else if exists("BISECT_LOG") {
        Some(Operation::Bisect)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nul(parts: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in parts {
            out.extend_from_slice(p.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn parses_branch_headers() {
        let data = nul(&[
            "# branch.oid 1234567890abcdef",
            "# branch.head feature/x",
            "# branch.upstream origin/feature/x",
            "# branch.ab +3 -1",
        ]);
        let (branch, files) = parse_porcelain_v2(&data).unwrap();
        assert!(files.is_empty());
        assert_eq!(branch.name.as_deref(), Some("feature/x"));
        assert_eq!(branch.upstream.as_deref(), Some("origin/feature/x"));
        assert_eq!((branch.ahead, branch.behind), (Some(3), Some(1)));
        assert!(!branch.detached && !branch.unborn);
    }

    #[test]
    fn parses_unborn_and_detached() {
        let (unborn, _) =
            parse_porcelain_v2(&nul(&["# branch.oid (initial)", "# branch.head trunk"])).unwrap();
        assert!(unborn.unborn);
        assert_eq!(unborn.name.as_deref(), Some("trunk"));

        let (detached, _) =
            parse_porcelain_v2(&nul(&["# branch.oid abc123", "# branch.head (detached)"])).unwrap();
        assert!(detached.detached);
        assert_eq!(detached.name, None);
        assert_eq!(detached.head_oid.as_deref(), Some("abc123"));
    }

    #[test]
    fn keeps_spaces_and_special_characters_in_paths() {
        let data = nul(&[
            "1 .M N... 100644 100644 100644 aaaa bbbb dir with space/arquivo ç [1].txt",
            "? novo arquivo.txt",
        ]);
        let (_, files) = parse_porcelain_v2(&data).unwrap();
        assert_eq!(files[0].path, "dir with space/arquivo ç [1].txt");
        assert_eq!(files[0].worktree, Some(ChangeKind::Modified));
        assert_eq!(files[0].index, None);
        assert_eq!(files[1].path, "novo arquivo.txt");
        assert!(files[1].untracked);
    }

    #[test]
    fn parses_rename_with_following_original_path() {
        let data = nul(&[
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 novo nome.txt",
            "nome antigo.txt",
            "? outro.txt",
        ]);
        let (_, files) = parse_porcelain_v2(&data).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "novo nome.txt");
        assert_eq!(files[0].orig_path.as_deref(), Some("nome antigo.txt"));
        assert_eq!(files[0].index, Some(ChangeKind::Renamed));
        assert_eq!(files[1].path, "outro.txt");
    }

    #[test]
    fn parses_unmerged_entries() {
        let data = nul(&["u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflito.txt"]);
        let (_, files) = parse_porcelain_v2(&data).unwrap();
        assert!(files[0].conflicted);
        assert_eq!(files[0].conflict_code.as_deref(), Some("UU"));
        assert_eq!(files[0].path, "conflito.txt");
    }

    #[test]
    fn staged_and_unstaged_on_same_file() {
        let data = nul(&["1 AM N... 000000 100644 100644 aaaa bbbb x.txt"]);
        let (_, files) = parse_porcelain_v2(&data).unwrap();
        assert_eq!(files[0].index, Some(ChangeKind::Added));
        assert_eq!(files[0].worktree, Some(ChangeKind::Modified));
    }

    #[test]
    fn flags_submodules() {
        let data = nul(&["1 .M S.M. 160000 160000 160000 aaaa bbbb libs/sub"]);
        let (_, files) = parse_porcelain_v2(&data).unwrap();
        assert!(files[0].submodule);
    }

    #[test]
    fn rejects_malformed_records() {
        assert!(parse_porcelain_v2(&nul(&["1 .M N... 100644"])).is_err());
        assert!(parse_porcelain_v2(&nul(&["Z qualquer coisa"])).is_err());
        assert!(parse_porcelain_v2(&nul(&["2 R. N... 100644 100644 100644 a b R100 novo"])).is_err());
        assert!(parse_porcelain_v2(&nul(&["1 XX N... 100644 100644 100644 a b p"])).is_err());
    }
}
