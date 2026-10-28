//! Fase 4 — importação segura de um ZIP em um repositório.
//!
//! ```text
//! inspect_zip ─► prepare_import_branch ─► apply_import ─► (revisão) ─► create_import_commit
//!                                            │                               │
//!                                     rollback_import                 push_import_branch
//!                                                                      / finish_import
//! ```
//!
//! O ZIP é tratado como entrada **não confiável**:
//! - a inspeção (`inspect_zip`) lê o arquivo sem extrair nada para o repositório e produz um
//!   [`ImportPlan`] com a classificação de cada entrada (nova, substituída, idêntica, bloqueada,
//!   ignorada). Nomes perigosos, duplicados, colisões do Windows, links simbólicos, entradas
//!   especiais e limites de tamanho são tratados aqui (ver [`paths`] e [`ImportLimits`]);
//! - o plano tem um `plan_id` (SHA-256 do ZIP, da branch-base e do estado dos arquivos de
//!   destino). Toda etapa seguinte **recalcula o plano a partir do disco** e recusa se o
//!   identificador mudou: a confirmação do usuário sempre corresponde ao que será executado, e o
//!   backend nunca confia em uma lista de caminhos vinda da interface;
//! - a importação **nunca remove** arquivos que não estão no ZIP, nunca sobrescreve sem
//!   confirmação explícita e nunca escreve fora da raiz do repositório;
//! - o commit só aceita arquivos que pertencem à operação, e o push só aceita o commit criado
//!   por ela (e reutiliza o `push_branch` da Fase 3).

mod apply;
pub mod journal;
pub mod paths;

pub use apply::{
    apply_import, apply_import_with, rollback_import, AppliedFile, ApplyImportRequest,
    ApplyOptions, ImportFailure, ImportResult, RollbackResult,
};
pub use journal::{EntryAction, ImportState};

use crate::{
    branches::{
        create_branch, head_oid, list_branches, switch_branch, CreateBranchRequest,
        SwitchBranchRequest,
    },
    commit::{create_commit, CommitRequest, CommitResult},
    error::{ErrorCode, GitError, Result},
    lock::RepoGuard,
    remote::{get_sync_status, push_branch, PushRequest, PushResult, SyncTarget},
    repository::require_repository_root,
    runner::{Git, RunOptions},
    status::{get_status, RepositoryStatus},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
use zip::ZipArchive;

// ----------------------------------------------------------------------------
// Limites
// ----------------------------------------------------------------------------

/// Limites de segurança da inspeção. Os valores padrão atendem a atualizações de código; podem
/// ser reduzidos nos testes.
#[derive(Debug, Clone)]
pub struct ImportLimits {
    /// Tamanho máximo do arquivo `.zip` em disco.
    pub max_zip_bytes: u64,
    /// Quantidade máxima de entradas (arquivos e pastas).
    pub max_entries: usize,
    /// Tamanho máximo de um arquivo depois de descomprimido.
    pub max_file_bytes: u64,
    /// Soma máxima dos arquivos depois de descomprimidos.
    pub max_total_bytes: u64,
    /// Razão máxima entre tamanho descomprimido e comprimido (proteção contra “ZIP bombs”).
    pub max_ratio: u64,
    /// A razão só é avaliada a partir deste tamanho (arquivos pequenos e repetitivos comprimem muito).
    pub ratio_min_bytes: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        ImportLimits {
            max_zip_bytes: 1024 * 1024 * 1024,
            max_entries: 5_000,
            max_file_bytes: 50 * 1024 * 1024,
            max_total_bytes: 500 * 1024 * 1024,
            max_ratio: 200,
            ratio_min_bytes: 1024 * 1024,
        }
    }
}

const MAX_LAYOUT_DIRS: usize = 300;
const MAX_BLOCKING_FILES_LISTED: usize = 8;
const IGNORE_CHUNK: usize = 500;

