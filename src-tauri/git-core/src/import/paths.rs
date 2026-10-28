//! Validação de nomes de entradas de ZIP.
//!
//! O nome de uma entrada é texto não confiável. Esta camada decide, **sem tocar no disco**, se
//! ele pode virar um caminho relativo dentro do repositório. Qualquer dúvida bloqueia a entrada:
//! é melhor recusar um nome legítimo e raro do que escrever fora da raiz.
//!
//! O que é bloqueado (e por quê):
//! - caminhos absolutos, letras de unidade (`C:`) e UNC (`//servidor`): escapariam da raiz;
//! - componentes `..` e `.` e componentes vazios (`a//b`): travessia ou ambiguidade;
//! - barra invertida: separador alternativo no Windows e nome comum em outros sistemas, portanto
//!   ambíguo; o ZIP padrão usa `/`;
//! - caracteres de controle e `< > : " | ? *`: inválidos no Windows (`:` também abriria fluxos
//!   alternativos de dados NTFS);
//! - nomes reservados do Windows (`CON`, `NUL`, `COM1`…, com ou sem extensão), componentes que
//!   terminam em ponto ou espaço (o Windows os apaga em silêncio, gerando nomes diferentes dos
//!   validados);
//! - `.git` (e `git~1`, o apelido curto do NTFS) em qualquer nível: escrever metadados do Git é
//!   um vetor de ataque conhecido; `.gitmodules` também é recusado nesta fase.

use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

/// Tamanho máximo, em bytes, de um caminho relativo dentro do repositório.
pub const MAX_PATH_BYTES: usize = 240;
/// Tamanho máximo, em bytes, de um componente (limite do NTFS/ReFS: 255 unidades UTF-16).
pub const MAX_COMPONENT_BYTES: usize = 255;
/// `MAX_PATH` do Windows, sem o terminador nulo.
pub const WINDOWS_MAX_PATH: usize = 259;

const RESERVED_WINDOWS_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Nome de entrada já validado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPath {
    /// Caminho normalizado, com `/`, sem barra final.
    pub path: String,
    pub is_dir: bool,
}

impl EntryPath {
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.path.split('/')
    }
}

/// Valida o nome bruto de uma entrada. `Err` traz o motivo, em português.
pub fn validate_entry_name(name: &str) -> Result<EntryPath, String> {
    if name.is_empty() {
        return Err("o nome da entrada está vazio".into());
    }
    if name.contains('\0') {
        return Err("o nome contém caractere nulo".into());
    }
    if name.contains('\\') {
        return Err(
            "o nome usa barra invertida (\\) como separador, o que é ambíguo entre sistemas".into(),
        );
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("o nome contém caracteres de controle".into());
    }
    if name.starts_with('/') {
        return Err("caminho absoluto (começa com “/”)".into());
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err("caminho com letra de unidade (por exemplo, “C:”)".into());
    }

    let is_dir = name.ends_with('/');
    let trimmed = if is_dir {
        &name[..name.len() - 1]
    } else {
        name
    };
    if trimmed.is_empty() {
        return Err("o nome da entrada está vazio".into());
    }
    if trimmed.len() > MAX_PATH_BYTES {
        return Err(format!(
            "caminho longo demais ({} bytes; o máximo é {MAX_PATH_BYTES})",
            trimmed.len()
        ));
    }
    for component in trimmed.split('/') {
        check_component(component)?;
    }
    Ok(EntryPath {
        path: trimmed.to_string(),
        is_dir,
    })
}

