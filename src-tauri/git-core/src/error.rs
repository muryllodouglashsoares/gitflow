//! Erros da camada Git. São serializáveis para que o frontend receba
//! `{ code, message, details? }` e possa decidir como apresentá-los.

use serde::Serialize;
use std::fmt;

/// Código estável do erro (usado pelo frontend para escolher a apresentação).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Executável do Git não encontrado.
    GitNotFound,
    /// O Git foi encontrado, mas não pôde ser executado.
    GitUnavailable,
    /// Caminho vazio, relativo ou malformado.
    InvalidPath,
    /// A pasta não existe.
    PathNotFound,
    /// O caminho existe, mas não é uma pasta.
    NotADirectory,
    /// A pasta existe, mas não pode ser lida.
    PathNotAccessible,
    /// A pasta não pertence a um repositório Git.
    NotARepository,
    /// Repositório sem diretório de trabalho (bare) ou pasta interna `.git`.
    BareRepository,
    /// O Git recusou o repositório (`safe.directory` / dono diferente).
    UnsafeRepository,
    /// Parâmetro recebido da interface é inválido.
    InvalidArgument,
    /// O Git terminou com erro.
    CommandFailed,
    /// O Git excedeu o tempo limite.
    Timeout,
    /// A saída do Git excedeu o limite permitido.
    OutputTooLarge,
    /// A saída do Git não está no formato esperado.
    UnexpectedOutput,
    /// O estado do repositório não permite executar a operação com segurança.
    OperationBlocked,
    /// O conteúdo (ou a branch) mudou desde a revisão apresentada ao usuário.
    StaleReview,
    /// Outra operação de escrita ainda está em andamento neste repositório.
    RepositoryBusy,
    /// Credenciais ausentes, inválidas ou impossíveis de solicitar (HTTPS ou SSH). Fase 3.
    AuthFailed,
    /// A autenticação funcionou, mas a conta não tem permissão para a operação. Fase 3.
    PermissionDenied,
    /// Falha de rede: sem conexão, DNS, servidor indisponível. Fase 3.
    NetworkError,
    /// Certificado ou conexão TLS recusados. Fase 3.
    TlsError,
    /// O repositório remoto não existe ou não está acessível. Fase 3.
    RemoteNotFound,
    /// A branch remota não existe. Fase 3.
    RemoteBranchMissing,
    /// O remoto rejeitou o envio (divergência, branch protegida, hook). Fase 3.
    PushRejected,
    /// A integração do pull gerou conflitos e o repositório ficou em estado de integração. Fase 3.
    MergeConflict,
    /// O ZIP não é válido, está corrompido ou excede os limites de segurança. Fase 4.
    InvalidArchive,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitError {
    pub code: ErrorCode,
    /// Mensagem em português, pronta para exibição.
    pub message: String,
    /// Informação técnica para diagnóstico (ex.: stderr do Git), limitada em tamanho.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

pub type Result<T> = std::result::Result<T, GitError>;

const MAX_DETAILS_CHARS: usize = 2000;

impl GitError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl AsRef<str>) -> Self {
        let trimmed = details.as_ref().trim();
        if !trimmed.is_empty() {
            self.details = Some(truncate_chars(trimmed, MAX_DETAILS_CHARS));
        }
        self
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    /// Operação recusada porque o estado do repositório não a permite com segurança.
    pub fn blocked(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::OperationBlocked, message)
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GitError {}

pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}