// ----------------------------------------------------------------------------
// Modelos
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectRequest {
    pub repo_path: String,
    pub zip_path: String,
    /// Pasta interna do ZIP que corresponde à raiz do projeto (`""` = raiz do ZIP).
    #[serde(default)]
    pub source_prefix: String,
    /// Branch-base escolhida pelo usuário (precisa ser a branch atual).
    pub base_branch: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    /// Caminho relativo à raiz do repositório (para entradas fora do prefixo ou bloqueadas, o
    /// nome original, higienizado para exibição).
    pub path: String,
    pub action: EntryAction,
    /// Tamanho descomprimido, em bytes.
    pub size: u64,
    /// Motivo da classificação, quando relevante (bloqueio, sobrescrita, identidade).
    pub reason: Option<String>,
    pub new_sha256: Option<String>,
    pub old_sha256: Option<String>,
    /// Posição da entrada no ZIP (usada só pelo backend para conferir a extração).
    pub zip_index: usize,
    /// Nome original da entrada no ZIP.
    pub zip_name: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub total_entries: usize,
    pub directories: usize,
    pub files_in_scope: usize,
    pub add: usize,
    pub replace: usize,
    pub identical: usize,
    pub blocked: usize,
    pub ignored: usize,
    /// Bytes que serão gravados (arquivos novos + substituídos).
    pub bytes_to_write: u64,
    /// Soma dos tamanhos descomprimidos de todos os arquivos regulares do ZIP.
    pub uncompressed_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZipLayout {
    /// Pastas de primeiro nível.
    pub top_level_dirs: Vec<String>,
    /// Há arquivos diretamente na raiz do ZIP.
    pub has_root_files: bool,
    /// Pasta única que envolve todo o conteúdo (sugestão; nunca é aplicada sozinha).
    pub suggested_prefix: Option<String>,
    /// Pastas candidatas a raiz do projeto (até 3 níveis).
    pub directories: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    /// Identifica o conteúdo do ZIP, a branch-base e o estado dos arquivos de destino.
    pub plan_id: String,
    pub repo_path: String,
    pub base_branch: String,
    pub base_oid: String,
    pub zip_path: String,
    pub zip_size: u64,
    pub zip_sha256: String,
    pub source_prefix: String,
    pub layout: ZipLayout,
    pub entries: Vec<PlanEntry>,
    pub summary: PlanSummary,
    /// Condições do repositório que impedem a importação agora.
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    /// `true` quando nada impede a execução (sem bloqueios, sem entradas bloqueadas, com algo a gravar).
    pub can_apply: bool,
}

// ----------------------------------------------------------------------------
// Utilidades
// ----------------------------------------------------------------------------

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn invalid_archive(message: impl Into<String>) -> GitError {
    GitError::new(ErrorCode::InvalidArchive, message)
}

fn io_error(what: &str, err: io::Error) -> GitError {
    GitError::new(ErrorCode::PathNotAccessible, what.to_string()).with_details(err.to_string())
}

pub(crate) fn dest_path(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in rel.split('/') {
        path.push(component);
    }
    path
}

// ----------------------------------------------------------------------------
// Abertura do ZIP
// ----------------------------------------------------------------------------

pub(crate) struct OpenedZip {
    pub archive: ZipArchive<File>,
    pub size: u64,
    pub sha256: String,
    /// Nomes que aparecem mais de uma vez no diretório central. O crate `zip` guarda as entradas
    /// em um mapa por nome e colapsa as repetidas em silêncio; por isso elas são lidas aqui,
    /// direto do diretório central, antes que isso aconteça.
    pub duplicate_names: HashSet<String>,
}

fn open_zip(zip_path: &str, limits: &ImportLimits) -> Result<OpenedZip> {
    if zip_path.trim().is_empty() || zip_path.contains('\0') {
        return Err(GitError::new(
            ErrorCode::InvalidPath,
            "Nenhum arquivo ZIP foi informado.",
        ));
    }
    let path = Path::new(zip_path);
    if !path.is_absolute() {
        return Err(GitError::new(
            ErrorCode::InvalidPath,
            "O caminho do arquivo ZIP deve ser absoluto.",
        ));
    }
    let is_zip_extension = path
        .extension()
        .map(|e| e.to_string_lossy().eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if !is_zip_extension {
        return Err(GitError::invalid_argument(
            "Selecione um arquivo com a extensão .zip.",
        ));
    }
    let meta = fs::metadata(path).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => {
            GitError::new(ErrorCode::PathNotFound, "O arquivo ZIP não existe mais.")
        }
        io::ErrorKind::PermissionDenied => GitError::new(
            ErrorCode::PathNotAccessible,
            "Sem permissão para ler o arquivo ZIP.",
        ),
        _ => io_error("Não foi possível acessar o arquivo ZIP.", err),
    })?;
    if !meta.is_file() {
        return Err(GitError::new(
            ErrorCode::InvalidPath,
            "O caminho selecionado não é um arquivo.",
        ));
    }
    if meta.len() == 0 {
        return Err(invalid_archive("O arquivo ZIP está vazio (0 bytes)."));
    }
    if meta.len() > limits.max_zip_bytes {
        return Err(invalid_archive(format!(
            "O arquivo ZIP tem {} MiB e o limite é {} MiB.",
            meta.len() / (1024 * 1024),
            limits.max_zip_bytes / (1024 * 1024)
        )));
    }

    let mut file =
        File::open(path).map_err(|e| io_error("Não foi possível abrir o arquivo ZIP.", e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| io_error("Falha ao ler o arquivo ZIP.", e))?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_error("Falha ao ler o arquivo ZIP.", e))?;

    let names = central_directory_names(&mut file, size)?;
    if names.len() > limits.max_entries {
        return Err(invalid_archive(format!(
            "O ZIP declara {} entradas e o limite é {}.",
            names.len(),
            limits.max_entries
        )));
    }
    let mut seen: HashSet<&[u8]> = HashSet::new();
    let mut duplicate_names: HashSet<String> = HashSet::new();
    for name in &names {
        if !seen.insert(name.as_slice()) {
            duplicate_names.insert(String::from_utf8_lossy(name).into_owned());
        }
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_error("Falha ao ler o arquivo ZIP.", e))?;

    let archive = ZipArchive::new(file).map_err(|e| {
        invalid_archive("O arquivo não é um ZIP válido ou está corrompido.")
            .with_details(e.to_string())
    })?;
    Ok(OpenedZip {
        archive,
        size,
        sha256: hex(&hasher.finalize()),
        duplicate_names,
    })
}

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const EOCD_MIN_LEN: usize = 22;
const CENTRAL_HEADER_FIXED_LEN: usize = 46;
const MAX_CENTRAL_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;

fn le_u16(data: &[u8], at: usize) -> usize {
    u16::from_le_bytes([data[at], data[at + 1]]) as usize
}

fn le_u32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// Lê os nomes (bytes) de **todas** as entradas direto do diretório central, inclusive as
/// repetidas. ZIP64 não é suportado: os limites de tamanho e de quantidade tornam-no
/// desnecessário para pacotes legítimos, e um ZIP64 forjado poderia esconder entradas.
fn central_directory_names(file: &mut File, size: u64) -> Result<Vec<Vec<u8>>> {
    let corrupt = |what: &str| invalid_archive(format!("O arquivo não é um ZIP válido ({what})."));
    let tail_len = size.min((EOCD_MIN_LEN + u16::MAX as usize) as u64);
    file.seek(SeekFrom::Start(size - tail_len))
        .map_err(|e| io_error("Falha ao ler o arquivo ZIP.", e))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)
        .map_err(|e| io_error("Falha ao ler o arquivo ZIP.", e))?;

    let eocd = (0..=tail.len().saturating_sub(EOCD_MIN_LEN))
        .rev()
        .find(|&i| {
            le_u32(&tail, i) == EOCD_SIGNATURE
                && i + EOCD_MIN_LEN + le_u16(&tail, i + 20) == tail.len()
        })
        .ok_or_else(|| corrupt("fim do diretório central não encontrado"))?;
    let total = le_u16(&tail, eocd + 10);
    let cd_size = le_u32(&tail, eocd + 12);
    let cd_offset = le_u32(&tail, eocd + 16);
    if total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
        return Err(invalid_archive(
            "ZIP64 não é suportado. Gere o pacote novamente sem ZIP64 (pacotes dentro dos limites do aplicativo não precisam dele).",
        ));
    }
    let cd_size = cd_size as u64;
    let eocd_abs = size - tail_len + eocd as u64;
    if cd_size > MAX_CENTRAL_DIRECTORY_BYTES || cd_size > eocd_abs {
        return Err(corrupt("diretório central com tamanho inválido"));
    }
    // O diretório central termina exatamente onde começa o registro de fim (sem ZIP64); isso
    // também funciona quando há dados antes do primeiro arquivo.
    file.seek(SeekFrom::Start(eocd_abs - cd_size))
        .map_err(|e| io_error("Falha ao ler o arquivo ZIP.", e))?;
    let mut cd = vec![0u8; cd_size as usize];
    file.read_exact(&mut cd)
        .map_err(|e| io_error("Falha ao ler o arquivo ZIP.", e))?;

    let mut names = Vec::with_capacity(total);
    let mut pos = 0usize;
    for _ in 0..total {
        if pos + CENTRAL_HEADER_FIXED_LEN > cd.len() || le_u32(&cd, pos) != CENTRAL_HEADER_SIGNATURE
        {
            return Err(corrupt("registro do diretório central inválido"));
        }
        let name_len = le_u16(&cd, pos + 28);
        let extra_len = le_u16(&cd, pos + 30);
        let comment_len = le_u16(&cd, pos + 32);
        let name_start = pos + CENTRAL_HEADER_FIXED_LEN;
        let next = name_start + name_len + extra_len + comment_len;
        if next > cd.len() {
            return Err(corrupt("registro do diretório central truncado"));
        }
        names.push(cd[name_start..name_start + name_len].to_vec());
        pos = next;
    }
    Ok(names)
}

