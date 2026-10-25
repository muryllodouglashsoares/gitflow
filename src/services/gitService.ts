/**
 * Única fronteira entre a interface e a camada nativa.
 *
 * Todos os comandos abaixo são explícitos e correspondem 1:1 aos `#[tauri::command]`
 * registrados em `src-tauri/src/commands.rs`. Leitura: checkGit, openRepository, getStatus,
 * getFileDiff, listBranches, checkBranchName, prepareCommitReview. Escrita local (Fase 2):
 * createBranch, switchBranch, createCommit. A interface nunca envia comandos ou argumentos
 * de Git: apenas caminhos, nomes de branch, mensagem de commit e identificadores.
 */

import { invoke, isTauri } from "@tauri-apps/api/core";
import { toGitError } from "../lib/errors";
import type {
  BranchList,
  BranchNameCheck,
  CommitRequest,
  CommitResult,
  CommitReview,
  CreateBranchRequest,
  CreatedBranch,
  DiffRequest,
  FileDiff,
  GitAvailability,
  GitError,
  RepositoryInfo,
  RepositoryStatus,
  ReviewRequest,
  SwitchBranchRequest,
  SwitchResult,
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

  // --- Fase 2: branches ---
  listBranches: (repoPath: string) => call<BranchList>("list_branches", { repoPath }),

  checkBranchName: (repoPath: string, name: string) =>
    call<BranchNameCheck>("check_branch_name", { repoPath, name }),

  /** Cria a branch SEM trocar para ela. */
  createBranch: (request: CreateBranchRequest) => call<CreatedBranch>("create_branch", { request }),

  /** Só troca se não houver alterações locais em risco; nunca força. */
  switchBranch: (request: SwitchBranchRequest) => call<SwitchResult>("switch_branch", { request }),

  // --- Fase 2: commit ---
  prepareCommitReview: (request: ReviewRequest) =>
    call<CommitReview>("prepare_commit_review", { request }),

  /** Cria o commit local (nunca faz push). */
  createCommit: (request: CommitRequest) => call<CommitResult>("create_commit", { request }),
};

/** Extrai o `GitError` de qualquer coisa lançada por este serviço. */
export function errorFrom(value: unknown): GitError {
  return value instanceof GitServiceError ? value.gitError : toGitError(value);
}
