//! Disponibilidade do Git e validação/identificação de repositórios.

use crate::{
    error::{ErrorCode, GitError, Result},
    runner::{Git, RunOptions},
};
use serde::Serialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitAvailability {
    pub available: bool,
    pub version: Option<String>,
    pub program_path: Option<String>,
    pub error: Option<GitError>,
}

/// Verifica se o Git está instalado e executável. Nunca falha: o resultado descreve o estado.
pub fn check_availability() -> GitAvailability {
    match Git::locate() {
        Ok(git) => match git.version() {
            Ok(version) => GitAvailability {
                available: true,
                version: Some(version),
                program_path: Some(git.program().to_string_lossy().into_owned()),
                error: None,
            },
            Err(error) => unavailable(error),
        },
        Err(error) => unavailable(error),
    }
}

fn unavailable(error: GitError) -> GitAvailability {
    GitAvailability {
        available: false,
        version: None,
        program_path: None,
        error: Some(error),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryInfo {
    /// Nome da pasta raiz do repositório.
    pub name: String,
    /// Raiz do repositório (diretório de trabalho), sempre canônica.
    pub path: String,
    /// Pasta que o usuário escolheu (pode ser uma subpasta da raiz).
    pub selected_path: String,
    /// `true` quando a pasta escolhida estava dentro de um repositório e a raiz foi resolvida.
    pub resolved_from_subfolder: bool,
}

/// Valida a pasta escolhida e resolve a raiz do repositório.
pub fn open_repository(git: &Git, selected: &str) -> Result<RepositoryInfo> {
    let selected_dir = validate_directory(selected)?;
    let root = resolve_root(git, &selected_dir)?;

    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    Ok(RepositoryInfo {
        name,
        path: root.to_string_lossy().into_owned(),
        selected_path: selected_dir.to_string_lossy().into_owned(),
        resolved_from_subfolder: root != selected_dir,
    })
}

/// Usado pelos comandos de leitura: o caminho recebido da interface precisa ser,
/// exatamente, a raiz de um repositório (e não um caminho arbitrário).
pub(crate) fn require_repository_root(git: &Git, repo_path: &str) -> Result<PathBuf> {
    let dir = validate_directory(repo_path)?;
    let root = resolve_root(git, &dir)?;
    if root != dir {
        return Err(GitError::invalid_argument(
            "O caminho informado não é a raiz de um repositório Git.",
        ));
    }
    Ok(root)
}

/// Garante que o caminho é absoluto, existe, é uma pasta legível e devolve sua forma canônica.
fn validate_directory(input: &str) -> Result<PathBuf> {
    if input.trim().is_empty() {
        return Err(GitError::new(
            ErrorCode::InvalidPath,
            "Nenhuma pasta foi informada.",
        ));
    }
    if input.contains('\0') {
        return Err(GitError::new(
            ErrorCode::InvalidPath,
            "O caminho informado contém caracteres inválidos.",
        ));
    }
    let path = Path::new(input);
    if !path.is_absolute() {
        return Err(GitError::new(
            ErrorCode::InvalidPath,
            "O caminho da pasta deve ser absoluto.",
        ));
    }

    let meta = fs::metadata(path).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => {
            GitError::new(ErrorCode::PathNotFound, "A pasta selecionada não existe.")
        }
        io::ErrorKind::PermissionDenied => GitError::new(
            ErrorCode::PathNotAccessible,
            "Sem permissão para acessar a pasta selecionada.",
        ),
        _ => GitError::new(
            ErrorCode::PathNotAccessible,
            "Não foi possível acessar a pasta selecionada.",
        )
        .with_details(err.to_string()),
    })?;
    if !meta.is_dir() {
        return Err(GitError::new(
            ErrorCode::NotADirectory,
            "O caminho selecionado não é uma pasta.",
        ));
    }
    // Confirma que o conteúdo da pasta pode ser lido.
    fs::read_dir(path).map_err(|err| {
        GitError::new(
            ErrorCode::PathNotAccessible,
            "Sem permissão para ler o conteúdo da pasta selecionada.",
        )
        .with_details(err.to_string())
    })?;

    let canonical = fs::canonicalize(path).map_err(|err| {
        GitError::new(
            ErrorCode::PathNotAccessible,
            "Não foi possível resolver o caminho da pasta selecionada.",
        )
        .with_details(err.to_string())
    })?;
    Ok(strip_verbatim_prefix(canonical))
}

/// Pergunta ao Git qual é a raiz do diretório de trabalho que contém `dir`.
fn resolve_root(git: &Git, dir: &Path) -> Result<PathBuf> {
    let opts = RunOptions {
        timeout: Duration::from_secs(15),
        max_stdout: 64 * 1024,
        ..RunOptions::default()
    };
    let out = git.run(Some(dir), &["rev-parse", "--show-toplevel"], &opts)?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Remove somente a quebra de linha final: nomes de pasta podem terminar com espaço.
    let line = text
        .strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(&text);
    if line.is_empty() {
        return Err(GitError::new(
            ErrorCode::UnexpectedOutput,
            "O Git não informou a raiz do repositório.",
        ));
    }
    let canonical = fs::canonicalize(line).map_err(|err| {
        GitError::new(
            ErrorCode::PathNotAccessible,
            "Não foi possível resolver a raiz do repositório.",
        )
        .with_details(err.to_string())
    })?;
    Ok(strip_verbatim_prefix(canonical))
}

/// No Windows, `canonicalize` devolve `\\?\C:\...`, que nem toda ferramenta aceita.
/// Remove o prefixo mantendo o significado do caminho.
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        // Só remove quando resta uma letra de unidade (`C:\...`).
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return PathBuf::from(rest);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_verbatim_drive_prefix() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\C:\Users\me\repo")),
            PathBuf::from(r"C:\Users\me\repo")
        );
    }

    #[test]
    fn strips_verbatim_unc_prefix() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share\repo")),
            PathBuf::from(r"\\server\share\repo")
        );
    }

    #[test]
    fn keeps_regular_paths() {
        assert_eq!(
            strip_verbatim_prefix(PathBuf::from("/home/me/repo")),
            PathBuf::from("/home/me/repo")
        );
    }

    #[test]
    fn rejects_relative_and_empty_paths() {
        assert_eq!(validate_directory("").unwrap_err().code, ErrorCode::InvalidPath);
        assert_eq!(validate_directory("   ").unwrap_err().code, ErrorCode::InvalidPath);
        assert_eq!(
            validate_directory("relative/dir").unwrap_err().code,
            ErrorCode::InvalidPath
        );
        assert_eq!(
            validate_directory("/tmp/\0evil").unwrap_err().code,
            ErrorCode::InvalidPath
        );
    }
}
