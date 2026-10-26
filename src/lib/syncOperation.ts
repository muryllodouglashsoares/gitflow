/**
 * Estado da operação remota em andamento (fetch, pull ou push), como função pura.
 *
 * Garantias (testadas em `syncOperation.test.ts`):
 * - cliques repetidos: `start` é ignorado enquanto uma operação está em execução;
 * - respostas obsoletas: `succeed`/`fail` só valem se pertencem ao repositório atual E à
 *   operação atual (número de sequência), então a resposta de um repositório ou de uma
 *   tentativa antiga nunca altera a tela;
 * - tempo esgotado vira "não confirmado" (o servidor pode ter concluído o trabalho), nunca
 *   "sucesso" nem simples "falha".
 */

import type { FetchResult, GitError, PullResult, PushResult } from "../types/git";

export type SyncOpKind = "fetch" | "pull" | "push";

export type SyncOpPhase = "idle" | "running" | "success" | "failure" | "unconfirmed";

export type SyncOpResult =
  | { kind: "fetch"; result: FetchResult }
  | { kind: "pull"; result: PullResult }
  | { kind: "push"; result: PushResult };

export interface SyncOpState {
  repoPath: string | null;
  /** Identifica a tentativa atual; respostas de outras tentativas são descartadas. */
  seq: number;
  kind: SyncOpKind | null;
  phase: SyncOpPhase;
  result: SyncOpResult | null;
  error: GitError | null;
}

export const INITIAL_SYNC_OP: SyncOpState = {
  repoPath: null,
  seq: 0,
  kind: null,
  phase: "idle",
  result: null,
  error: null,
};

export type SyncOpAction =
  | { type: "reset"; repoPath: string | null }
  | { type: "start"; repoPath: string; kind: SyncOpKind; seq: number }
  | { type: "succeed"; repoPath: string; seq: number; result: SyncOpResult }
  | { type: "fail"; repoPath: string; seq: number; error: GitError }
  | { type: "dismiss" };

export function syncOpReducer(state: SyncOpState, action: SyncOpAction): SyncOpState {
  switch (action.type) {
    case "reset":
      return { ...INITIAL_SYNC_OP, repoPath: action.repoPath, seq: state.seq };
    case "start":
      if (state.phase === "running") return state; // clique repetido
      if (state.repoPath !== action.repoPath) return state; // repositório já mudou
      return {
        repoPath: action.repoPath,
        seq: action.seq,
        kind: action.kind,
        phase: "running",
        result: null,
        error: null,
      };
    case "succeed":
      if (!isCurrentAttempt(state, action)) return state;
      return { ...state, phase: "success", result: action.result, error: null };
    case "fail":
      if (!isCurrentAttempt(state, action)) return state;
      return {
        ...state,
        phase: action.error.code === "TIMEOUT" ? "unconfirmed" : "failure",
        result: null,
        error: action.error,
      };
    case "dismiss":
      if (state.phase === "running") return state;
      return { ...INITIAL_SYNC_OP, repoPath: state.repoPath, seq: state.seq };
  }
}

function isCurrentAttempt(
  state: SyncOpState,
  action: { repoPath: string; seq: number },
): boolean {
  return (
    state.phase === "running" && state.repoPath === action.repoPath && state.seq === action.seq
  );
}

/** Mensagem de etapa: não há porcentagem real de rede, então o indicador é indeterminado. */
export function stepMessage(kind: SyncOpKind): string {
  switch (kind) {
    case "fetch":
      return "Atualizando referências remotas…";
    case "pull":
      return "Baixando e integrando alterações…";
    case "push":
      return "Enviando commits…";
  }
}
