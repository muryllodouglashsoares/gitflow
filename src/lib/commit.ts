import type { FileEntry, RepositoryStatus } from "../types/git";
import { operationLabel } from "./changes";

export const MAX_COMMIT_MESSAGE_CHARS = 8000;
/** Convenção usual para a primeira linha; só gera um aviso, nunca bloqueia. */
export const SUBJECT_SOFT_LIMIT = 72;

export interface Eligibility {
  eligible: boolean;
  /** Por que o arquivo não pode ser selecionado (quando `eligible` é `false`). */
  reason: string | null;
}

/** Arquivos em estados que não podem entrar num commit com segurança nesta fase. */
export function commitEligibility(file: FileEntry): Eligibility {
  if (file.conflicted) {
    return { eligible: false, reason: "Em conflito: resolva o conflito antes de commitar este arquivo." };
  }
  if (file.submodule) {
    return { eligible: false, reason: "Submódulos ainda não são suportados." };
  }
  if (file.path.endsWith("/")) {
    return { eligible: false, reason: "Pasta ou repositório aninhado: ainda não suportado." };
  }
  return { eligible: true, reason: null };
}

export function eligiblePaths(files: readonly FileEntry[]): string[] {
  return files.filter((f) => commitEligibility(f).eligible).map((f) => f.path);
}

/** Mantém somente caminhos que ainda existem na lista e continuam elegíveis. */
export function pruneSelection(
  selected: ReadonlySet<string>,
  files: readonly FileEntry[],
): Set<string> {
  const allowed = new Set(eligiblePaths(files));
  const next = new Set<string>();
  for (const path of selected) {
    if (allowed.has(path)) next.add(path);
  }
  return next;
}

export function toggleInSet(selected: ReadonlySet<string>, path: string): Set<string> {
  const next = new Set(selected);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  return next;
}

/** `true` se existe diferença entre os dois conjuntos (evita renders desnecessários). */
export function sameSet(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return false;
  for (const item of a) {
    if (!b.has(item)) return false;
  }
  return true;
}

/** Mensagem de erro de validação, ou `null` se a mensagem é aceitável. */
export function validateCommitMessage(message: string): string | null {
  if (message.includes("\u0000")) return "A mensagem contém caracteres inválidos.";
  if (message.trim() === "") return "Escreva uma mensagem para o commit.";
  if (message.trim().length > MAX_COMMIT_MESSAGE_CHARS) {
    return `A mensagem é longa demais (máximo de ${MAX_COMMIT_MESSAGE_CHARS} caracteres).`;
  }
  return null;
}

/** Aviso (não bloqueante) quando a primeira linha passa do tamanho usual. */
export function subjectWarning(message: string): string | null {
  const first = message.trim().split(/\r?\n/, 1)[0] ?? "";
  return first.length > SUBJECT_SOFT_LIMIT
    ? `A primeira linha tem ${first.length} caracteres; o usual é até ${SUBJECT_SOFT_LIMIT}.`
    : null;
}

/**
 * Motivo pelo qual NENHUM commit pode ser criado agora (estado do repositório), ou `null`.
 * Espelha as verificações do backend, que continuam sendo a autoridade final.
 */
export function commitBlocker(status: RepositoryStatus | null): string | null {
  if (!status) return "Aguarde o carregamento do repositório.";
  if (status.operation) {
    return `${operationLabel(status.operation)}: conclua ou cancele essa operação antes de commitar por aqui.`;
  }
  if (status.summary.conflicted > 0) {
    return "Há arquivos em conflito: resolva os conflitos antes de commitar.";
  }
  if (status.branch.detached) {
    return "HEAD desanexado: crie uma branch neste ponto pelo Git (git switch -c nome) antes de commitar.";
  }
  return null;
}

export interface FileMarks {
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
  /** Alterações preparadas E não preparadas no mesmo arquivo. */
  partial: boolean;
}

export function fileMarks(file: Pick<FileEntry, "index" | "worktree" | "untracked">): FileMarks {
  const staged = file.index !== null;
  const unstaged = file.worktree !== null;
  return { staged, unstaged, untracked: file.untracked, partial: staged && unstaged };
}

/** Arquivos preparados que NÃO estão na seleção (continuarão preparados e fora do commit). */
export function stagedOutsideSelection(
  files: readonly FileEntry[],
  selected: ReadonlySet<string>,
): string[] {
  return files
    .filter((f) => f.index !== null && !f.conflicted && !selected.has(f.path))
    .map((f) => f.path);
}