// ----------------------------------------------------------------------------
// Varredura de metadados (nada é descomprimido aqui)
// ----------------------------------------------------------------------------

enum ScanKind {
    File(String),
    Dir(String),
    Blocked(String),
}

struct Scanned {
    index: usize,
    raw_name: String,
    kind: ScanKind,
    size: u64,
    compressed: u64,
}

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

fn scan_entries(
    archive: &mut ZipArchive<File>,
    limits: &ImportLimits,
    duplicate_names: &HashSet<String>,
) -> Result<Vec<Scanned>> {
    let count = archive.len();
    if count == 0 {
        return Err(invalid_archive("O ZIP não contém nenhuma entrada."));
    }
    if count > limits.max_entries {
        return Err(invalid_archive(format!(
            "O ZIP tem {count} entradas e o limite é {}.",
            limits.max_entries
        )));
    }

    let mut scanned = Vec::with_capacity(count);
    for index in 0..count {
        let file = archive.by_index_raw(index).map_err(|e| {
            invalid_archive("Uma entrada do ZIP não pôde ser lida: o arquivo está corrompido.")
                .with_details(e.to_string())
        })?;
        let raw_name = file.name().to_string();
        let size = file.size();
        let compressed = file.compressed_size();
        let mode = file.unix_mode();
        let encrypted = file.encrypted();
        drop(file);

        let kind = classify_entry(&raw_name, size, compressed, mode, encrypted, limits);
        scanned.push(Scanned {
            index,
            raw_name,
            kind,
            size,
            compressed,
        });
    }
    mark_conflicts(&mut scanned);
    let mut matched = false;
    for s in scanned.iter_mut() {
        if duplicate_names.contains(&s.raw_name) {
            matched = true;
            s.kind = ScanKind::Blocked(
                "Entrada duplicada: o mesmo nome aparece mais de uma vez no ZIP e o conteúdo seria ambíguo.".into(),
            );
        }
    }
    if !duplicate_names.is_empty() && !matched {
        // Nome repetido que não pôde ser associado a uma entrada (codificação incomum).
        return Err(invalid_archive(
            "O ZIP contém entradas com o mesmo nome. Gere o pacote novamente sem duplicatas.",
        ));
    }
    Ok(scanned)
}

fn classify_entry(
    name: &str,
    size: u64,
    compressed: u64,
    mode: Option<u32>,
    encrypted: bool,
    limits: &ImportLimits,
) -> ScanKind {
    let entry = match paths::validate_entry_name(name) {
        Ok(entry) => entry,
        Err(reason) => return ScanKind::Blocked(capitalize(&reason)),
    };
    if encrypted {
        return ScanKind::Blocked("Entrada protegida por senha: não é suportada.".into());
    }
    let file_type = mode.map(|m| m & S_IFMT);
    match file_type {
        Some(S_IFLNK) => {
            return ScanKind::Blocked(
                "Link simbólico: não é importado por segurança (poderia apontar para fora do repositório).".into(),
            )
        }
        Some(t) if t != 0 && t != S_IFREG && t != S_IFDIR => {
            return ScanKind::Blocked(
                "Entrada especial (dispositivo, FIFO, socket…): não é suportada.".into(),
            )
        }
        _ => {}
    }
    if entry.is_dir {
        return ScanKind::Dir(entry.path);
    }
    if file_type == Some(S_IFDIR) {
        return ScanKind::Blocked("Marcada como pasta, mas o nome não termina em “/”.".into());
    }
    if size > limits.max_file_bytes {
        return ScanKind::Blocked(format!(
            "Arquivo grande demais ({} MiB declarados; o limite é {} MiB).",
            size / (1024 * 1024),
            limits.max_file_bytes / (1024 * 1024)
        ));
    }
    if ratio_exceeded(size, compressed, limits) {
        return ScanKind::Blocked(format!(
            "Taxa de compressão suspeita (acima de {}:1): possível “ZIP bomb”.",
            limits.max_ratio
        ));
    }
    ScanKind::File(entry.path)
}

fn ratio_exceeded(size: u64, compressed: u64, limits: &ImportLimits) -> bool {
    size >= limits.ratio_min_bytes && size / compressed.max(1) > limits.max_ratio
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!(
            "{}{}.",
            first.to_uppercase(),
            chars.as_str().trim_end_matches('.')
        ),
        None => String::new(),
    }
}

/// Duplicatas, colisões de nome e arquivo × pasta.
fn mark_conflicts(scanned: &mut [Scanned]) {
    let files: Vec<(usize, String)> = scanned
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match &s.kind {
            ScanKind::File(path) => Some((i, path.clone())),
            _ => None,
        })
        .collect();
    let refs: Vec<(usize, &str)> = files.iter().map(|(i, p)| (*i, p.as_str())).collect();
    for (i, reason) in paths::find_collisions(&refs) {
        scanned[i].kind = ScanKind::Blocked(capitalize(&reason));
    }

    // Um nome não pode ser, ao mesmo tempo, arquivo e pasta.
    let mut dir_keys: HashSet<String> = HashSet::new();
    for s in scanned.iter() {
        match &s.kind {
            ScanKind::Dir(path) => {
                dir_keys.insert(paths::collision_key(path));
            }
            ScanKind::File(path) => {
                for prefix in paths::parent_prefixes(path) {
                    dir_keys.insert(paths::collision_key(prefix));
                }
            }
            ScanKind::Blocked(_) => {}
        }
    }
    for s in scanned.iter_mut() {
        if let ScanKind::File(path) = &s.kind {
            if dir_keys.contains(&paths::collision_key(path)) {
                let reason = format!("“{path}” é, ao mesmo tempo, um arquivo e uma pasta no ZIP.");
                s.kind = ScanKind::Blocked(reason);
            }
        }
    }
}

