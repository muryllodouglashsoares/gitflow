/**
 * Fluxo de importação de ZIP (Fase 4) como função pura.
 *
 * O backend é a fronteira de segurança (revalida caminhos, plano e estado do repositório a cada
 * etapa). Este módulo garante o lado da interface, testado em `importFlow.test.ts`:
 *
 * - **etapas explícitas e transições permitidas** (`TRANSITIONS`): não existe um caminho que pule
 *   a revisão do plano, a confirmação do commit ou a confirmação do push; eventos fora de ordem
 *   são ignorados;
 * - **respostas obsoletas**: toda resposta assíncrona carrega o repositório e o número da
 *   tentativa; se o usuário trocou de repositório, cancelou ou recomeçou, ela é descartada;
 * - **cliques repetidos**: ações de uma etapa em andamento são ignoradas;
 * - **registro auditável** (`log`) de cada etapa: início, resultado e detalhe relevante;
 * - **falhas** guardam a etapa em que ocorreram, o que já foi concluído e o que verificar antes
 *   de continuar — nunca avançam silenciosamente.
 */

import type {
  EntryAction,
  GitError,
  ImportOperation,
  ImportPlan,
  ImportResult,
  CommitResult,
  PlanEntry,
  PushResult,
  RepositoryStatus,
  RollbackResult,
  SyncStatus,
  SyncTarget,
} from "../types/git";
import { quickBranchNameProblem } from "./branches";
import { isProtectedDestination, pushBlock, syncTarget } from "./sync";

// ---------------------------------------------------------------------------
// Etapas
// ---------------------------------------------------------------------------

export type ImportStage =
  | "selecting" // Aguardando seleção
  | "inspecting" // Inspecionando ZIP (somente leitura)
  | "zipInvalid" // ZIP inválido (ou inspeção sem sucesso)
  | "planReview" // Aguardando revisão/confirmação do plano
  | "creatingBranch" // Validando repositório e criando a branch de trabalho
  | "importing" // Importando arquivos
  | "partial" // Importação parcialmente concluída (exige desfazer)
  | "rollingBack" // Desfazendo a importação
  | "rolledBack" // Importação desfeita
  | "commitReview" // Importação concluída: revisão do diff e do commit
  | "committing" // Criando commit
  | "committed" // Commit concluído: aguardando confirmação de push
  | "pushing" // Enviando ao remoto
  | "done" // Concluído
  | "failed" // Falha (recuperável ou que exige intervenção manual)
  | "cancelled"; // Cancelado (somente quando realmente ocorreu)

export const STAGE_LABELS: Record<ImportStage, string> = {
  selecting: "Aguardando seleção",
  inspecting: "Inspecionando o ZIP",
  zipInvalid: "ZIP inválido",
  planReview: "Aguardando revisão do plano",
  creatingBranch: "Criando a branch de trabalho",
  importing: "Importando arquivos",
  partial: "Importação parcialmente concluída",
  rollingBack: "Desfazendo a importação",
  rolledBack: "Importação desfeita",
  commitReview: "Aguardando revisão do diff e confirmação do commit",
  committing: "Criando o commit",
  committed: "Commit concluído — aguardando confirmação do push",
  pushing: "Enviando ao remoto",
  done: "Concluído",
  failed: "Falha",
  cancelled: "Cancelado",
};

/** Transições permitidas. Qualquer outra é ignorada pelo reducer. */
export const TRANSITIONS: Readonly<Record<ImportStage, readonly ImportStage[]>> = {
  // `partial`, `commitReview` e `committed` aqui são a retomada de uma operação já gravada no disco.
  selecting: ["inspecting", "partial", "commitReview", "committed"],
  inspecting: ["planReview", "zipInvalid", "cancelled"],
  zipInvalid: ["selecting", "inspecting"],
  planReview: ["inspecting", "creatingBranch", "selecting"],
  creatingBranch: ["importing", "failed"],
  importing: ["commitReview", "partial", "failed"],
  partial: ["rollingBack"],
  rollingBack: ["rolledBack", "partial", "commitReview"],
  rolledBack: ["selecting"],
  commitReview: ["committing", "committed", "rollingBack"],
  committing: ["committed", "commitReview"],
  committed: ["pushing", "done"],
  pushing: ["done", "committed"],
  done: ["selecting"],
  failed: ["selecting"],
  cancelled: ["selecting"],
};

