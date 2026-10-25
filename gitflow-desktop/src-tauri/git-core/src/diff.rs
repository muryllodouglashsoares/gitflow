//! Diferenças de um arquivo (somente leitura).
//!
//! - alterações preparadas:   `git diff --cached -- <arquivo>`
//! - alterações não preparadas / conflitos: `git diff -- <arquivo>`
//! - arquivos não rastreados: `git diff --no-index -- /dev/null <arquivo>` (prévia como "arquivo novo")
//!
//! O caminho do arquivo vem da interface: é validado, passado após `--` e interpretado
//! de forma literal (`--literal-pathspecs`), sem curingas nem "pathspec magic".

use crate::{
    error::{ErrorCode, GitError, Result},
    repository::require_repository_root,
    runner::{Git, RunOptions},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_LINE_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiffTarget {
    Staged,
    Unstaged,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRequest {
    pub repo_path: String,
    pub path: String,
    /// Caminho original em renomeações preparadas (permite ao Git detectar a renomeação).
    pub orig_path: Option<String>,
    pub target: DiffTarget,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffContent {
    /// Diff textual com hunks.
    Text,
    /// Arquivo binário: não há diff textual.
    Binary,
    /// Sem diferenças a exibir.
    Empty,
    /// Situação sem diff textual (ex.: pasta/repositório aninhado).
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LineKind {
    Context,
    Added,
    Removed,
    NoNewline,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub repo_path: String,
    pub path: String,
    pub target: DiffTarget,
    pub content: DiffContent,
    pub hunks: Vec<Hunk>,
    pub additions: u32,
    pub deletions: u32,
    pub is_new: bool,
    pub is_deleted: bool,
    pub renamed_from: Option<String>,
    /// A saída do Git passou do limite e foi cortada.
    pub truncated: bool,
    /// Explicação para os casos sem hunks (binário, vazio, não suportado).
    pub notice: Option<String>,
}

pub fn get_file_diff(git: &Git, request: &DiffRequest) -> Result<FileDiff> {
    let root = require_repository_root(git, &request.repo_path)?;
    validate_relative_path(&request.path)?;
    if let Some(orig) = &request.orig_path {
        validate_relative_path(orig)?;
    }

    let mut diff = FileDiff {
        repo_path: root.to_string_lossy().into_owned(),
        path: request.path.clone(),
        target: request.target,
        content: DiffContent::Empty,
        hunks: Vec::new(),
        additions: 0,
        deletions: 0,
        is_new: false,
        is_deleted: false,
        renamed_from: None,
        truncated: false,
        notice: None,
    };

    // Pastas e repositórios aninhados aparecem como "pasta/" e não têm diff textual.
    if request.path.ends_with('/') {
        diff.content = DiffContent::Unsupported;
        diff.notice = Some(
            "Esta entrada é uma pasta ou repositório aninhado; não há diferenças textuais para exibir."
                .to_string(),
        );
        return Ok(diff);
    }

    let mut args: Vec<&str> = vec![
        "--literal-pathspecs",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--unified=3",
    ];
    let mut ok_exit_codes: &[i32] = &[0];
    match request.target {
        DiffTarget::Staged => {
            args.extend(["--cached", "-M", "--"]);
            if let Some(orig) = request.orig_path.as_deref() {
                args.push(orig);
            }
            args.push(request.path.as_str());
        }
        DiffTarget::Unstaged | DiffTarget::Conflicted => {
            args.extend(["--no-renames", "--", request.path.as_str()]);
        }
        DiffTarget::Untracked => {
            // `--no-index` sai com código 1 quando há diferenças (comportamento esperado).
            args.extend(["--no-index", "--", "/dev/null", request.path.as_str()]);
            ok_exit_codes = &[0, 1];
        }
    }

    let opts = RunOptions {
        timeout: Duration::from_secs(30),
        max_stdout: MAX_DIFF_BYTES,
        ok_exit_codes,
    };
    let out = git.run(Some(&root), &args, &opts)?;
    diff.truncated = out.stdout_truncated;

    let parsed = parse_unified_diff(&out.stdout);
    diff.additions = parsed.additions;
    diff.deletions = parsed.deletions;
    diff.is_new = parsed.is_new;
    diff.is_deleted = parsed.is_deleted;
    if parsed.renamed {
        diff.renamed_from = request.orig_path.clone();
    }

    if parsed.binary {
        diff.content = DiffContent::Binary;
        diff.notice = Some(
            "Arquivo binário: não é possível exibir diferenças textuais.".to_string(),
        );
    } else if parsed.hunks.is_empty() {
        diff.content = DiffContent::Empty;
        let notice = if parsed.renamed && !parsed.mode_changed {
            "Arquivo renomeado ou movido sem alterações de conteúdo."
        } else {
            empty_notice(request.target, parsed.mode_changed)
        };
        diff.notice = Some(notice.to_string());
    } else {
        diff.content = DiffContent::Text;
        diff.hunks = parsed.hunks;
    }
    Ok(diff)
}

fn empty_notice(target: DiffTarget, mode_changed: bool) -> &'static str {
    if mode_changed {
        return "Somente as permissões do arquivo foram alteradas; não há mudanças de conteúdo.";
    }
    match target {
        DiffTarget::Untracked => "Arquivo novo e vazio: não há conteúdo para exibir.",
        DiffTarget::Staged => "Não há diferenças preparadas para este arquivo.",
        DiffTarget::Unstaged => "Não há diferenças não preparadas para este arquivo.",
        DiffTarget::Conflicted => "Não há diferenças textuais disponíveis para este conflito.",
    }
}

/// Aceita somente caminhos relativos à raiz do repositório e sem componentes `..`.
pub(crate) fn validate_relative_path(path: &str) -> Result<()> {
    let invalid = |msg: &str| Err(GitError::new(ErrorCode::InvalidArgument, msg.to_string()));

    if path.is_empty() {
        return invalid("O caminho do arquivo está vazio.");
    }
    if path.contains('\0') {
        return invalid("O caminho do arquivo contém caracteres inválidos.");
    }
    let bytes = path.as_bytes();
    let looks_absolute = path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    if looks_absolute {
        return invalid("O caminho do arquivo deve ser relativo ao repositório.");
    }
    let is_separator = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    if path.split(is_separator).any(|segment| segment == "..") {
        return invalid("O caminho do arquivo não pode conter \"..\".");
    }
    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct ParsedDiff {
    pub hunks: Vec<Hunk>,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub is_new: bool,
    pub is_deleted: bool,
    pub renamed: bool,
    pub mode_changed: bool,
}

/// Interpreta um diff unificado (inclusive o formato combinado `@@@` de conflitos).
pub(crate) fn parse_unified_diff(output: &[u8]) -> ParsedDiff {
    let text = String::from_utf8_lossy(output);
    let mut parsed = ParsedDiff::default();

    let mut sections = 0u32;
    let (mut new_flag, mut deleted_flag) = (false, false);
    let mut current: Option<Hunk> = None;
    let mut columns = 1usize; // colunas de prefixo (2 ou mais em diffs combinados)
    let (mut old_no, mut new_no) = (0u32, 0u32);
    let mut combined = false;

    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);

        if line.starts_with("diff --git ") || line.starts_with("diff --cc ") || line.starts_with("diff --combined ") {
            if let Some(h) = current.take() {
                parsed.hunks.push(h);
            }
            sections += 1;
            continue;
        }

        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                parsed.hunks.push(h);
            }
            let at_count = line.chars().take_while(|c| *c == '@').count();
            columns = at_count.saturating_sub(1).max(1);
            combined = columns > 1;
            let (o, n) = hunk_starts(line, at_count);
            old_no = o;
            new_no = n;
            current = Some(Hunk {
                header: crate::error::truncate_chars(line, MAX_LINE_CHARS),
                lines: Vec::new(),
            });
            continue;
        }

        match current.as_mut() {
            None => {
                // Cabeçalhos do arquivo.
                if line == "new file mode" || line.starts_with("new file mode ") {
                    new_flag = true;
                } else if line.starts_with("deleted file mode ") {
                    deleted_flag = true;
                } else if line.starts_with("rename from ") || line.starts_with("rename to ") {
                    parsed.renamed = true;
                } else if line.starts_with("old mode ") || line.starts_with("new mode ") {
                    parsed.mode_changed = true;
                } else if (line.starts_with("Binary files ") && line.ends_with(" differ"))
                    || line == "GIT binary patch"
                {
                    parsed.binary = true;
                }
            }
            Some(hunk) => {
                if line.starts_with('\\') {
                    hunk.lines.push(DiffLine {
                        kind: LineKind::NoNewline,
                        old_no: None,
                        new_no: None,
                        text: crate::error::truncate_chars(line, MAX_LINE_CHARS),
                    });
                    continue;
                }
                if raw.is_empty() {
                    // Última "linha" após o \n final (ou artefato de corte): ignora.
                    continue;
                }
                let prefix: String = line.chars().take(columns).collect();
                let body: String = line.chars().skip(columns).collect();
                let kind = if prefix.contains('+') {
                    LineKind::Added
                } else if prefix.contains('-') {
                    LineKind::Removed
                } else {
                    LineKind::Context
                };
                let (o, n) = match kind {
                    LineKind::Context => {
                        let pair = (Some(old_no), Some(new_no));
                        old_no += 1;
                        new_no += 1;
                        pair
                    }
                    LineKind::Added => {
                        parsed.additions += 1;
                        let pair = (None, Some(new_no));
                        new_no += 1;
                        pair
                    }
                    LineKind::Removed => {
                        parsed.deletions += 1;
                        let pair = (Some(old_no), None);
                        old_no += 1;
                        pair
                    }
                    LineKind::NoNewline => (None, None),
                };
                hunk.lines.push(DiffLine {
                    kind,
                    // Em diffs combinados não há uma única numeração "antiga".
                    old_no: if combined { None } else { o },
                    new_no: n,
                    text: crate::error::truncate_chars(&body, MAX_LINE_CHARS),
                });
            }
        }
    }
    if let Some(h) = current.take() {
        parsed.hunks.push(h);
    }

    // Flags só fazem sentido quando o diff trata de um único arquivo (ex.: troca de tipo gera dois).
    if sections <= 1 {
        parsed.is_new = new_flag;
        parsed.is_deleted = deleted_flag;
    }
    parsed
}

