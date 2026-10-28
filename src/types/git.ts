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
  | "OPERATION_BLOCKED"
  | "STALE_REVIEW"
  | "REPOSITORY_BUSY"
  // Fase 3: sincronização remota
  | "AUTH_FAILED"
  | "PERMISSION_DENIED"
  | "NETWORK_ERROR"
  | "TLS_ERROR"
  | "REMOTE_NOT_FOUND"
  | "REMOTE_BRANCH_MISSING"
  | "PUSH_REJECTED"
  | "MERGE_CONFLICT"
  // Fase 4: importação de ZIP
  | "INVALID_ARCHIVE"
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

/**
 * `head` (Fase 2): conteúdo atual do arquivo comparado ao último commit, isto é, exatamente
 * o que um commit seletivo registraria para o arquivo.
 */
export type DiffTarget = "staged" | "unstaged" | "untracked" | "conflicted" | "head";

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

// ---------------------------------------------------------------------------
// Fase 2: branches locais
// ---------------------------------------------------------------------------

export interface BranchEntry {
  /** Nome sem o prefixo `refs/heads/`. */
  name: string;
  current: boolean;
  oid: string;
  upstream: string | null;
  subject: string | null;
}

export interface BranchList {
  repoPath: string;
  current: string | null;
  detached: boolean;
  /** A branch atual ainda não tem commits (por isso não aparece em `branches`). */
  unborn: boolean;
  headOid: string | null;
  branches: BranchEntry[];
}

export interface BranchNameCheck {
  valid: boolean;
  reason: string | null;
}

export interface CreateBranchRequest {
  repoPath: string;
  name: string;
  base: string;
}

export interface CreatedBranch {
  repoPath: string;
  name: string;
  base: string;
  oid: string;
}

export interface SwitchBranchRequest {
  repoPath: string;
  name: string;
}

export interface SwitchResult {
  repoPath: string;
  previous: string | null;
  current: string;
  headOid: string | null;
}

// ---------------------------------------------------------------------------
// Fase 2: revisão e commit seletivo
// ---------------------------------------------------------------------------

export interface ReviewRequest {
  repoPath: string;
  files: string[];
}

export interface ReviewFile {
  path: string;
  origPath: string | null;
  untracked: boolean;
  hasStaged: boolean;
  hasUnstaged: boolean;
  /** `head` (arquivo rastreado) ou `untracked` (arquivo novo). */
  diffTarget: DiffTarget;
}

export interface CommitReview {
  repoPath: string;
  branch: string;
  headOid: string | null;
  files: ReviewFile[];
  /** Arquivos preparados que não foram selecionados: continuam preparados e fora do commit. */
  preservedStaged: string[];
  /** Identifica o conteúdo revisado; é devolvido em `createCommit`. */
  fingerprint: string;
}

export interface CommitRequest {
  repoPath: string;
  files: string[];
  message: string;
  expectedBranch: string;
  fingerprint: string;
}

export interface CommittedFile {
  /** Letra do Git: A, M, D, R, C, T. */
  status: string;
  path: string;
  origPath: string | null;
}

export interface CommitResult {
  repoPath: string;
  branch: string;
  oid: string;
  shortOid: string;
  subject: string;
  files: CommittedFile[];
  summary: string | null;
}

// ---------------------------------------------------------------------------
// Fase 3: sincronização remota
// ---------------------------------------------------------------------------

export interface RemoteInfo {
  name: string;
  /** URL já higienizada pelo backend (sem usuário/senha). */
  fetchUrl: string | null;
  pushUrl: string | null;
}

export type SyncState =
  | "noRemote"
  | "detachedHead"
  | "unborn"
  | "noUpstream"
  | "upstreamInvalid"
  | "upstreamNotFetched"
  | "synced"
  | "ahead"
  | "behind"
  | "diverged";

export type PullStrategy = "merge" | "rebase" | "fastForwardOnly" | "unspecified";

export interface UpstreamInfo {
  remote: string;
  remoteBranch: string;
  /** Ex.: `origin/main`. */
  display: string;
  trackingRef: string;
  trackingRefExists: boolean;
}

export interface SyncStatus {
  repoPath: string;
  branch: string | null;
  headOid: string | null;
  remotes: RemoteInfo[];
  upstream: UpstreamInfo | null;
  state: SyncState;
  /** Contagens conforme as referências LOCAIS (podem estar desatualizadas). */
  ahead: number | null;
  behind: number | null;
  pullStrategy: PullStrategy;
  /** Segundos Unix da última gravação de FETCH_HEAD (último fetch/pull). */
  lastFetchAt: number | null;
}

/** Destino que o usuário viu e confirmou; o backend recusa se ele mudou. */
export interface SyncTarget {
  branch: string;
  remote: string;
  remoteBranch: string;
}

export interface FetchRequest {
  repoPath: string;
  remote: string;
}