export function canTransition(from: ImportStage, to: ImportStage): boolean {
  return TRANSITIONS[from].includes(to);
}

/** Etapas em que há trabalho em andamento (a interface bloqueia ações incompatíveis). */
export function isRunning(stage: ImportStage): boolean {
  return (
    stage === "inspecting" ||
    stage === "creatingBranch" ||
    stage === "importing" ||
    stage === "rollingBack" ||
    stage === "committing" ||
    stage === "pushing"
  );
}

/** Etapas que alteram o repositório e não podem ser interrompidas com segurança. */
export function isWriting(stage: ImportStage): boolean {
  return isRunning(stage) && stage !== "inspecting";
}

/** O fluxo está ativo (há plano confirmado, arquivos gravados ou commit a publicar). */
export function isEngaged(stage: ImportStage): boolean {
  return (
    stage === "creatingBranch" ||
    stage === "importing" ||
    stage === "partial" ||
    stage === "rollingBack" ||
    stage === "commitReview" ||
    stage === "committing" ||
    stage === "committed" ||
    stage === "pushing"
  );
}

// ---------------------------------------------------------------------------
// Estado
// ---------------------------------------------------------------------------

export interface FlowLogEntry {
  step: string;
  outcome: "started" | "ok" | "failed" | "info";
  detail?: string;
  at: number;
}

export interface FlowFailure {
  /** Etapa em que a falha ocorreu. */
  at: ImportStage;
  error: GitError;
  /** `true` quando é preciso conferir algo manualmente antes de continuar. */
  manual: boolean;
  /** O que já foi concluído antes da falha. */
  completed: string[];
  /** O que o usuário deve verificar/fazer. */
  advice: string[];
}

/** Problema não fatal em uma etapa que permanece disponível (push, desfazer, encerrar). */
export interface FlowIssue {
  kind: "push" | "rollback" | "finish" | "apply";
  error: GitError;
  /** Resultado incerto (tempo esgotado): o que foi pedido pode ter acontecido. */
  unconfirmed: boolean;
}

export interface ImportFlowState {
  /** Repositório a que o fluxo pertence. */
  repoPath: string | null;
  /** Número da tentativa assíncrona atual; respostas de outras tentativas são descartadas. */
  seq: number;
  stage: ImportStage;
  zipPath: string | null;
  sourcePrefix: string;
  baseBranch: string | null;
  workBranch: string;
  plan: ImportPlan | null;
  /** Arquivos do plano que o usuário optou por NÃO importar (ordenados). */
  excluded: readonly string[];
  opId: string | null;
  result: ImportResult | null;
  /** Caminhos importados (resultado da gravação ou da operação retomada). */
  importedPaths: readonly string[];
  commit: CommitResult | null;
  /** OID do commit da importação (também quando a operação foi retomada). */
  commitOid: string | null;
  push: PushResult | null;
  rollback: RollbackResult | null;
  /** Erro da inspeção (etapa `zipInvalid`). */
  inspectError: GitError | null;
  failure: FlowFailure | null;
  issue: FlowIssue | null;
  /** O fluxo foi retomado de uma operação gravada no disco. */
  resumed: boolean;
  /** Etapa a que se volta se "desfazer" falhar (`partial` ou `commitReview`). */
  rollbackFrom: ImportStage | null;
  log: readonly FlowLogEntry[];
}

const MAX_LOG = 200;

export const INITIAL_IMPORT_FLOW: ImportFlowState = {
  repoPath: null,
  seq: 0,
  stage: "selecting",
  zipPath: null,
  sourcePrefix: "",
  baseBranch: null,
  workBranch: "",
  plan: null,
  excluded: [],
  opId: null,
  result: null,
  importedPaths: [],
  commit: null,
  commitOid: null,
  push: null,
  rollback: null,
  inspectError: null,
  failure: null,
  issue: null,
  resumed: false,
  rollbackFrom: null,
  log: [],
};

// ---------------------------------------------------------------------------
// Ações
// ---------------------------------------------------------------------------

type Scoped = { repoPath: string; seq: number; at: number };