/// Extrai os números iniciais (antigo/novo) de um cabeçalho `@@ -a,b +c,d @@`.
fn hunk_starts(header: &str, at_count: usize) -> (u32, u32) {
    let after = &header[at_count..];
    let ranges = after.split(&"@".repeat(at_count)).next().unwrap_or("");
    let mut old_start = 0;
    let mut new_start = 0;
    for token in ranges.split_whitespace() {
        let number = |t: &str| -> u32 {
            t.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(0)
        };
        if let Some(rest) = token.strip_prefix('-') {
            if old_start == 0 {
                old_start = number(rest);
            }
        } else if let Some(rest) = token.strip_prefix('+') {
            new_start = number(rest);
        }
    }
    (old_start, new_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_validation() {
        assert!(validate_relative_path("src/main.rs").is_ok());
        assert!(validate_relative_path("pasta com espaço/arq ç.txt").is_ok());
        assert!(validate_relative_path("-estranho.txt").is_ok());
        for bad in ["", "../fora", "a/../../b", "/etc/passwd", "\\\\server\\share", "C:\\x", "a\0b"] {
            assert_eq!(
                validate_relative_path(bad).unwrap_err().code,
                ErrorCode::InvalidArgument,
                "{bad:?} deveria ser rejeitado"
            );
        }
    }

    #[test]
    fn parses_text_diff_with_line_numbers() {
        let diff = "diff --git a/f.txt b/f.txt\nindex 111..222 100644\n--- a/f.txt\n+++ b/f.txt\n\
                    @@ -1,3 +1,3 @@ fn x\n um\n-dois\n+DOIS\n tres\n";
        let p = parse_unified_diff(diff.as_bytes());
        assert_eq!((p.additions, p.deletions), (1, 1));
        assert_eq!(p.hunks.len(), 1);
        let lines = &p.hunks[0].lines;
        assert_eq!(lines[0].kind, LineKind::Context);
        assert_eq!((lines[0].old_no, lines[0].new_no), (Some(1), Some(1)));
        assert_eq!(lines[1].kind, LineKind::Removed);
        assert_eq!((lines[1].old_no, lines[1].new_no), (Some(2), None));
        assert_eq!(lines[2].kind, LineKind::Added);
        assert_eq!((lines[2].old_no, lines[2].new_no), (None, Some(2)));
        assert_eq!((lines[3].old_no, lines[3].new_no), (Some(3), Some(3)));
        assert_eq!(lines[2].text, "DOIS");
    }

    #[test]
    fn detects_new_deleted_binary_and_no_newline() {
        let new = "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+x\n\\ No newline at end of file\n";
        let p = parse_unified_diff(new.as_bytes());
        assert!(p.is_new && !p.is_deleted);
        assert_eq!(p.hunks[0].lines[1].kind, LineKind::NoNewline);

        let del = "diff --git a/n b/n\ndeleted file mode 100644\n--- a/n\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n";
        assert!(parse_unified_diff(del.as_bytes()).is_deleted);

        let bin = "diff --git a/i.png b/i.png\nnew file mode 100644\nBinary files /dev/null and b/i.png differ\n";
        let p = parse_unified_diff(bin.as_bytes());
        assert!(p.binary && p.hunks.is_empty());
    }

    #[test]
    fn parses_combined_conflict_diff() {
        let diff = "diff --cc f.txt\nindex 1,2..3\n--- a/f.txt\n+++ b/f.txt\n@@@ -1,3 -1,3 +1,7 @@@\n  base\n++<<<<<<< HEAD\n +ours\n++=======\n+ theirs\n++>>>>>>> other\n";
        let p = parse_unified_diff(diff.as_bytes());
        assert_eq!(p.hunks.len(), 1);
        let lines = &p.hunks[0].lines;
        assert_eq!(lines[0].kind, LineKind::Context);
        assert_eq!(lines[0].text, "base");
        assert_eq!(lines[1].kind, LineKind::Added);
        assert_eq!(lines[1].text, "<<<<<<< HEAD");
        assert!(lines.iter().all(|l| l.old_no.is_none()));
        assert_eq!(p.additions, 5);
    }

    #[test]
    fn truncates_huge_lines() {
        let long = "x".repeat(MAX_LINE_CHARS + 500);
        let diff = format!("@@ -0,0 +1 @@\n+{long}\n");
        let p = parse_unified_diff(diff.as_bytes());
        let text = &p.hunks[0].lines[0].text;
        assert!(text.chars().count() <= MAX_LINE_CHARS + 1);
        assert!(text.ends_with('…'));
    }
}