fn check_component(component: &str) -> Result<(), String> {
    match component {
        "" => return Err("contém um componente vazio (barras duplicadas)".into()),
        "." => return Err("contém o componente “.”".into()),
        ".." => return Err("tenta subir de diretório com “..” (path traversal)".into()),
        _ => {}
    }
    if component.len() > MAX_COMPONENT_BYTES {
        return Err(format!(
            "um componente do caminho tem {} bytes (o máximo é {MAX_COMPONENT_BYTES})",
            component.len()
        ));
    }
    if let Some(bad) = component
        .chars()
        .find(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(format!(
            "o componente “{component}” contém “{bad}”, inválido em nomes de arquivo do Windows"
        ));
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(format!(
            "o componente “{component}” termina em ponto ou espaço, que o Windows remove em silêncio"
        ));
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if RESERVED_WINDOWS_NAMES.contains(&stem.as_str()) {
        return Err(format!("“{component}” é um nome reservado do Windows"));
    }
    let lower = component.to_lowercase();
    if lower == ".git" || lower == "git~1" {
        return Err(format!(
            "“{component}” é um diretório de metadados do Git e não pode ser importado"
        ));
    }
    if lower == ".gitmodules" {
        return Err(
            "“.gitmodules” altera de onde o Git baixa código externo e não é importado por esta ferramenta"
                .into(),
        );
    }
    Ok(())
}

/// Valida o prefixo escolhido pelo usuário (`""` = raiz do ZIP). Devolve-o sem barras nas pontas.
pub fn validate_prefix(prefix: &str) -> Result<String, String> {
    let trimmed = prefix.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    // O prefixo é tratado como um nome de diretório qualquer.
    let validated = validate_entry_name(&format!("{trimmed}/"))?;
    Ok(validated.path)
}

/// Chave de comparação para colisões: Unicode normalizado (NFC) e sem diferença de caixa, que é
/// como o Windows (NTFS, sem distinção de maiúsculas) enxerga dois nomes.
pub fn collision_key(path: &str) -> String {
    path.nfc().collect::<String>().to_lowercase()
}

/// Remove o prefixo e devolve o caminho relativo, se a entrada estiver dentro dele.
pub fn strip_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(path);
    }
    path.strip_prefix(prefix)?.strip_prefix('/')
}

/// `true` se `root_len + "/" + rel_len` ultrapassa o limite de caminho informado.
pub fn exceeds_path_limit(root_len: usize, rel_len: usize, limit: usize) -> bool {
    root_len + 1 + rel_len > limit
}

/// Todos os prefixos de diretório de um caminho (`a/b/c.txt` → `a`, `a/b`).
pub fn parent_prefixes(path: &str) -> Vec<&str> {
    path.match_indices('/').map(|(i, _)| &path[..i]).collect()
}

/// Detecta, entre os nomes informados, os que colidem entre si após a normalização.
/// Devolve, para cada índice envolvido, o nome com o qual colide.
pub fn find_collisions(paths: &[(usize, &str)]) -> HashMap<usize, String> {
    let mut groups: HashMap<String, Vec<(usize, &str)>> = HashMap::new();
    for (index, path) in paths {
        groups
            .entry(collision_key(path))
            .or_default()
            .push((*index, path));
    }
    let mut out = HashMap::new();
    for members in groups.values().filter(|m| m.len() > 1) {
        for (index, path) in members {
            let other = members
                .iter()
                .find(|(i, _)| i != index)
                .map(|(_, p)| (*p).to_string())
                .unwrap_or_default();
            let reason = if *path == other.as_str() {
                format!("entrada duplicada (“{other}” aparece mais de uma vez)")
            } else {
                format!(
                    "colide com “{other}”: nomes equivalentes no Windows (maiúsculas/minúsculas ou Unicode)"
                )
            };
            out.insert(*index, reason);
        }
    }
    out
}