fn compute_layout(scanned: &[Scanned]) -> ZipLayout {
    let mut top: BTreeSet<String> = BTreeSet::new();
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut has_root_files = false;
    for s in scanned {
        let (path, is_dir) = match &s.kind {
            ScanKind::File(p) => (p.as_str(), false),
            ScanKind::Dir(p) => (p.as_str(), true),
            ScanKind::Blocked(_) => continue,
        };
        match path.split_once('/') {
            Some((first, _)) => {
                top.insert(first.to_string());
            }
            None if is_dir => {
                top.insert(path.to_string());
            }
            None => has_root_files = true,
        }
        let mut candidates: Vec<&str> = paths::parent_prefixes(path);
        if is_dir {
            candidates.push(path);
        }
        for candidate in candidates {
            if candidate.matches('/').count() < 3 {
                dirs.insert(candidate.to_string());
            }
        }
    }
    let suggested_prefix = if !has_root_files && top.len() == 1 {
        top.iter().next().cloned()
    } else {
        None
    };
    ZipLayout {
        top_level_dirs: top.into_iter().collect(),
        has_root_files,
        suggested_prefix,
        directories: dirs.into_iter().take(MAX_LAYOUT_DIRS).collect(),
    }
}

// ----------------------------------------------------------------------------
// Estado do repositório
// ----------------------------------------------------------------------------

/// Oid da branch local informada; recusa se ela não existe.
pub(crate) fn base_branch_oid(git: &Git, repo_path: &str, base: &str) -> Result<String> {
    let list = list_branches(git, repo_path)?;
    list.branches
        .iter()
        .find(|b| b.name == base)
        .map(|b| b.oid.clone())
        .ok_or_else(|| {
            GitError::invalid_argument(
                "A branch-base escolhida não existe entre as branches locais.",
            )
        })
}

/// Condições que impedem a importação (vazio = o repositório está apto). `expected` é a branch
/// em que o usuário precisa estar agora e `role` a descreve nas mensagens.
pub(crate) fn repo_blockers(status: &RepositoryStatus, expected: &str, role: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(operation) = status.operation {
        out.push(format!(
            "Há um {} em andamento. Conclua ou cancele essa operação antes de importar.",
            crate::branches::operation_name(operation)
        ));
    }
    if status.summary.conflicted > 0 {
        out.push("Há arquivos em conflito. Resolva os conflitos antes de importar.".into());
    }
    if status.branch.unborn {
        out.push(
            "Este repositório ainda não tem commits. Crie o primeiro commit antes de importar."
                .into(),
        );
    }
    if status.branch.detached {
        out.push(
            "O HEAD está desanexado (fora de qualquer branch). Volte para uma branch antes de importar.".into(),
        );
    } else if let Some(current) = status.branch.name.as_deref() {
        if current != expected {
            out.push(format!(
                "A branch atual é “{current}”, mas a {role} é “{expected}”. Troque para “{expected}” \
                 (aba Branches) e inspecione novamente."
            ));
        }
    }
    if !status.files.is_empty() {
        let listed: Vec<&str> = status
            .files
            .iter()
            .take(MAX_BLOCKING_FILES_LISTED)
            .map(|f| f.path.as_str())
            .collect();
        let more = status.files.len().saturating_sub(listed.len());
        let mut text = listed.join(", ");
        if more > 0 {
            text.push_str(&format!(" e mais {more}"));
        }
        out.push(format!(
            "Há {} arquivo(s) com alterações locais ({text}). A importação não mistura alterações \
             preexistentes: faça o commit delas, guarde-as ou descarte-as antes (nada é apagado por este aplicativo).",
            status.files.len()
        ));
    }
    out
}

fn tracked_names(git: &Git, root: &Path) -> Result<HashMap<String, String>> {
    let opts = RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 32 * 1024 * 1024,
        ..RunOptions::default()
    };
    let out = git.run(Some(root), &["ls-files", "-z"], &opts)?;
    if out.stdout_truncated {
        return Err(GitError::new(
            ErrorCode::OutputTooLarge,
            "O repositório tem arquivos demais para conferir colisões de nomes com segurança.",
        ));
    }
    let mut map = HashMap::new();
    for raw in out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let path = String::from_utf8_lossy(raw).into_owned();
        for prefix in paths::parent_prefixes(&path) {
            map.entry(paths::collision_key(prefix))
                .or_insert_with(|| prefix.to_string());
        }
        map.insert(paths::collision_key(&path), path);
    }
    Ok(map)
}

fn tracked_case_conflict(tracked: &HashMap<String, String>, rel: &str) -> Option<String> {
    let mut candidates: Vec<&str> = paths::parent_prefixes(rel);
    candidates.push(rel);
    for prefix in candidates {
        if let Some(existing) = tracked.get(&paths::collision_key(prefix)) {
            if existing != prefix {
                return Some(format!(
                    "Difere apenas em maiúsculas/minúsculas (ou Unicode) de “{existing}”, que já existe no repositório."
                ));
            }
        }
    }
    None
}

enum Dest {
    New,
    Existing,
    Blocked(String),
}

/// Examina o destino sem seguir links simbólicos: nenhum componente existente pode ser um link
/// ou um arquivo no lugar de uma pasta.
fn inspect_destination(root: &Path, rel: &str) -> Dest {
    let components: Vec<&str> = rel.split('/').collect();
    let mut current = root.to_path_buf();
    for (i, component) in components.iter().enumerate() {
        current.push(component);
        let last = i + 1 == components.len();
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Dest::Blocked(
                    "O destino passa por um link simbólico, que poderia levar a escrita para fora do repositório.".into(),
                )
            }
            Ok(meta) if last && meta.is_file() => return Dest::Existing,
            Ok(meta) if last && meta.is_dir() => {
                return Dest::Blocked("Já existe uma pasta com este nome no repositório.".into())
            }
            Ok(_) if last => {
                return Dest::Blocked("O destino existe, mas não é um arquivo comum.".into())
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Dest::Blocked(
                    "Existe um arquivo no repositório onde o ZIP precisa de uma pasta.".into(),
                )
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Dest::New,
            Err(err) => {
                return Dest::Blocked(format!("Não foi possível examinar o destino: {err}."))
            }
        }
    }
    Dest::New
}

// ----------------------------------------------------------------------------
// Hash de uma entrada (descompressão controlada, sem gravar nada)
// ----------------------------------------------------------------------------

enum EntryHashError {
    Entry(String),
    TotalExceeded,
}

