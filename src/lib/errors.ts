import type { ErrorCode, GitError } from "../types/git";

const KNOWN_CODES: ReadonlySet<string> = new Set<ErrorCode>([
  "GIT_NOT_FOUND",
  "GIT_UNAVAILABLE",
  "INVALID_PATH",
  "PATH_NOT_FOUND",
  "NOT_A_DIRECTORY",
  "PATH_NOT_ACCESSIBLE",
  "NOT_A_REPOSITORY",
  "BARE_REPOSITORY",
  "UNSAFE_REPOSITORY",
  "INVALID_ARGUMENT",
  "COMMAND_FAILED",
  "TIMEOUT",
  "OUTPUT_TOO_LARGE",
  "UNEXPECTED_OUTPUT",
  "OPERATION_BLOCKED",
  "STALE_REVIEW",
  "REPOSITORY_BUSY",
  "AUTH_FAILED",
  "PERMISSION_DENIED",
  "NETWORK_ERROR",
  "TLS_ERROR",
  "REMOTE_NOT_FOUND",
  "REMOTE_BRANCH_MISSING",
  "PUSH_REJECTED",
  "MERGE_CONFLICT",
  "FRONTEND",
]);

const GENERIC_MESSAGE = "Ocorreu um erro inesperado. Tente novamente.";

/**
 * Converte qualquer valor rejeitado em um `GitError` seguro para exibição.
 * O Tauri rejeita com o objeto serializado pelo Rust ({ code, message, details? }),
 * mas strings, `Error` e valores inesperados também são tratados.
 */
export function toGitError(value: unknown): GitError {
  if (typeof value === "string" && value.trim() !== "") {
    return { code: "FRONTEND", message: value };
  }
  if (value instanceof Error) {
    return { code: "FRONTEND", message: value.message || GENERIC_MESSAGE };
  }
  if (typeof value === "object" && value !== null) {
    const record = value as Record<string, unknown>;
    const message = typeof record.message === "string" ? record.message : null;
    if (message) {
      const code =
        typeof record.code === "string" && KNOWN_CODES.has(record.code)
          ? (record.code as ErrorCode)
          : "FRONTEND";
      const details = typeof record.details === "string" ? record.details : undefined;
      return details ? { code, message, details } : { code, message };
    }
  }
  return { code: "FRONTEND", message: GENERIC_MESSAGE };
}

/** Erros que indicam que o problema é o Git em si (e não a pasta escolhida). */
export function isGitInstallationError(error: GitError): boolean {
  return error.code === "GIT_NOT_FOUND" || error.code === "GIT_UNAVAILABLE";
}

/** Falhas que indicam que o remoto não pôde ser consultado/verificado (rede, credenciais, tempo). */
const REMOTE_UNVERIFIABLE: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
  "NETWORK_ERROR",
  "AUTH_FAILED",
  "PERMISSION_DENIED",
  "TLS_ERROR",
  "REMOTE_NOT_FOUND",
  "TIMEOUT",
]);

export function isRemoteUnverifiable(error: GitError): boolean {
  return REMOTE_UNVERIFIABLE.has(error.code);
}