/// Nome para exibição: sem caracteres de controle e com tamanho limitado.
pub fn display_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .take(300)
        .collect();
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(name: &str) -> EntryPath {
        validate_entry_name(name).unwrap_or_else(|e| panic!("“{name}” deveria ser aceito: {e}"))
    }

    fn bad(name: &str) -> String {
        validate_entry_name(name).expect_err(&format!("“{name}” deveria ser recusado"))
    }

    #[test]
    fn accepts_regular_paths_including_hidden_and_unicode() {
        assert_eq!(ok("src/main.rs").path, "src/main.rs");
        assert_eq!(ok(".gitignore").path, ".gitignore");
        assert_eq!(
            ok(".github/workflows/ci.yml").path,
            ".github/workflows/ci.yml"
        );
        assert_eq!(
            ok("docs/relatório final.md").path,
            "docs/relatório final.md"
        );
        assert_eq!(
            ok("pasta com espaço/ação.txt").path,
            "pasta com espaço/ação.txt"
        );
        assert!(ok("src/").is_dir);
        assert_eq!(ok("src/").path, "src");
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert!(bad("../evil.txt").contains(".."));
        assert!(bad("a/../../evil.txt").contains(".."));
        assert!(bad("a/b/..").contains(".."));
        assert!(bad("/etc/passwd").contains("absoluto"));
        assert!(bad("C:/Windows/win.ini").contains("unidade"));
        assert!(bad("c:evil.txt").contains("unidade"));
        assert!(bad("//servidor/share/x").contains("absoluto"));
        assert!(bad("./a.txt").contains("“.”"));
        assert!(bad("a//b.txt").contains("vazio"));
    }

    #[test]
    fn rejects_alternative_separators() {
        assert!(bad("a\\b.txt").contains("barra invertida"));
        assert!(bad("..\\evil.txt").contains("barra invertida"));
        assert!(bad("\\\\server\\share\\x").contains("barra invertida"));
    }

    #[test]
    fn rejects_windows_invalid_and_reserved_names() {
        for name in [
            "a<b.txt",
            "a>b.txt",
            "a:b.txt",
            "a\"b.txt",
            "a|b.txt",
            "a?b.txt",
            "a*b.txt",
            "arquivo.txt:stream",
        ] {
            bad(name);
        }
        for name in [
            "CON",
            "con.txt",
            "src/NUL",
            "aux.tar.gz",
            "COM1",
            "lpt9.log",
            "Prn.",
            "dir/CON /x",
        ] {
            bad(name);
        }
        bad("termina-com-ponto.");
        bad("termina-com-espaco ");
        bad("a\u{0007}b.txt");
        // Nomes parecidos, mas válidos.
        ok("console.txt");
        ok("COM10.txt");
        ok("nul-e-void/x.txt");
    }

    #[test]
    fn rejects_git_metadata() {
        bad(".git/config");
        bad("projeto/.GIT/hooks/pre-commit");
        bad("sub/git~1/config");
        bad(".gitmodules");
        ok(".gitignore");
        ok(".gitattributes");
        ok("src/.github/x.yml");
    }

    #[test]
    fn rejects_overlong_paths_and_components() {
        let long_component = "a".repeat(MAX_COMPONENT_BYTES + 1);
        bad(&format!("{long_component}.txt"));
        let long_path = vec!["abcdefghij"; 30].join("/");
        assert!(long_path.len() > MAX_PATH_BYTES);
        bad(&long_path);
    }

    #[test]
    fn empty_and_nul_are_rejected() {
        bad("");
        bad("/");
        bad("a\0b");
    }

    #[test]
    fn prefix_validation_and_stripping() {
        assert_eq!(validate_prefix("").unwrap(), "");
        assert_eq!(validate_prefix("/proj/").unwrap(), "proj");
        assert_eq!(validate_prefix("proj/src").unwrap(), "proj/src");
        assert!(validate_prefix("../x").is_err());
        assert!(validate_prefix(".git").is_err());

        assert_eq!(strip_prefix("proj/a.txt", "proj"), Some("a.txt"));
        assert_eq!(strip_prefix("proj/src/a.txt", "proj"), Some("src/a.txt"));
        assert_eq!(strip_prefix("projeto/a.txt", "proj"), None);
        assert_eq!(strip_prefix("proj", "proj"), None);
        assert_eq!(strip_prefix("a.txt", ""), Some("a.txt"));
    }

    #[test]
    fn collisions_use_case_and_unicode_normalization() {
        // "é" composto (U+00E9) × "e" + acento combinante (U+0065 U+0301).
        let composed = "caf\u{00E9}.txt";
        let decomposed = "cafe\u{0301}.txt";
        assert_eq!(collision_key(composed), collision_key(decomposed));
        assert_eq!(collision_key("README.md"), collision_key("readme.MD"));

        let found = find_collisions(&[(0, "src/A.txt"), (1, "src/a.txt"), (2, "src/b.txt")]);
        assert!(found.contains_key(&0) && found.contains_key(&1));
        assert!(!found.contains_key(&2));
        assert!(found[&0].contains("equivalentes"));

        let dup = find_collisions(&[(0, "x.txt"), (1, "x.txt")]);
        assert!(dup[&0].contains("duplicada"));
    }

    #[test]
    fn path_limit_math() {
        assert!(!exceeds_path_limit(100, 100, WINDOWS_MAX_PATH));
        assert!(exceeds_path_limit(200, 100, WINDOWS_MAX_PATH));
        assert!(!exceeds_path_limit(100, 158, WINDOWS_MAX_PATH));
        assert!(exceeds_path_limit(100, 159, WINDOWS_MAX_PATH));
    }

    #[test]
    fn parent_prefixes_lists_directories() {
        assert_eq!(parent_prefixes("a/b/c.txt"), vec!["a", "a/b"]);
        assert!(parent_prefixes("c.txt").is_empty());
    }
}