export type ImportFlowAction =
  | { type: "reset"; repoPath: string | null }
  // --- seleção ---
  | { type: "chooseZip"; repoPath: string; zipPath: string; baseBranch: string | null; workBranch: string }
  | { type: "setBase"; repoPath: string; baseBranch: string }
  | { type: "setPrefix"; repoPath: string; prefix: string }
  | { type: "setWorkBranch"; repoPath: string; name: string }
  | { type: "toggleExclude"; repoPath: string; path: string }
  // --- inspeção ---
  | ({ type: "inspectStart" } & Scoped)
  | ({ type: "inspectOk"; plan: ImportPlan } & Scoped)
  | ({ type: "inspectFail"; error: GitError } & Scoped)
  | { type: "cancel"; repoPath: string; at: number }
  // --- branch de trabalho + gravação (confirmados juntos) ---
  | ({ type: "branchStart" } & Scoped)
  | ({ type: "branchOk"; baseOid: string } & Scoped)
  | ({ type: "branchFail"; error: GitError } & Scoped)
  | ({ type: "applyOk"; result: ImportResult } & Scoped)
  | ({ type: "applyFail"; error: GitError } & Scoped)
  // --- desfazer ---
  | ({ type: "rollbackStart" } & Scoped)
  | ({ type: "rollbackOk"; result: RollbackResult } & Scoped)
  | ({ type: "rollbackFail"; error: GitError } & Scoped)
  // --- commit (executado pelo fluxo de commit existente) ---
  | { type: "commitStart"; repoPath: string; at: number }
  | { type: "commitAborted"; repoPath: string; at: number }
  | { type: "commitOk"; repoPath: string; result: CommitResult; at: number }
  // --- push ---
  | ({ type: "pushStart" } & Scoped)
  | ({ type: "pushOk"; result: PushResult } & Scoped)
  | ({ type: "pushFail"; error: GitError } & Scoped)
  // --- encerramento ---
  | ({ type: "finishOk" } & Scoped)
  | ({ type: "finishFail"; error: GitError } & Scoped)
  | { type: "resume"; repoPath: string; operation: ImportOperation; at: number }
  | { type: "dismissIssue" }
  | { type: "recover"; repoPath: string; at: number }
  | { type: "restart"; repoPath: string };

function log(
  state: ImportFlowState,
  step: string,
  outcome: FlowLogEntry["outcome"],
  at: number,
  detail?: string,
): readonly FlowLogEntry[] {
  const entry: FlowLogEntry = detail === undefined ? { step, outcome, at } : { step, outcome, at, detail };
  const next = [...state.log, entry];
  return next.length > MAX_LOG ? next.slice(next.length - MAX_LOG) : next;
}

/** Aplica uma transição somente se ela é permitida; do contrário, devolve o estado intacto. */
function go(
  state: ImportFlowState,
  to: ImportStage,
  patch: Partial<ImportFlowState> = {},
): ImportFlowState {
  if (!canTransition(state.stage, to)) return state;
  return { ...state, ...patch, stage: to };
}

/** Resposta assíncrona: vale só para o repositório atual, a tentativa atual e a etapa esperada. */
function isCurrent(state: ImportFlowState, action: Scoped, expected: ImportStage): boolean {
  return (
    state.repoPath === action.repoPath && state.seq === action.seq && state.stage === expected
  );
}

function sameRepo(state: ImportFlowState, repoPath: string): boolean {
  return state.repoPath !== null && state.repoPath === repoPath;
}

const EDITABLE_PLAN_STAGES: readonly ImportStage[] = ["selecting", "zipInvalid", "planReview"];

function sortedUnique(paths: readonly string[]): string[] {
  return [...new Set(paths)].sort();
}

