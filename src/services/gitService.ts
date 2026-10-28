/**
 * Única fronteira entre a interface e a camada nativa.
 *
 * Todos os comandos abaixo são explícitos e correspondem 1:1 aos `#[tauri::command]`
 * registrados em `src-tauri/src/commands.rs`. Leitura: checkGit, openRepository, getStatus,
 * getFileDiff, listBranches, checkBranchName, prepareCommitReview. Escrita local (Fase 2):
 * createBranch, switchBranch, createCommit. Sincronização remota (Fase 3): getSyncStatus (somente
 * leitura, sem rede) e as operações predefinidas fetchRemote, pullBranch e pushBranch. Importação de
 * ZIP (Fase 4): inspectZip (somente leitura), prepareImportBranch, applyImport, rollbackImport,
 * createImportCommit, pushImportBranch, findImportOperation e finishImport. A interface nunca envia
 * comandos ou argumentos de Git: apenas caminhos, nomes de branch, mensagem de commit e
 * identificadores.
 */

import { invoke, isTauri } from "@tauri-apps/api/core";
import { toGitError } from "../lib/errors";
import type {
  ApplyImportRequest,
  BranchList,
  BranchNameCheck,
  CommitRequest,
  CommitResult,
  CommitReview,
  CreateBranchRequest,
  CreatedBranch,
  DiffRequest,
  FetchRequest,
  FetchResult,
  FileDiff,
  GitAvailability,
  GitError,
  ImportCommitRequest,
  ImportOperation,
  ImportOperationRequest,
  ImportPlan,
  ImportResult,
  InspectRequest,
  PrepareBranchRequest,
  PreparedBranch,
  PullRequest,
  PullResult,
  PushImportRequest,
  PushRequest,
  PushResult,
  RepositoryInfo,
  RepositoryStatus,
  ReviewRequest,
  RollbackResult,
  SwitchBranchRequest,
  SwitchResult,
  SyncStatus,
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

  // --- Fase 3: sincronização remota ---
  /** Remotos, upstream e situação (somente leitura; não usa a rede). */
  getSyncStatus: (repoPath: string) => call<SyncStatus>("get_sync_status", { repoPath }),

  /** Atualiza as referências do remoto. Não altera arquivos nem integra commits. */
  fetchRemote: (request: FetchRequest) => call<FetchResult>("fetch_remote", { request }),

  /** Baixa e integra; o backend bloqueia com alterações locais, conflitos ou operação em andamento. */
  pullBranch: (request: PullRequest) => call<PullResult>("pull_branch", { request }),

  /** Envia a branch atual ao seu upstream. Nunca força e nunca cria upstream. */
  pushBranch: (request: PushRequest) => call<PushResult>("push_branch", { request }),

  // --- Fase 4: importação de ZIP ---
  /** Lê o ZIP SEM extrair nada e devolve o plano (somente leitura). */
  inspectZip: (request: InspectRequest) => call<ImportPlan>("inspect_zip", { request }),

  /** Cria a branch de trabalho a partir da base e troca para ela (recalcula o plano antes). */
  prepareImportBranch: (request: PrepareBranchRequest) =>
    call<PreparedBranch>("prepare_import_branch", { request }),

  /** Grava os arquivos do plano confirmado. Nunca exclui arquivos nem faz commit. */
  applyImport: (request: ApplyImportRequest) => call<ImportResult>("apply_import", { request }),

  /** Desfaz uma importação ainda sem commit, preservando edições feitas depois. */
  rollbackImport: (request: ImportOperationRequest) =>
    call<RollbackResult>("rollback_import", { request }),

  /** Commit restrito aos arquivos da importação. Nunca faz push. */
  createImportCommit: (request: ImportCommitRequest) =>
    call<CommitResult>("create_import_commit", { request }),

  /** Envia o commit da importação com o push da Fase 3 (nunca força, nunca cria upstream). */
  pushImportBranch: (request: PushImportRequest) =>
    call<PushResult>("push_import_branch", { request }),

  /** Operação ainda não encerrada neste repositório (retomada). */
  findImportOperation: (repoPath: string) =>
    call<ImportOperation | null>("find_import_operation", { repoPath }),

  /** Encerra a operação (após commit ou reversão) e limpa temporários e backups. */
  finishImport: (request: ImportOperationRequest) =>
    call<ImportOperation>("finish_import", { request }),
};

/** Extrai o `GitError` de qualquer coisa lançada por este serviço. */
export function errorFrom(value: unknown): GitError {
  return value instanceof GitServiceError ? value.gitError : toGitError(value);
}