fn hash_entry(
    archive: &mut ZipArchive<File>,
    scanned: &Scanned,
    limits: &ImportLimits,
    total: &mut u64,
) -> std::result::Result<(String, u64), EntryHashError> {
    let mut file = archive.by_index(scanned.index).map_err(|e| {
        EntryHashError::Entry(format!(
            "Não foi possível ler a entrada (compressão não suportada ou dados corrompidos): {e}."
        ))
    })?;
    if file.name() != scanned.raw_name {
        return Err(EntryHashError::Entry(
            "O ZIP mudou durante a leitura (nome da entrada diferente do inspecionado).".into(),
        ));
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|e| {
            EntryHashError::Entry(format!(
                "Conteúdo corrompido (falha ao descomprimir ou na verificação CRC): {e}."
            ))
        })?;
        if n == 0 {
            break;
        }
        len += n as u64;
        if len > limits.max_file_bytes {
            return Err(EntryHashError::Entry(format!(
                "O conteúdo descomprimido passa de {} MiB, ao contrário do que o cabeçalho declara.",
                limits.max_file_bytes / (1024 * 1024)
            )));
        }
        *total += n as u64;
        if *total > limits.max_total_bytes {
            return Err(EntryHashError::TotalExceeded);
        }
        hasher.update(&buf[..n]);
    }
    if ratio_exceeded(len, scanned.compressed, limits) {
        return Err(EntryHashError::Entry(format!(
            "Taxa de compressão suspeita (acima de {}:1): possível “ZIP bomb”.",
            limits.max_ratio
        )));
    }
    Ok((hex(&hasher.finalize()), len))
}

// ----------------------------------------------------------------------------
// Construção do plano
// ----------------------------------------------------------------------------

pub(crate) struct PlanParams<'a> {
    pub root: &'a Path,
    pub zip_path: &'a str,
    pub source_prefix: &'a str,
    pub base_branch: &'a str,
    pub base_oid: &'a str,
}

/// Lê o ZIP e o repositório e monta o plano. **Não grava nada** no repositório.
pub(crate) fn build_plan(
    git: &Git,
    params: &PlanParams<'_>,
    limits: &ImportLimits,
) -> Result<(ImportPlan, ZipArchive<File>)> {
    let prefix = paths::validate_prefix(params.source_prefix).map_err(|reason| {
        GitError::invalid_argument(format!(
            "O diretório de origem informado é inválido: {reason}."
        ))
    })?;
    let OpenedZip {
        mut archive,
        size: zip_size,
        sha256: zip_sha256,
        duplicate_names,
    } = open_zip(params.zip_path, limits)?;

    let scanned = scan_entries(&mut archive, limits, &duplicate_names)?;
    let declared_total: u64 = scanned
        .iter()
        .filter(|s| matches!(s.kind, ScanKind::File(_)))
        .map(|s| s.size)
        .sum();
    if declared_total > limits.max_total_bytes {
        return Err(invalid_archive(format!(
            "O conteúdo descomprimido declarado é de {} MiB e o limite é {} MiB.",
            declared_total / (1024 * 1024),
            limits.max_total_bytes / (1024 * 1024)
        )));
    }

    let layout = compute_layout(&scanned);
    let tracked = tracked_names(git, params.root)?;
    let root_len = params.root.to_string_lossy().chars().count();

    let mut warnings: Vec<String> = Vec::new();
    if prefix.is_empty() {
        if let Some(suggested) = &layout.suggested_prefix {
            warnings.push(format!(
                "Todo o conteúdo do ZIP está dentro da pasta “{suggested}”. Se essa pasta é a raiz do \
                 projeto, selecione-a como diretório de origem; do contrário os arquivos serão criados \
                 em “{suggested}/” dentro do repositório."
            ));
        }
    } else if !layout.directories.iter().any(|d| d == &prefix)
        && !prefix_has_entries(&scanned, &prefix)
    {
        warnings.push(format!("Nenhuma entrada do ZIP está dentro de “{prefix}”."));
    }

    let mut entries: Vec<PlanEntry> = Vec::new();
    let mut summary = PlanSummary {
        total_entries: scanned.len(),
        ..PlanSummary::default()
    };
    let mut total_read = 0u64;

    for s in &scanned {
        summary.uncompressed_bytes += if matches!(s.kind, ScanKind::File(_)) {
            s.size
        } else {
            0
        };
        match &s.kind {
            ScanKind::Dir(_) => summary.directories += 1,
            ScanKind::Blocked(reason) => entries.push(PlanEntry {
                path: paths::display_name(&s.raw_name),
                action: EntryAction::Blocked,
                size: s.size,
                reason: Some(reason.clone()),
                new_sha256: None,
                old_sha256: None,
                zip_index: s.index,
                zip_name: paths::display_name(&s.raw_name),
            }),
            ScanKind::File(path) => {
                let Some(rel) = paths::strip_prefix(path, &prefix) else {
                    entries.push(PlanEntry {
                        path: path.clone(),
                        action: EntryAction::Ignored,
                        size: s.size,
                        reason: Some(if prefix.is_empty() {
                            "Fora do diretório de origem selecionado.".to_string()
                        } else {
                            format!("Fora do diretório de origem selecionado (“{prefix}”).")
                        }),
                        new_sha256: None,
                        old_sha256: None,
                        zip_index: s.index,
                        zip_name: s.raw_name.clone(),
                    });
                    continue;
                };
                entries.push(plan_file_entry(
                    &mut archive,
                    s,
                    rel,
                    params.root,
                    root_len,
                    &tracked,
                    limits,
                    &mut total_read,
                )?);
            }
        }
    }

    mark_ignored_by_gitignore(git, params.root, &mut entries)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.zip_index.cmp(&b.zip_index)));

    for entry in &entries {
        match entry.action {
            EntryAction::Add => {
                summary.add += 1;
                summary.files_in_scope += 1;
                summary.bytes_to_write += entry.size;
            }
            EntryAction::Replace => {
                summary.replace += 1;
                summary.files_in_scope += 1;
                summary.bytes_to_write += entry.size;
            }
            EntryAction::Identical => {
                summary.identical += 1;
                summary.files_in_scope += 1;
            }
            EntryAction::Blocked => summary.blocked += 1,
            EntryAction::Ignored => summary.ignored += 1,
        }
    }

    let plan_id = compute_plan_id(
        &zip_sha256,
        params.root,
        params.base_branch,
        params.base_oid,
        &prefix,
        &entries,
    );
    let can_apply = summary.blocked == 0 && summary.add + summary.replace > 0;
    let plan = ImportPlan {
        plan_id,
        repo_path: params.root.to_string_lossy().into_owned(),
        base_branch: params.base_branch.to_string(),
        base_oid: params.base_oid.to_string(),
        zip_path: params.zip_path.to_string(),
        zip_size,
        zip_sha256,
        source_prefix: prefix,
        layout,
        entries,
        summary,
        blockers: Vec::new(),
        warnings,
        can_apply,
    };
    Ok((plan, archive))
}