export function importFlowReducer(state: ImportFlowState, action: ImportFlowAction): ImportFlowState {
  switch (action.type) {
    case "reset":
      // Trocar de repositório zera tudo; o contador de tentativas continua crescendo.
      return { ...INITIAL_IMPORT_FLOW, repoPath: action.repoPath, seq: state.seq };

    case "chooseZip": {
      if (!sameRepo(state, action.repoPath)) return state;
      if (!EDITABLE_PLAN_STAGES.includes(state.stage) && state.stage !== "cancelled") return state;
      return {
        ...state,
        stage: "selecting",
        zipPath: action.zipPath,
        sourcePrefix: "",
        baseBranch: action.baseBranch ?? state.baseBranch,
        workBranch: action.workBranch,
        plan: null,
        excluded: [],
        inspectError: null,
        issue: null,
      };
    }

    case "setBase": {
      if (!sameRepo(state, action.repoPath) || !EDITABLE_PLAN_STAGES.includes(state.stage)) return state;
      if (state.baseBranch === action.baseBranch) return state;
      // O plano depende da branch-base: precisa ser refeito.
      return { ...state, stage: "selecting", baseBranch: action.baseBranch, plan: null, excluded: [], inspectError: null };
    }

    case "setPrefix": {
      if (!sameRepo(state, action.repoPath) || !EDITABLE_PLAN_STAGES.includes(state.stage)) return state;
      if (state.sourcePrefix === action.prefix) return state;
      return { ...state, stage: "selecting", sourcePrefix: action.prefix, plan: null, excluded: [], inspectError: null };
    }

    case "setWorkBranch": {
      if (!sameRepo(state, action.repoPath) || !EDITABLE_PLAN_STAGES.includes(state.stage)) return state;
      return { ...state, workBranch: action.name };
    }

    case "toggleExclude": {
      if (!sameRepo(state, action.repoPath) || state.stage !== "planReview" || !state.plan) return state;
      const entry = state.plan.entries.find((e) => e.path === action.path);
      if (!entry || (entry.action !== "add" && entry.action !== "replace")) return state;
      const set = new Set(state.excluded);
      if (set.has(action.path)) set.delete(action.path);
      else set.add(action.path);
      return { ...state, excluded: sortedUnique([...set]) };
    }

    // ----- inspeção -----
    case "inspectStart": {
      if (!sameRepo(state, action.repoPath)) return state;
      if (!state.zipPath || !state.baseBranch) return state;
      if (state.stage === "inspecting") return state; // clique repetido
      if (!canTransition(state.stage, "inspecting")) return state;
      return go(state, "inspecting", {
        seq: action.seq,
        plan: null,
        excluded: [],
        inspectError: null,
        issue: null,
        log: log(state, "Inspeção do ZIP", "started", action.at, state.zipPath),
      });
    }
    case "inspectOk": {
      if (!isCurrent(state, action, "inspecting")) return state;
      // Um plano de outro repositório/base/prefixo nunca é aceito.
      if (action.plan.repoPath !== state.repoPath || action.plan.baseBranch !== state.baseBranch) return state;
      const s = action.plan.summary;
      return go(state, "planReview", {
        plan: action.plan,
        excluded: [],
        log: log(
          state,
          "Inspeção do ZIP",
          "ok",
          action.at,
          `${s.add} novo(s), ${s.replace} substituído(s), ${s.identical} idêntico(s), ${s.blocked} bloqueado(s)`,
        ),
      });
    }
    case "inspectFail": {
      if (!isCurrent(state, action, "inspecting")) return state;
      return go(state, "zipInvalid", {
        inspectError: action.error,
        log: log(state, "Inspeção do ZIP", "failed", action.at, action.error.message),
      });
    }
    case "cancel": {
      if (!sameRepo(state, action.repoPath)) return state;
      // Só é cancelável o que não altera nada. Durante gravação/commit/push, não há cancelamento.
      if (state.stage !== "inspecting") return state;
      return go(state, "cancelled", {
        seq: -1, // a resposta da inspeção em andamento será descartada
        log: log(state, "Inspeção do ZIP", "info", action.at, "Cancelada; nada foi alterado."),
      });
    }

    // ----- branch de trabalho -----
    case "branchStart": {
      if (!sameRepo(state, action.repoPath) || state.stage !== "planReview") return state;
      if (!state.plan || !state.plan.canApply) return state;
      return go(state, "creatingBranch", {
        seq: action.seq,
        failure: null,
        issue: null,
        log: log(state, "Branch de trabalho", "started", action.at, `${state.workBranch} (a partir de ${state.baseBranch})`),
      });
    }
    case "branchOk": {
      if (!isCurrent(state, action, "creatingBranch")) return state;
      return go(state, "importing", {
        log: log(state, "Branch de trabalho", "ok", action.at, `${state.workBranch} @ ${action.baseOid.slice(0, 7)}`),
      });
    }
    case "branchFail": {
      if (!isCurrent(state, action, "creatingBranch")) return state;
      const created = branchMayHaveBeenCreated(action.error);
      return go(state, "failed", {
        failure: {
          at: "creatingBranch",
          error: action.error,
          manual: created,
          completed: ["ZIP inspecionado", "Plano revisado e confirmado"],
          advice: [
            "Nenhum arquivo do ZIP foi gravado.",
            created
              ? `A branch “${state.workBranch}” foi criada, mas o aplicativo não conseguiu trocar para ela. Confira na aba Branches e troque manualmente antes de recomeçar.`
              : `Confira na aba Branches se a branch “${state.workBranch}” existe; ela não é removida automaticamente.`,
          ],
        },
        log: log(state, "Branch de trabalho", "failed", action.at, action.error.message),
      });
    }

    // ----- gravação -----
    case "applyOk": {
      if (!isCurrent(state, action, "importing")) return state;
      const r = action.result;
      const paths = r.applied.map((f) => f.path);
      if (r.state === "applied") {
        return go(state, "commitReview", {
          opId: r.opId,
          result: r,
          importedPaths: sortedUnique(paths),
          log: log(state, "Importação de arquivos", "ok", action.at, `${paths.length} arquivo(s) gravado(s)`),
        });
      }
      return go(state, "partial", {
        opId: r.opId,
        result: r,
        importedPaths: sortedUnique(paths),
        log: log(
          state,
          "Importação de arquivos",
          "failed",
          action.at,
          `${paths.length} gravado(s) antes da falha${r.failure ? ` em ${r.failure.path}` : ""}`,
        ),
      });
    }
    case "applyFail": {
      if (!isCurrent(state, action, "importing")) return state;
      return go(state, "failed", {
        failure: {
          at: "importing",
          error: action.error,
          manual: false,
          completed: ["ZIP inspecionado", "Plano confirmado", `Branch de trabalho “${state.workBranch}” criada e ativa`],
          advice: [
            "A validação falhou antes de qualquer gravação: nenhum arquivo do ZIP foi alterado no repositório.",
            `Volte para a branch-base “${state.baseBranch ?? ""}” e inspecione o ZIP novamente. A branch “${state.workBranch}” continua existindo (vazia).`,
          ],
        },
        log: log(state, "Importação de arquivos", "failed", action.at, action.error.message),
      });
    }

    // ----- desfazer -----
    case "rollbackStart": {
      if (!sameRepo(state, action.repoPath) || !state.opId) return state;
      if (state.stage !== "partial" && state.stage !== "commitReview") return state;
      return go(state, "rollingBack", {
        seq: action.seq,
        issue: null,
        rollbackFrom: state.stage,
        log: log(state, "Desfazer importação", "started", action.at),
      });
    }
    case "rollbackOk": {
      if (!isCurrent(state, action, "rollingBack")) return state;
      const r = action.result;
      const kept = r.keptModified.length;
      return go(state, "rolledBack", {
        rollback: r,
        rollbackFrom: null,
        log: log(
          state,
          "Desfazer importação",
          "ok",
          action.at,
          `${r.restored.length} restaurado(s), ${r.removed.length} removido(s)${kept ? `, ${kept} preservado(s) por terem sido editados` : ""}`,
        ),
      });
    }
    case "rollbackFail": {
      if (!isCurrent(state, action, "rollingBack")) return state;
      // Nada é dado como desfeito: volta à etapa de origem, com o problema visível.
      return go(state, state.rollbackFrom ?? "partial", {
        rollbackFrom: null,
        issue: { kind: "rollback", error: action.error, unconfirmed: action.error.code === "TIMEOUT" },
        log: log(state, "Desfazer importação", "failed", action.at, action.error.message),
      });
    }

    // ----- commit -----
    case "commitStart": {
      if (!sameRepo(state, action.repoPath)) return state;
      if (state.stage !== "commitReview") return state;
      return go(state, "committing", { log: log(state, "Commit", "started", action.at) });
    }
    case "commitAborted": {
      if (!sameRepo(state, action.repoPath) || state.stage !== "committing") return state;
      return go(state, "commitReview", { log: log(state, "Commit", "failed", action.at, "Não foi criado; os arquivos foram preservados.") });
    }
    case "commitOk": {
      if (!sameRepo(state, action.repoPath)) return state;
      if (state.stage !== "commitReview" && state.stage !== "committing") return state;
      return go(state, "committed", {
        commit: action.result,
        commitOid: action.result.oid,
        log: log(state, "Commit", "ok", action.at, `${action.result.shortOid} — ${action.result.subject}`),
      });
    }

    // ----- push -----
    case "pushStart": {
      if (!sameRepo(state, action.repoPath) || state.stage !== "committed") return state;
      return go(state, "pushing", {
        seq: action.seq,
        issue: null,
        log: log(state, "Push", "started", action.at),
      });
    }
    case "pushOk": {
      if (!isCurrent(state, action, "pushing")) return state;
      return go(state, "done", {
        push: action.result,
        log: log(state, "Push", "ok", action.at, `${action.result.target.remote}/${action.result.target.remoteBranch}`),
      });
    }
    case "pushFail": {
      if (!isCurrent(state, action, "pushing")) return state;
      // A importação e o commit continuam válidos: só a publicação não foi concluída.
      return go(state, "committed", {
        issue: { kind: "push", error: action.error, unconfirmed: action.error.code === "TIMEOUT" },
        log: log(state, "Push", "failed", action.at, action.error.message),
      });
    }

    // ----- encerramento -----
    case "finishOk": {
      if (!sameRepo(state, action.repoPath) || state.seq !== action.seq) return state;
      if (state.stage !== "committed") return state;
      return go(state, "done", { log: log(state, "Encerramento", "ok", action.at, "Concluído sem enviar ao remoto.") });
    }
    case "finishFail": {
      if (!sameRepo(state, action.repoPath) || state.seq !== action.seq) return state;
      return {
        ...state,
        issue: { kind: "finish", error: action.error, unconfirmed: false },
        log: log(state, "Encerramento", "failed", action.at, action.error.message),
      };
    }

    case "resume": {
      if (!sameRepo(state, action.repoPath) || state.stage !== "selecting") return state;
      const op = action.operation;
      const to = resumeStageFor(op);
      if (!to) return state;
      return go(state, to, {
        opId: op.opId,
        zipPath: op.zipPath,
        sourcePrefix: op.sourcePrefix,
        baseBranch: op.baseBranch,
        workBranch: op.workBranch,
        importedPaths: sortedUnique(op.files.map((f) => f.path)),
        commit: null,
        commitOid: op.commitOid,
        resumed: true,
        log: log(state, "Operação retomada", "info", action.at, `${op.workBranch} (${op.state})`),
      });
    }

    case "dismissIssue":
      return state.issue ? { ...state, issue: null } : state;

    case "recover": {
      if (!sameRepo(state, action.repoPath)) return state;
      if (state.stage !== "failed") return state;
      return go(state, "selecting", {
        plan: null,
        excluded: [],
        failure: null,
        issue: null,
        log: log(state, "Recuperação", "info", action.at, "Recomeçando a partir da seleção; o repositório é revalidado."),
      });
    }

    case "restart": {
      if (!sameRepo(state, action.repoPath)) return state;
      if (!canTransition(state.stage, "selecting")) return state;
      return {
        ...INITIAL_IMPORT_FLOW,
        repoPath: state.repoPath,
        seq: state.seq,
        baseBranch: state.baseBranch,
        log: state.log,
      };
    }
  }
}

