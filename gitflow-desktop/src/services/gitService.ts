/**
 * Única fronteira entre a interface e a camada nativa.
 *
 * Todos os comandos abaixo são explícitos, somente leitura e correspondem 1:1 aos
 * `#[tauri::command]` registrados em `src-tauri/src/commands.rs`. A interface nunca
 * envia comandos ou argumentos de Git: apenas caminhos e identificadores.
 */

import { invoke, isTauri } from "@tauri-apps/api/core";
import { toGitError } from "../lib/errors";
import type {
  DiffRequest,
  FileDiff,
  GitAvailability,
  GitError,
  RepositoryInfo,
  RepositoryStatus,
} from "../types/git";

/** Erro tipado lançado pelo serviço; carrega o `GitError` original. */
export class GitServiceError extends Error {
  readonly gitError: GitError;

  constructor(gitError: GitError) {
    super(gitError.message);
    this.name = "GitServiceError";
    this.gitError = gitError;
  }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new GitServiceError({
      code: "FRONTEND",
      message:
        "Este recurso só funciona dentro do aplicativo desktop. Execute \"npm run tauri:dev\" em vez de abrir a página no navegador.",
    });
  }
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new GitServiceError(toGitError(error));
  }
}

export const gitService = {
  checkGit: () => call<GitAvailability>("check_git"),

  openRepository: (path: string) => call<RepositoryInfo>("open_repository", { path }),

  getStatus: (repoPath: string) =>
    call<RepositoryStatus>("get_repository_status", { repoPath }),

  getFileDiff: (request: DiffRequest) => call<FileDiff>("get_file_diff", { request }),
};

/** Extrai o `GitError` de qualquer coisa lançada por este serviço. */
export function errorFrom(value: unknown): GitError {
  return value instanceof GitServiceError ? value.gitError : toGitError(value);
}