fn prefix_has_entries(scanned: &[Scanned], prefix: &str) -> bool {
    scanned.iter().any(|s| match &s.kind {
        ScanKind::File(p) | ScanKind::Dir(p) => {
            p == prefix || paths::strip_prefix(p, prefix).is_some()
        }
        ScanKind::Blocked(_) => false,
    })
}

#[allow(clippy::too_many_arguments)]
fn plan_file_entry(
    archive: &mut ZipArchive<File>,
    s: &Scanned,
    rel: &str,
    root: &Path,
    root_len: usize,
    tracked: &HashMap<String, String>,
    limits: &ImportLimits,
    total_read: &mut u64,
) -> Result<PlanEntry> {
    let blocked = |reason: String| PlanEntry {
        path: rel.to_string(),
        action: EntryAction::Blocked,
        size: s.size,
        reason: Some(reason),
        new_sha256: None,
        old_sha256: None,
        zip_index: s.index,
        zip_name: s.raw_name.clone(),
    };

    if cfg!(windows)
        && paths::exceeds_path_limit(root_len, rel.chars().count(), paths::WINDOWS_MAX_PATH)
    {
        return Ok(blocked(format!(
            "O caminho completo no repositório passaria de {} caracteres, o limite do Windows.",
            paths::WINDOWS_MAX_PATH
        )));
    }
    if let Some(reason) = tracked_case_conflict(tracked, rel) {
        return Ok(blocked(reason));
    }
    let dest = dest_path(root, rel);
    let existing = match inspect_destination(root, rel) {
        Dest::Blocked(reason) => return Ok(blocked(reason)),
        Dest::New => false,
        Dest::Existing => true,
    };

    let (new_sha, len) = match hash_entry(archive, s, limits, total_read) {
        Ok(result) => result,
        Err(EntryHashError::Entry(reason)) => return Ok(blocked(reason)),
        Err(EntryHashError::TotalExceeded) => {
            return Err(invalid_archive(format!(
                "O conteúdo descomprimido real passa de {} MiB, o limite do aplicativo.",
                limits.max_total_bytes / (1024 * 1024)
            )))
        }
    };

    if !existing {
        return Ok(PlanEntry {
            path: rel.to_string(),
            action: EntryAction::Add,
            size: len,
            reason: None,
            new_sha256: Some(new_sha),
            old_sha256: None,
            zip_index: s.index,
            zip_name: s.raw_name.clone(),
        });
    }
    let old_sha = match hash_file(&dest) {
        Ok(sha) => sha,
        Err(err) => {
            return Ok(blocked(format!(
                "Não foi possível ler o arquivo existente: {err}."
            )))
        }
    };
    let identical = old_sha == new_sha;
    Ok(PlanEntry {
        path: rel.to_string(),
        action: if identical {
            EntryAction::Identical
        } else {
            EntryAction::Replace
        },
        size: len,
        reason: Some(if identical {
            "O conteúdo é idêntico ao arquivo existente: nada será alterado.".into()
        } else {
            "O arquivo existente será substituído.".into()
        }),
        new_sha256: Some(new_sha),
        old_sha256: Some(old_sha),
        zip_index: s.index,
        zip_name: s.raw_name.clone(),
    })
}

/// Arquivos ignorados pelo `.gitignore` não apareceriam no Git depois da importação: o plano os
/// bloqueia em vez de gravar algo que não poderia ser revisado nem commitado.
fn mark_ignored_by_gitignore(git: &Git, root: &Path, entries: &mut [PlanEntry]) -> Result<()> {
    let candidates: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e.action, EntryAction::Add | EntryAction::Replace))
        .map(|(i, _)| i)
        .collect();
    let opts = RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 4 * 1024 * 1024,
        ok_exit_codes: &[0, 1],
    };
    for chunk in candidates.chunks(IGNORE_CHUNK) {
        let mut input: Vec<u8> = Vec::new();
        for &i in chunk {
            input.extend_from_slice(entries[i].path.as_bytes());
            input.push(0);
        }
        let raw = git.run_raw(
            Some(root),
            &["check-ignore", "--stdin", "-z"],
            Some(input.as_slice()),
            &opts,
        )?;
        match raw.exit_code {
            Some(0) | Some(1) => {}
            _ => {
                return Err(GitError::new(
                    ErrorCode::CommandFailed,
                    "O Git não conseguiu verificar o .gitignore.",
                )
                .with_details(String::from_utf8_lossy(&raw.stderr)))
            }
        }
        let ignored: HashSet<String> = raw
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        for &i in chunk {
            if ignored.contains(&entries[i].path) {
                entries[i].action = EntryAction::Blocked;
                entries[i].reason = Some(
                    "Ignorado por uma regra do .gitignore: a importação não apareceria no Git."
                        .into(),
                );
                entries[i].new_sha256 = None;
                entries[i].old_sha256 = None;
            }
        }
    }
    Ok(())
}

fn compute_plan_id(
    zip_sha256: &str,
    root: &Path,
    base_branch: &str,
    base_oid: &str,
    prefix: &str,
    entries: &[PlanEntry],
) -> String {
    let mut hasher = Sha256::new();
    let mut line = |text: String| {
        hasher.update(text.as_bytes());
        hasher.update(b"\n");
    };
    line(format!("zip:{zip_sha256}"));
    line(format!("repo:{}", root.to_string_lossy()));
    line(format!("base:{base_branch}:{base_oid}"));
    line(format!("prefix:{prefix}"));
    let mut sorted: Vec<&PlanEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path).then(a.zip_index.cmp(&b.zip_index)));
    for e in sorted {
        line(format!(
            "{}\t{}\t{:?}\t{}\t{}\t{}",
            e.path,
            e.zip_index,
            e.action,
            e.size,
            e.new_sha256.as_deref().unwrap_or("-"),
            e.old_sha256.as_deref().unwrap_or("-")
        ));
    }
    hex(&hasher.finalize())
}

// ----------------------------------------------------------------------------
// Inspeção
// ----------------------------------------------------------------------------

pub fn inspect_zip(git: &Git, request: &InspectRequest) -> Result<ImportPlan> {
    inspect_zip_with(git, request, &ImportLimits::default())
}