export interface FetchResult {
  repoPath: string;
  remote: string;
  updatedRefs: number;
  output: string;
  sync: SyncStatus;
}

export interface PullRequest {
  repoPath: string;
  target: SyncTarget;
}

export type PullOutcome = "alreadyUpToDate" | "updated";

export interface PullResult {
  repoPath: string;
  target: SyncTarget;
  outcome: PullOutcome;
  mergeCommit: boolean;
  headBefore: string | null;
  headAfter: string | null;
  output: string;
  sync: SyncStatus;
}

export interface PushRequest {
  repoPath: string;
  target: SyncTarget;
  confirmProtectedBranch: boolean;
}

export interface PushResult {
  repoPath: string;
  target: SyncTarget;
  pushedCommits: number;
  localOid: string;
  output: string;
  sync: SyncStatus;
}

// ---------------------------------------------------------------------------
// Fase 4 — importação de ZIP
// ---------------------------------------------------------------------------

export type EntryAction = "add" | "replace" | "identical" | "blocked" | "ignored";

/** Etapa persistida da operação (journal em `.git/gitflow-import/<id>`). */
export type ImportState =
  | "applying"
  | "partial"
  | "applied"
  | "committed"
  | "rolledBack"
  | "finished";

export interface InspectRequest {
  repoPath: string;
  zipPath: string;
  /** Pasta interna do ZIP que corresponde à raiz do projeto (`""` = raiz do ZIP). */
  sourcePrefix: string;
  baseBranch: string;
}

export interface PlanEntry {
  path: string;
  action: EntryAction;
  size: number;
  reason: string | null;
  newSha256: string | null;
  oldSha256: string | null;
  zipIndex: number;
  zipName: string;
}

export interface PlanSummary {
  totalEntries: number;
  directories: number;
  filesInScope: number;
  add: number;
  replace: number;
  identical: number;
  blocked: number;
  ignored: number;
  bytesToWrite: number;
  uncompressedBytes: number;
}

export interface ZipLayout {
  topLevelDirs: string[];
  hasRootFiles: boolean;
  suggestedPrefix: string | null;
  directories: string[];
}

export interface ImportPlan {
  /** Identifica o ZIP, a branch-base e o estado dos arquivos de destino. */
  planId: string;
  repoPath: string;
  baseBranch: string;
  baseOid: string;
  zipPath: string;
  zipSize: number;
  zipSha256: string;
  sourcePrefix: string;
  layout: ZipLayout;
  entries: PlanEntry[];
  summary: PlanSummary;
  /** Condições do repositório que impedem a importação agora. */
  blockers: string[];
  warnings: string[];
  canApply: boolean;
}

export interface PrepareBranchRequest {
  repoPath: string;
  zipPath: string;
  sourcePrefix: string;
  baseBranch: string;
  workBranch: string;
  planId: string;
}

export interface PreparedBranch {
  repoPath: string;
  workBranch: string;
  baseBranch: string;
  baseOid: string;
}

export interface ApplyImportRequest {
  repoPath: string;
  zipPath: string;
  sourcePrefix: string;
  baseBranch: string;
  workBranch: string;
  planId: string;
  /** Arquivos do plano que o usuário optou por NÃO importar. */
  excludedPaths: string[];
  /** Confirmação explícita de que arquivos existentes serão substituídos. */
  confirmOverwrites: boolean;
}

export interface AppliedFile {
  path: string;
  action: EntryAction;
  /** O Git enxerga alteração neste arquivo (se não, ele não poderá ser commitado). */
  visibleInGit: boolean;
}

export interface ImportFailure {
  path: string;
  message: string;
}

export interface ImportResult {
  opId: string;
  /** `applied` (tudo gravado) ou `partial` (a gravação falhou no meio). */
  state: ImportState;
  repoPath: string;
  workBranch: string;
  applied: AppliedFile[];
  failure: ImportFailure | null;
  notAttempted: string[];
  identicalSkipped: number;
  recovery: string | null;
}

export interface ImportOperationRequest {
  repoPath: string;
  opId: string;
}

export interface RollbackResult {
  opId: string;
  state: ImportState;
  restored: string[];
  removed: string[];
  keptModified: string[];
  problems: string[];
}

export interface ImportOperationFile {
  path: string;
  action: EntryAction;
}

export interface ImportOperation {
  opId: string;
  state: ImportState;
  repoPath: string;
  workBranch: string;
  baseBranch: string;
  baseOid: string;
  zipPath: string;
  sourcePrefix: string;
  files: ImportOperationFile[];
  commitOid: string | null;
  createdAt: number;
}

export interface ImportCommitRequest {
  opId: string;
  repoPath: string;
  files: string[];
  message: string;
  expectedBranch: string;
  fingerprint: string;
}

export interface PushImportRequest {
  opId: string;
  repoPath: string;
  target: SyncTarget;
  confirmProtectedBranch: boolean;
}