// ---------------------------------------------------------------------------
// Seletores e utilidades
// ---------------------------------------------------------------------------

/**
 * O backend informa, na mensagem, que a branch foi criada mas a troca falhou
 * (`prepare_import_branch`). Usado só para orientar o usuário.
 */
export function branchMayHaveBeenCreated(error: GitError): boolean {
  return /foi criada/i.test(error.message);
}

export function resumeStageFor(operation: ImportOperation): ImportStage | null {
  switch (operation.state) {
    case "applying":
    case "partial":
      return "partial";
    case "applied":
      return "commitReview";
    case "committed":
      return "committed";
    default:
      return null;
  }
}

const WRITE_ACTIONS: readonly EntryAction[] = ["add", "replace"];

/** Arquivos que serão gravados: novos e substituições não excluídos pelo usuário. */
export function authorizedEntries(plan: ImportPlan | null, excluded: readonly string[]): PlanEntry[] {
  if (!plan) return [];
  const skip = new Set(excluded);
  return plan.entries.filter((e) => WRITE_ACTIONS.includes(e.action) && !skip.has(e.path));
}

export function overwrittenEntries(plan: ImportPlan | null, excluded: readonly string[]): PlanEntry[] {
  return authorizedEntries(plan, excluded).filter((e) => e.action === "replace");
}