pub fn inspect_zip_with(
    git: &Git,
    request: &InspectRequest,
    limits: &ImportLimits,
) -> Result<ImportPlan> {
    let root = require_repository_root(git, &request.repo_path)?;
    let root_text = root.to_string_lossy().into_owned();
    let base_oid = base_branch_oid(git, &root_text, &request.base_branch)?;
    let status = get_status(git, &root_text)?;

    let params = PlanParams {
        root: &root,
        zip_path: &request.zip_path,
        source_prefix: &request.source_prefix,
        base_branch: &request.base_branch,
        base_oid: &base_oid,
    };
    let (mut plan, _archive) = build_plan(git, &params, limits)?;
    plan.blockers = repo_blockers(&status, &request.base_branch, "branch-base");
    plan.can_apply = plan.can_apply && plan.blockers.is_empty();
    Ok(plan)
}

/// Confere que o plano recalculado agora é o que o usuário confirmou.
pub(crate) fn ensure_plan_unchanged(plan: &ImportPlan, confirmed_plan_id: &str) -> Result<()> {
    if plan.plan_id != confirmed_plan_id {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            "O ZIP, a branch-base ou os arquivos de destino mudaram desde a prévia. \
             Nada foi alterado: inspecione o pacote novamente e confirme o novo plano.",
        ));
    }
    if plan.summary.blocked > 0 {
        return Err(GitError::blocked(
            "O pacote contém entradas bloqueadas por segurança. Corrija o ZIP e inspecione novamente.",
        ));
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Branch de trabalho
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareBranchRequest {
    pub repo_path: String,
    pub zip_path: String,
    #[serde(default)]
    pub source_prefix: String,
    pub base_branch: String,
    /// Nome da branch de trabalho, escolhido e confirmado pelo usuário.
    pub work_branch: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedBranch {
    pub repo_path: String,
    pub work_branch: String,
    pub base_branch: String,
    pub base_oid: String,
}

/// Cria a branch de trabalho a partir da branch-base e troca para ela, reutilizando
/// `create_branch` e `switch_branch` da Fase 2. Antes de qualquer alteração, recalcula o plano e
/// confere o estado do repositório. Nunca descarta alterações locais.
pub fn prepare_import_branch(git: &Git, request: &PrepareBranchRequest) -> Result<PreparedBranch> {
    prepare_import_branch_with(git, request, &ImportLimits::default())
}

pub fn prepare_import_branch_with(
    git: &Git,
    request: &PrepareBranchRequest,
    limits: &ImportLimits,
) -> Result<PreparedBranch> {
    let root = require_repository_root(git, &request.repo_path)?;
    let root_text = root.to_string_lossy().into_owned();
    if request.work_branch == request.base_branch {
        return Err(GitError::invalid_argument(
            "A branch de trabalho precisa ser diferente da branch-base.",
        ));
    }
    let base_oid = base_branch_oid(git, &root_text, &request.base_branch)?;
    let status = get_status(git, &root_text)?;
    let blockers = repo_blockers(&status, &request.base_branch, "branch-base");
    if let Some(first) = blockers.first() {
        return Err(GitError::blocked(first.clone()).with_details(blockers.join("\n")));
    }

    let params = PlanParams {
        root: &root,
        zip_path: &request.zip_path,
        source_prefix: &request.source_prefix,
        base_branch: &request.base_branch,
        base_oid: &base_oid,
    };
    let (plan, _archive) = build_plan(git, &params, limits)?;
    ensure_plan_unchanged(&plan, &request.plan_id)?;
    if plan.summary.add + plan.summary.replace == 0 {
        return Err(GitError::blocked(
            "O plano não tem nenhum arquivo para gravar.",
        ));
    }

    create_branch(
        git,
        &CreateBranchRequest {
            repo_path: root_text.clone(),
            name: request.work_branch.clone(),
            base: request.base_branch.clone(),
        },
    )?;
    if let Err(err) = switch_branch(
        git,
        &SwitchBranchRequest {
            repo_path: root_text.clone(),
            name: request.work_branch.clone(),
        },
    ) {
        let mut wrapped = GitError::new(
            err.code,
            format!(
                "A branch “{}” foi criada, mas não foi possível trocar para ela: {} \
                 Nenhum arquivo foi importado.",
                request.work_branch, err.message
            ),
        );
        if let Some(details) = &err.details {
            wrapped = wrapped.with_details(details);
        }
        return Err(wrapped);
    }
    Ok(PreparedBranch {
        repo_path: root_text,
        work_branch: request.work_branch.clone(),
        base_branch: request.base_branch.clone(),
        base_oid,
    })
}

// ----------------------------------------------------------------------------
// Consulta e encerramento da operação
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportOperationFile {
    pub path: String,
    pub action: EntryAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOperation {
    pub op_id: String,
    pub state: ImportState,
    pub repo_path: String,
    pub work_branch: String,
    pub base_branch: String,
    pub base_oid: String,
    pub zip_path: String,
    pub source_prefix: String,
    pub files: Vec<ImportOperationFile>,
    pub commit_oid: Option<String>,
    pub created_at: u64,
}

fn operation_from(journal: &journal::Journal, state: ImportState) -> ImportOperation {
    ImportOperation {
        op_id: journal.op_id.clone(),
        state,
        repo_path: journal.repo_path.clone(),
        work_branch: journal.work_branch.clone(),
        base_branch: journal.base_branch.clone(),
        base_oid: journal.base_oid.clone(),
        zip_path: journal.zip_path.clone(),
        source_prefix: journal.source_prefix.clone(),
        files: journal
            .entries
            .iter()
            .map(|e| ImportOperationFile {
                path: e.path.clone(),
                action: e.action,
            })
            .collect(),
        commit_oid: journal.commit_oid.clone(),
        created_at: journal.created_at,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOperationRequest {
    pub repo_path: String,
    pub op_id: String,
}

pub type FinishImportRequest = ImportOperationRequest;

/// Operação de importação ainda não encerrada neste repositório (permite retomar depois de
/// fechar o aplicativo). Uma gravação interrompida é reportada como `partial`.
pub fn find_import_operation(git: &Git, repo_path: &str) -> Result<Option<ImportOperation>> {
    let root = require_repository_root(git, repo_path)?;
    let git_dir = journal::git_dir(git, &root)?;
    let running = RepoGuard::acquire(&root).is_err();
    for entry in journal::list(&git_dir) {
        if Path::new(&entry.repo_path) != root {
            continue;
        }
        match entry.state {
            ImportState::Finished | ImportState::RolledBack => continue,
            ImportState::Applying if !running => {
                return Ok(Some(operation_from(&entry, ImportState::Partial)))
            }
            state => return Ok(Some(operation_from(&entry, state))),
        }
    }
    Ok(None)
}

/// Encerra a operação e remove os temporários e backups. Só é permitido depois do commit (ou
/// depois de desfazer a importação): enquanto há arquivos aplicados sem commit, é preciso
/// commitar ou desfazer.
pub fn finish_import(git: &Git, request: &FinishImportRequest) -> Result<ImportOperation> {
    let root = require_repository_root(git, &request.repo_path)?;
    let _guard = RepoGuard::acquire(&root)?;
    let git_dir = journal::git_dir(git, &root)?;
    let (dir, mut entry) = journal::load_for(&git_dir, &root, &request.op_id)?;
    match entry.state {
        ImportState::Committed | ImportState::RolledBack | ImportState::Finished => {}
        _ => {
            return Err(GitError::blocked(
                "A importação ainda tem arquivos aplicados sem commit. Crie o commit ou desfaça a importação antes de encerrar.",
            ))
        }
    }
    entry.state = ImportState::Finished;
    journal::save(&dir, &entry)?;
    let _ = fs::remove_dir_all(journal::staged_dir(&dir));
    let _ = fs::remove_dir_all(journal::backup_dir(&dir));
    Ok(operation_from(&entry, ImportState::Finished))
}

// ----------------------------------------------------------------------------
// Commit isolado
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommitRequest {
    pub op_id: String,
    pub repo_path: String,
    pub files: Vec<String>,
    pub message: String,
    pub expected_branch: String,
    pub fingerprint: String,
}

/// Cria o commit da importação reutilizando `create_commit` (Fase 2), com travas adicionais:
/// a operação precisa estar concluída (`applied`), a branch precisa ser a de trabalho e **todo**
/// arquivo selecionado precisa pertencer à importação. Alterações preexistentes ou criadas por
/// fora nunca entram no commit.
pub fn create_import_commit(git: &Git, request: &ImportCommitRequest) -> Result<CommitResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let git_dir = journal::git_dir(git, &root)?;
    let (dir, mut entry) = journal::load_for(&git_dir, &root, &request.op_id)?;

    match entry.state {
        ImportState::Applied => {}
        ImportState::Committed | ImportState::Finished => {
            return Err(GitError::blocked(format!(
                "O commit desta importação já foi criado{}. Não será criado outro.",
                entry
                    .commit_oid
                    .as_deref()
                    .map(|oid| format!(" ({})", &oid[..oid.len().min(7)]))
                    .unwrap_or_default()
            )))
        }
        _ => {
            return Err(GitError::blocked(
                "A importação não foi concluída: não é possível commitar. Desfaça a importação e tente novamente.",
            ))
        }
    }
    if request.expected_branch != entry.work_branch {
        return Err(GitError::new(
            ErrorCode::StaleReview,
            format!(
                "O commit da importação só pode ser criado na branch de trabalho “{}”.",
                entry.work_branch
            ),
        ));
    }
    if let Some(foreign) = request.files.iter().find(|f| !entry.allows(f)) {
        return Err(GitError::blocked(format!(
            "“{foreign}” não faz parte desta importação e não pode entrar no commit dela."
        )));
    }

    let result = create_commit(
        git,
        &CommitRequest {
            repo_path: request.repo_path.clone(),
            files: request.files.clone(),
            message: request.message.clone(),
            expected_branch: request.expected_branch.clone(),
            fingerprint: request.fingerprint.clone(),
        },
    )?;

    // O commit já existe: se o registro não puder ser atualizado, o resultado real do Git é
    // devolvido mesmo assim (uma nova tentativa de commit falharia por não haver alterações).
    entry.state = ImportState::Committed;
    entry.commit_oid = Some(result.oid.clone());
    let _ = journal::save(&dir, &entry);
    Ok(result)
}

// ----------------------------------------------------------------------------
// Push (reutiliza a Fase 3)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushImportRequest {
    pub op_id: String,
    pub repo_path: String,
    pub target: SyncTarget,
    #[serde(default)]
    pub confirm_protected_branch: bool,
}

/// Envia o commit da importação com `push_branch` (Fase 3). Só é aceito depois do commit, para a
/// branch de trabalho e enquanto o HEAD for exatamente o commit criado pela importação. Sem
/// upstream configurado, o `push_branch` recusa: nenhum destino é inventado.
pub fn push_import_branch(git: &Git, request: &PushImportRequest) -> Result<PushResult> {
    let root = require_repository_root(git, &request.repo_path)?;
    let git_dir = journal::git_dir(git, &root)?;
    let (dir, mut entry) = journal::load_for(&git_dir, &root, &request.op_id)?;

    if entry.state != ImportState::Committed {
        return Err(GitError::blocked(
            "O envio só é permitido depois que o commit da importação foi criado.",
        ));
    }
    if request.target.branch != entry.work_branch {
        return Err(GitError::blocked(format!(
            "Só a branch de trabalho “{}” pode ser enviada por este fluxo.",
            entry.work_branch
        )));
    }
    let head = head_oid(git, &root)?;
    let sync = get_sync_status(git, &request.repo_path)?;
    let expected = entry.commit_oid.clone().unwrap_or_default();
    if sync.branch.as_deref() != Some(entry.work_branch.as_str())
        || head.as_deref() != Some(expected.as_str())
    {
        return Err(GitError::blocked(
            "A branch de trabalho não está mais no commit criado pela importação (há outros commits ou ela mudou). \
             Use a aba Sincronização para enviar com revisão própria.",
        ));
    }

    let result = push_branch(
        git,
        &PushRequest {
            repo_path: request.repo_path.clone(),
            target: request.target.clone(),
            confirm_protected_branch: request.confirm_protected_branch,
        },
    )?;

    entry.state = ImportState::Finished;
    if journal::save(&dir, &entry).is_ok() {
        let _ = fs::remove_dir_all(journal::staged_dir(&dir));
        let _ = fs::remove_dir_all(journal::backup_dir(&dir));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_check_only_applies_to_large_entries() {
        let limits = ImportLimits::default();
        assert!(!ratio_exceeded(1000, 1, &limits));
        assert!(ratio_exceeded(2 * 1024 * 1024, 100, &limits));
        assert!(!ratio_exceeded(2 * 1024 * 1024, 100_000, &limits));
        assert!(ratio_exceeded(2 * 1024 * 1024, 0, &limits));
    }

    #[test]
    fn capitalize_adds_period() {
        assert_eq!(capitalize("caminho absoluto"), "Caminho absoluto.");
        assert_eq!(capitalize("já termina."), "Já termina.");
    }
}
