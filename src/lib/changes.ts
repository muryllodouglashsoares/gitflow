import type {
  BranchInfo,
  ChangeKind,
  DiffTarget,
  FileEntry,
  Operation,
  RepositoryStatus,
} from "../types/git";

export type BadgeTone = "added" | "modified" | "deleted" | "renamed" | "untracked" | "conflict";

export interface Badge {
  /** Letra curta exibida no selo. */
  symbol: string;
  /** Texto completo para leitores de tela e dica. */
  label: string;
  tone: BadgeTone;
}

export type GroupId = "conflicted" | "staged" | "unstaged" | "untracked";

export interface ChangeRow {
  /** Chave única na lista (um arquivo pode aparecer em mais de um grupo). */
  key: string;
  path: string;
  origPath: string | null;
  target: DiffTarget;
  badge: Badge;
}

export interface ChangeGroup {
  id: GroupId;
  title: string;
  rows: ChangeRow[];
}

/** Identifica o que está selecionado para exibir o diff. */
export interface Selection {
  path: string;
  origPath: string | null;
  target: DiffTarget;
}

const KIND_BADGES: Record<ChangeKind, Badge> = {
  added: { symbol: "A", label: "Adicionado", tone: "added" },
  modified: { symbol: "M", label: "Modificado", tone: "modified" },
  deleted: { symbol: "D", label: "Removido", tone: "deleted" },
  renamed: { symbol: "R", label: "Renomeado", tone: "renamed" },
  copied: { symbol: "C", label: "Copiado", tone: "renamed" },
  typeChanged: { symbol: "T", label: "Tipo alterado", tone: "modified" },
};

const UNTRACKED_BADGE: Badge = { symbol: "U", label: "Não rastreado", tone: "untracked" };
const CONFLICT_BADGE: Badge = { symbol: "!", label: "Em conflito", tone: "conflict" };

export const GROUP_TITLES: Record<GroupId, string> = {
  conflicted: "Conflitos",
  staged: "Preparadas (staged)",
  unstaged: "Não preparadas",
  untracked: "Não rastreados",
};

export function selectionKey(selection: Pick<Selection, "path" | "target">): string {
  return `${selection.target}:${selection.path}`;
}

/**
 * Organiza os arquivos em grupos. Um arquivo com alteração preparada E não preparada
 * aparece nos dois grupos, como no Git, porque cada área tem um diff diferente.
 */
export function groupChanges(files: readonly FileEntry[]): ChangeGroup[] {
  const groups: Record<GroupId, ChangeRow[]> = {
    conflicted: [],
    staged: [],
    unstaged: [],
    untracked: [],
  };

  for (const file of files) {
    const base = { path: file.path, origPath: file.origPath };
    if (file.conflicted) {
      groups.conflicted.push({
        ...base,
        key: selectionKey({ path: file.path, target: "conflicted" }),
        target: "conflicted",
        badge: CONFLICT_BADGE,
      });
      continue;
    }
    if (file.untracked) {
      groups.untracked.push({
        ...base,
        key: selectionKey({ path: file.path, target: "untracked" }),
        target: "untracked",
        badge: UNTRACKED_BADGE,
      });
      continue;
    }
    if (file.index) {
      groups.staged.push({
        ...base,
        key: selectionKey({ path: file.path, target: "staged" }),
        target: "staged",
        badge: KIND_BADGES[file.index],
      });
    }
    if (file.worktree) {
      groups.unstaged.push({
        ...base,
        // No diff do diretório de trabalho o caminho original não se aplica.
        origPath: null,
        key: selectionKey({ path: file.path, target: "unstaged" }),
        target: "unstaged",
        badge: KIND_BADGES[file.worktree],
      });
    }
  }

  const order: GroupId[] = ["conflicted", "staged", "unstaged", "untracked"];
  return order
    .filter((id) => groups[id].length > 0)
    .map((id) => ({ id, title: GROUP_TITLES[id], rows: groups[id] }));
}

/** `true` se a seleção ainda corresponde a uma linha existente na lista atual. */
export function selectionExists(
  files: readonly FileEntry[],
  selection: Selection | null,
): boolean {
  if (!selection) return false;
  return groupChanges(files).some((group) =>
    group.rows.some((row) => row.target === selection.target && row.path === selection.path),
  );
}

export function splitPath(path: string): { dir: string; name: string } {
  const trimmed = path.endsWith("/") ? path.slice(0, -1) : path;
  const index = trimmed.lastIndexOf("/");
  const suffix = path.endsWith("/") ? "/" : "";
  if (index === -1) return { dir: "", name: trimmed + suffix };
  return { dir: trimmed.slice(0, index + 1), name: trimmed.slice(index + 1) + suffix };
}

export function shortOid(oid: string | null): string | null {
  return oid ? oid.slice(0, 7) : null;
}

/** Texto principal da branch exibido no cabeçalho. */
export function describeBranch(branch: BranchInfo): string {
  if (branch.detached) {
    const oid = shortOid(branch.headOid);
    return oid ? `HEAD desanexado (${oid})` : "HEAD desanexado";
  }
  if (branch.name) return branch.name;
  return "Branch desconhecida";
}

/** Informação complementar da branch (upstream, ahead/behind, sem commits). */
export function describeTracking(branch: BranchInfo): string | null {
  if (branch.unborn) return "Sem commits ainda";
  if (!branch.upstream) return branch.detached ? null : "Sem branch remota associada";
  const parts: string[] = [];
  if (branch.ahead) parts.push(`${branch.ahead} à frente`);
  if (branch.behind) parts.push(`${branch.behind} atrás`);
  const sync = parts.length > 0 ? parts.join(", ") : "em dia com a última busca";
  return `${branch.upstream} · ${sync}`;
}

const OPERATION_LABELS: Record<Operation, string> = {
  merge: "Merge em andamento",
  rebase: "Rebase em andamento",
  cherryPick: "Cherry-pick em andamento",
  revert: "Revert em andamento",
  bisect: "Bisect em andamento",
};

export function operationLabel(operation: Operation): string {
  return OPERATION_LABELS[operation];
}

export type StateTone = "clean" | "dirty" | "warning";

export interface RepoStateSummary {
  tone: StateTone;
  label: string;
}

/** Resumo do estado geral exibido no cabeçalho. */
export function summarizeState(status: RepositoryStatus): RepoStateSummary {
  const { summary, operation } = status;
  if (operation || summary.conflicted > 0) {
    const label =
      summary.conflicted > 0
        ? `${summary.conflicted} ${summary.conflicted === 1 ? "conflito" : "conflitos"}`
        : operationLabel(operation as Operation);
    return { tone: "warning", label: operation && summary.conflicted > 0 ? `${operationLabel(operation)} · ${label}` : label };
  }
  if (status.isClean) return { tone: "clean", label: "Repositório limpo" };
  const n = summary.total;
  return { tone: "dirty", label: `${n} ${n === 1 ? "arquivo alterado" : "arquivos alterados"}` };
}