export function blockedEntries(plan: ImportPlan | null): PlanEntry[] {
  return plan ? plan.entries.filter((e) => e.action === "blocked") : [];
}

/** Valida o nome da branch de trabalho (o backend/Git repetem e são a autoridade). */
export function workBranchProblem(
  name: string,
  baseBranch: string | null,
  existing: readonly string[],
): string | null {
  const quick = quickBranchNameProblem(name);
  if (quick) return quick;
  if (baseBranch !== null && name === baseBranch) return "A branch de trabalho precisa ser diferente da branch-base.";
  if (existing.includes(name)) return `Já existe uma branch chamada “${name}”. Escolha outro nome.`;
  return null;
}

export interface ConfirmContext {
  /** Branch atualmente ativa no repositório. */
  currentBranch: string | null;
  /** Nomes das branches locais existentes. */
  existingBranches: readonly string[];
  /** O usuário marcou a confirmação de sobrescrita. */
  overwriteAck: boolean;
  /** O usuário marcou a confirmação do plano e da branch. */
  planAck: boolean;
}

/** Tudo o que ainda impede de confirmar o plano (lista vazia = pode confirmar). */
export function confirmProblems(state: ImportFlowState, ctx: ConfirmContext): string[] {
  const problems: string[] = [];
  const plan = state.plan;
  if (state.stage !== "planReview" || !plan) return ["Inspecione o ZIP primeiro."];
  if (plan.blockers.length > 0) problems.push(...plan.blockers);
  if (blockedEntries(plan).length > 0) {
    problems.push("O ZIP tem entradas bloqueadas por segurança. Corrija o pacote e inspecione de novo.");
  }
  if (ctx.currentBranch !== null && state.baseBranch !== null && ctx.currentBranch !== state.baseBranch) {
    problems.push(`A branch ativa é “${ctx.currentBranch}”, mas a base escolhida é “${state.baseBranch}”. Troque para a base antes.`);
  }
  if (authorizedEntries(plan, state.excluded).length === 0) {
    problems.push("Nenhum arquivo será gravado (tudo é idêntico, bloqueado ou foi excluído por você).");
  }
  const nameProblem = workBranchProblem(state.workBranch, state.baseBranch, ctx.existingBranches);
  if (nameProblem) problems.push(nameProblem);
  if (overwrittenEntries(plan, state.excluded).length > 0 && !ctx.overwriteAck) {
    problems.push("Confirme que os arquivos existentes listados serão substituídos.");
  }
  if (!ctx.planAck) problems.push("Confirme o plano e a criação da branch de trabalho.");
  return problems;
}

