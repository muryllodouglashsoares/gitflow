/**
 * Tipos que espelham, 1:1, os modelos serializados pelo crate Rust `gitflow-git`
 * (camelCase). Qualquer mudança lá deve ser refletida aqui.
 */

export type ErrorCode =
  | "GIT_NOT_FOUND"
  | "GIT_UNAVAILABLE"
  | "INVALID_PATH"
  | "PATH_NOT_FOUND"
  | "NOT_A_DIRECTORY"
  | "PATH_NOT_ACCESSIBLE"
  | "NOT_A_REPOSITORY"
  | "BARE_REPOSITORY"
  | "UNSAFE_REPOSITORY"
  | "INVALID_ARGUMENT"
  | "COMMAND_FAILED"
  | "TIMEOUT"
  | "OUTPUT_TOO_LARGE"
  | "UNEXPECTED_OUTPUT"
  /** Erros que nascem no frontend (ex.: executando fora do Tauri). */
  | "FRONTEND";

export interface GitError {
  code: ErrorCode;
  message: string;
  details?: string;
}

export interface GitAvailability {
  available: boolean;
  version: string | null;
  programPath: string | null;
  error: GitError | null;
}

export interface RepositoryInfo {
  name: string;
  /** Raiz do repositório. */
  path: string;
  /** Pasta escolhida pelo usuário (pode ser uma subpasta da raiz). */
  selectedPath: string;
  resolvedFromSubfolder: boolean;
}

export type ChangeKind =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "copied"
  | "typeChanged";

export interface FileEntry {
  path: string;
  origPath: string | null;
  index: ChangeKind | null;
  worktree: ChangeKind | null;
  untracked: boolean;
  conflicted: boolean;
  conflictCode: string | null;
  submodule: boolean;
}

export interface BranchInfo {
  name: string | null;
  detached: boolean;
  unborn: boolean;
  headOid: string | null;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
}

export type Operation = "merge" | "rebase" | "cherryPick" | "revert" | "bisect";

export interface StatusSummary {
  total: number;
  staged: number;
  unstaged: number;
  untracked: number;
  conflicted: number;
}

export interface RepositoryStatus {
  repoPath: string;
  branch: BranchInfo;
  files: FileEntry[];
  summary: StatusSummary;
  isClean: boolean;
  operation: Operation | null;
}

export type DiffTarget = "staged" | "unstaged" | "untracked" | "conflicted";

export interface DiffRequest {
  repoPath: string;
  path: string;
  origPath: string | null;
  target: DiffTarget;
}

export type DiffContent = "text" | "binary" | "empty" | "unsupported";
export type LineKind = "context" | "added" | "removed" | "noNewline";

export interface DiffLine {
  kind: LineKind;
  oldNo: number | null;
  newNo: number | null;
  text: string;
}

export interface Hunk {
  header: string;
  lines: DiffLine[];
}

export interface FileDiff {
  repoPath: string;
  path: string;
  target: DiffTarget;
  content: DiffContent;
  hunks: Hunk[];
  additions: number;
  deletions: number;
  isNew: boolean;
  isDeleted: boolean;
  renamedFrom: string | null;
  truncated: boolean;
  notice: string | null;
}
