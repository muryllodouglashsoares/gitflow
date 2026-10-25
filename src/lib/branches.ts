import type { BranchEntry, BranchList, RepositoryStatus } from "../types/git";
import { operationLabel } from "./changes";

export const MAX_BRANCH_NAME_CHARS = 200;

/** Branch atual primeiro; as demais em ordem alfabética. */
export function sortBranches(branches: readonly BranchEntry[]): BranchEntry[] {
  return [...branches].sort((a, b) => {
    if (a.current !== b.current) return a.current ? -1 : 1;
    return a.name.localeCompare(b.name, "pt-BR");
  });
}

/**
 * Verificação rápida (sem chamar o Git) para dar retorno imediato ao digitar.
 * O backend repete essas regras e valida o nome com o próprio Git: ele é a autoridade.
 */
export function quickBranchNameProblem(name: string): string | null {
  if (name.trim() === "") return "Informe um nome para a branch.";
  if (name !== name.trim()) return "O nome não pode começar nem terminar com espaços.";
  if (/\s/.test(name)) return "O nome não pode conter espaços.";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(name)) return "O nome contém caracteres de controle.";
  if (name.startsWith("-")) return "O nome não pode começar com hífen.";
  if (name === "HEAD") return "“HEAD” não pode ser usado como nome de branch.";
  if (name.length > MAX_BRANCH_NAME_CHARS) return "O nome é longo demais (máximo de 200 caracteres).";
  return null;
}

/** Branch-base sugerida: a atual, se estiver na lista; senão a primeira. */
export function suggestBase(list: BranchList): string | null {
  if (list.current && list.branches.some((b) => b.name === list.current)) return list.current;
  return list.branches[0]?.name ?? null;
}

export interface SwitchBlock {
  reason: string;
  files: string[];
}

/**
 * Motivo pelo qual a troca de branch deve ser bloqueada agora, ou `null` se é segura.
 * Alterações em arquivos rastreados (preparadas ou não) bloqueiam; arquivos apenas não
 * rastreados não bloqueiam (o Git recusa sozinho qualquer troca que os sobrescreva).
 */
export function switchBlock(status: RepositoryStatus | null): SwitchBlock | null {
  if (!status) return { reason: "Aguarde o carregamento do repositório.", files: [] };
  if (status.operation) {
    return {
      reason: `${operationLabel(status.operation)}: conclua ou cancele essa operação antes de trocar de branch.`,
      files: [],
    };
  }
  if (status.summary.conflicted > 0) {
    return { reason: "Há arquivos em conflito: resolva-os antes de trocar de branch.", files: [] };
  }
  const atRisk = status.files.filter((f) => f.index !== null || f.worktree !== null);
  if (atRisk.length > 0) {
    return {
      reason:
        `Há ${atRisk.length} ${atRisk.length === 1 ? "arquivo" : "arquivos"} com alterações não commitadas. ` +
        "Faça o commit delas, ou guarde-as/descarte-as manualmente (por exemplo, com git stash ou git restore), " +
        "e tente novamente. O aplicativo não faz isso por você.",
      files: atRisk.map((f) => f.path),
    };
  }
  return null;
}

/** Quantos arquivos não rastreados existem (aviso informativo na troca de branch). */
export function untrackedCount(status: RepositoryStatus | null): number {
  return status ? status.files.filter((f) => f.untracked).length : 0;
}