/** Nas etapas de seleção/revisão ainda é possível mudar ZIP, base, pasta do pacote e exclusões. */
export function canEditSelection(stage: ImportStage): boolean {
  return EDITABLE_PLAN_STAGES.includes(stage);
}

export function canInspect(state: ImportFlowState): boolean {
  return (
    state.repoPath !== null &&
    state.zipPath !== null &&
    state.baseBranch !== null &&
    (state.stage === "selecting" || state.stage === "zipInvalid" || state.stage === "planReview")
  );
}

/** Sugestão (editável e sempre exibida) para o nome da branch: `importacao/<zip>-AAAAMMDD`. */
export function suggestWorkBranch(zipPath: string, now: Date): string {
  const file = zipPath.split(/[\\/]/).pop() ?? "";
  const base = file.replace(/\.zip$/i, "");
  const slug = base
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/\.{2,}/g, ".")
    .replace(/^[-.]+|[-.]+$/g, "")
    .replace(/(\.lock)$/g, "")
    .slice(0, 40);
  const pad = (n: number) => String(n).padStart(2, "0");
  const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`;
  return `importacao/${slug || "zip"}-${date}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB"];
  let value = bytes / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[i]}`;
}

export const ACTION_LABELS: Record<EntryAction, string> = {
  add: "Novo",
  replace: "Substitui",
  identical: "Idêntico",
  blocked: "Bloqueado",
  ignored: "Ignorado",
};

export type ActionTone = "added" | "modified" | "untracked" | "conflict" | "deleted";

export function actionTone(action: EntryAction): ActionTone {
  switch (action) {
    case "add":
      return "added";
    case "replace":
      return "modified";
    case "blocked":
      return "conflict";
    default:
      return "untracked";
  }
}

// ----- commit isolado -----

/**
 * Status com SOMENTE os arquivos da importação. O fluxo de commit existente é alimentado com
 * este recorte, então nada além do que a importação gravou pode ser selecionado (o backend
 * também recusa qualquer arquivo que não pertença à operação).
 */
export function restrictStatus(data: RepositoryStatus, allowed: ReadonlySet<string>): RepositoryStatus {
  const files = data.files.filter((f) => allowed.has(f.path));
  return {
    ...data,
    files,
    summary: {
      total: files.length,
      staged: files.filter((f) => f.index !== null).length,
      unstaged: files.filter((f) => f.worktree !== null).length,
      untracked: files.filter((f) => f.untracked).length,
      conflicted: files.filter((f) => f.conflicted).length,
    },
    isClean: files.length === 0,
  };
}

/** Alterações no repositório que NÃO pertencem à importação (não entram no commit dela). */
export function foreignChanges(data: RepositoryStatus | null, allowed: ReadonlySet<string>): string[] {
  return data ? data.files.filter((f) => !allowed.has(f.path)).map((f) => f.path) : [];
}

// ----- push -----

export interface PushPlan {
  target: SyncTarget | null;
  problems: string[];
  /** O destino é `main`/`master`: exige uma confirmação extra e explícita. */
  protectedDestination: boolean;
}

/**
 * Destino do push, só se for inequívoco: a branch ativa é a de trabalho, o HEAD é o commit da
 * importação e existe um upstream já configurado (o aplicativo nunca inventa nem define um).
 */
export function planPush(
  sync: SyncStatus | null,
  status: RepositoryStatus | null,
  workBranch: string,
  commitOid: string | null,
): PushPlan {
  const problems: string[] = [];
  if (!sync || !status) {
    return { target: null, problems: ["Aguardando o diagnóstico de sincronização."], protectedDestination: false };
  }
  if (sync.branch !== workBranch) {
    problems.push(`A branch ativa (${sync.branch ?? "HEAD desanexado"}) não é a branch de trabalho “${workBranch}”.`);
  }
  if (commitOid !== null && sync.headOid !== null && sync.headOid !== commitOid) {
    problems.push("O HEAD não é mais o commit criado pela importação. Use a aba Sincronização para enviar com revisão própria.");
  }
  const block = pushBlock(sync, status);
  if (block) problems.push(block.reason);
  const target = problems.length === 0 ? syncTarget(sync) : null;
  if (problems.length === 0 && !target) problems.push("O destino do envio não é inequívoco.");
  return {
    target,
    problems,
    protectedDestination: target ? isProtectedDestination(target.remoteBranch) : false,
  };
}

// ----- etapas visuais -----

export interface StepperItem {
  id: string;
  label: string;
  state: "done" | "current" | "todo";
}

const STEPS: { id: string; label: string; stages: readonly ImportStage[] }[] = [
  { id: "select", label: "Seleção", stages: ["selecting", "cancelled"] },
  { id: "inspect", label: "Inspeção", stages: ["inspecting", "zipInvalid"] },
  { id: "plan", label: "Plano", stages: ["planReview"] },
  { id: "import", label: "Importação", stages: ["creatingBranch", "importing", "partial", "rollingBack", "rolledBack", "failed"] },
  { id: "commit", label: "Diff e commit", stages: ["commitReview", "committing"] },
  { id: "push", label: "Push", stages: ["committed", "pushing"] },
  { id: "done", label: "Concluído", stages: ["done"] },
];

export function stepperFor(stage: ImportStage): StepperItem[] {
  const current = STEPS.findIndex((s) => s.stages.includes(stage));
  return STEPS.map((step, i) => ({
    id: step.id,
    label: step.label,
    state: i < current ? "done" : i === current ? "current" : "todo",
  }));
}
