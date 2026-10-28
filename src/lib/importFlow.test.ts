import { describe, expect, it } from "vitest";
import {
  ACTION_LABELS,
  INITIAL_IMPORT_FLOW,
  STAGE_LABELS,
  TRANSITIONS,
  authorizedEntries,
  blockedEntries,
  branchMayHaveBeenCreated,
  canInspect,
  canTransition,
  confirmProblems,
  formatBytes,
  importFlowReducer,
  isEngaged,
  isRunning,
  isWriting,
  overwrittenEntries,
  foreignChanges,
  planPush,
  resumeStageFor,
  restrictStatus,
  stepperFor,
  suggestWorkBranch,
  workBranchProblem,
  type ConfirmContext,
  type ImportFlowAction,
  type ImportFlowState,
  type ImportStage,
} from "./importFlow";
import type {
  CommitResult,
  GitError,
  ImportOperation,
  ImportPlan,
  ImportResult,
  PlanEntry,
  PushResult,
  RepositoryStatus,
  RollbackResult,
  SyncStatus,
} from "../types/git";

const REPO = "/repo";
const OTHER = "/outro";
const T = 1_800_000_000_000;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function entry(path: string, action: PlanEntry["action"], extra: Partial<PlanEntry> = {}): PlanEntry {
  return {
    path,
    action,
    size: 10,
    reason: null,
    newSha256: "n".repeat(8),
    oldSha256: action === "replace" || action === "identical" ? "o".repeat(8) : null,
    zipIndex: 0,
    zipName: path,
    ...extra,
  };
}

function plan(overrides: Partial<ImportPlan> = {}, entries?: PlanEntry[]): ImportPlan {
  const list = entries ?? [entry("novo.txt", "add"), entry("src/app.ts", "replace"), entry("igual.txt", "identical")];
  return {
    planId: "p".repeat(64),
    repoPath: REPO,
    baseBranch: "main",
    baseOid: "a".repeat(40),
    zipPath: "/zips/pacote.zip",
    zipSize: 100,
    zipSha256: "z".repeat(64),
    sourcePrefix: "",
    layout: { topLevelDirs: [], hasRootFiles: true, suggestedPrefix: null, directories: [] },
    entries: list,
    summary: {
      totalEntries: list.length,
      directories: 0,
      filesInScope: list.length,
      add: list.filter((e) => e.action === "add").length,
      replace: list.filter((e) => e.action === "replace").length,
      identical: list.filter((e) => e.action === "identical").length,
      blocked: list.filter((e) => e.action === "blocked").length,
      ignored: 0,
      bytesToWrite: 20,
      uncompressedBytes: 30,
    },
    blockers: [],
    warnings: [],
    canApply: true,
    ...overrides,
  };
}

function appliedResult(state: ImportResult["state"] = "applied"): ImportResult {
  return {
    opId: "0123456789abcdef",
    state,
    repoPath: REPO,
    workBranch: "importacao/x",
    applied: [
      { path: "novo.txt", action: "add", visibleInGit: true },
      { path: "src/app.ts", action: "replace", visibleInGit: true },
    ],
    failure: state === "partial" ? { path: "z.txt", message: "disco cheio" } : null,
    notAttempted: [],
    identicalSkipped: 1,
    recovery: state === "partial" ? "Desfaça a importação." : null,
  };
}

function commitResult(): CommitResult {
  return {
    repoPath: REPO,
    branch: "importacao/x",
    oid: "c".repeat(40),
    shortOid: "ccccccc",
    subject: "Importa ZIP",
    files: [],
    summary: null,
  } as unknown as CommitResult;
}

function pushResult(): PushResult {
  return {
    repoPath: REPO,
    target: { branch: "importacao/x", remote: "origin", remoteBranch: "importacao/x" },
    pushedCommits: 1,
    localOid: "c".repeat(40),
    output: "",
    sync: sync(),
  };
}

function rollbackResult(): RollbackResult {
  return { opId: "0123456789abcdef", state: "rolledBack", restored: ["src/app.ts"], removed: ["novo.txt"], keptModified: [], problems: [] };
}

function err(code: GitError["code"] = "COMMAND_FAILED", message = "falhou"): GitError {
  return { code, message };
}

function sync(overrides: Partial<SyncStatus> = {}): SyncStatus {
  return {
    repoPath: REPO,
    branch: "importacao/x",
    headOid: "c".repeat(40),
    remotes: [{ name: "origin", fetchUrl: "https://h/x.git", pushUrl: "https://h/x.git" }],
    upstream: {
      remote: "origin",
      remoteBranch: "importacao/x",
      display: "origin/importacao/x",
      trackingRef: "refs/remotes/origin/importacao/x",
      trackingRefExists: true,
    },
    state: "ahead",
    ahead: 1,
    behind: 0,
    pullStrategy: "merge",
    lastFetchAt: 1,
    ...overrides,
  };
}

function repoStatus(overrides: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    repoPath: REPO,
    branch: { name: "importacao/x", detached: false, unborn: false, headOid: "c".repeat(40), upstream: "origin/importacao/x", ahead: 1, behind: 0 },
    files: [],
    summary: { total: 0, staged: 0, unstaged: 0, untracked: 0, conflicted: 0 },
    operation: null,
    clean: true,
    ...overrides,
  } as unknown as RepositoryStatus;
}

function operation(state: ImportOperation["state"], overrides: Partial<ImportOperation> = {}): ImportOperation {
  return {
    opId: "0123456789abcdef",
    state,
    repoPath: REPO,
    workBranch: "importacao/x",
    baseBranch: "main",
    baseOid: "a".repeat(40),
    zipPath: "/zips/pacote.zip",
    sourcePrefix: "",
    files: [{ path: "novo.txt", action: "add" }, { path: "src/app.ts", action: "replace" }],
    commitOid: state === "committed" ? "c".repeat(40) : null,
    createdAt: 1,
    ...overrides,
  };
}

/** Executa ações em sequência a partir de um estado. */
function run(state: ImportFlowState, ...actions: ImportFlowAction[]): ImportFlowState {
  return actions.reduce(importFlowReducer, state);
}

const base = (): ImportFlowState => importFlowReducer(INITIAL_IMPORT_FLOW, { type: "reset", repoPath: REPO });

const choose = (): ImportFlowAction => ({
  type: "chooseZip",
  repoPath: REPO,
  zipPath: "/zips/pacote.zip",
  baseBranch: "main",
  workBranch: "importacao/pacote-20261027",
});

const inspecting = (seq = 1): ImportFlowState =>
  run(base(), choose(), { type: "inspectStart", repoPath: REPO, seq, at: T });

const reviewing = (p: ImportPlan = plan()): ImportFlowState =>
  run(inspecting(1), { type: "inspectOk", repoPath: REPO, seq: 1, plan: p, at: T });

const creatingBranch = (): ImportFlowState =>
  run(reviewing(), { type: "branchStart", repoPath: REPO, seq: 2, at: T });

const importing = (): ImportFlowState =>
  run(creatingBranch(), { type: "branchOk", repoPath: REPO, seq: 2, baseOid: "a".repeat(40), at: T });

const commitReview = (): ImportFlowState =>
  run(importing(), { type: "applyOk", repoPath: REPO, seq: 2, result: appliedResult(), at: T });

const partial = (): ImportFlowState =>
  run(importing(), { type: "applyOk", repoPath: REPO, seq: 2, result: appliedResult("partial"), at: T });

const committed = (): ImportFlowState =>
  run(commitReview(), { type: "commitStart", repoPath: REPO, at: T }, { type: "commitOk", repoPath: REPO, result: commitResult(), at: T });

// ---------------------------------------------------------------------------
// Transições
// ---------------------------------------------------------------------------

describe("transições permitidas", () => {
  it("todas as etapas têm rótulo e entrada na tabela", () => {
    const stages = Object.keys(TRANSITIONS) as ImportStage[];
    expect(stages.length).toBe(16);
    for (const stage of stages) {
      expect(STAGE_LABELS[stage]).toBeTruthy();
      for (const to of TRANSITIONS[stage]) expect(stages).toContain(to);
    }
  });

  it("não permite pular a revisão do plano, a do commit nem a confirmação do push", () => {
    const forbidden: [ImportStage, ImportStage][] = [
      ["selecting", "creatingBranch"],
      ["selecting", "importing"],
      ["inspecting", "creatingBranch"],
      ["planReview", "importing"],
      ["planReview", "commitReview"],
      ["creatingBranch", "commitReview"],
      ["importing", "committed"],
      ["importing", "pushing"],
      ["commitReview", "pushing"],
      ["commitReview", "done"],
      ["partial", "commitReview"],
      ["partial", "committing"],
      ["committed", "importing"],
      ["done", "pushing"],
      ["failed", "importing"],
      ["failed", "pushing"],
    ];
    for (const [from, to] of forbidden) expect(canTransition(from, to), `${from} → ${to}`).toBe(false);
  });

  it("uma falha só leva de volta à seleção (nunca adiante)", () => {
    expect(TRANSITIONS.failed).toEqual(["selecting"]);
    expect(TRANSITIONS.zipInvalid).not.toContain("creatingBranch");
  });

  it("classifica etapas em andamento, de escrita e engajadas", () => {
    expect(isRunning("inspecting")).toBe(true);
    expect(isWriting("inspecting")).toBe(false);
    for (const s of ["creatingBranch", "importing", "rollingBack", "committing", "pushing"] as const) {
      expect(isRunning(s)).toBe(true);
      expect(isWriting(s)).toBe(true);
    }
    expect(isRunning("planReview")).toBe(false);
    expect(isEngaged("partial")).toBe(true);
    expect(isEngaged("planReview")).toBe(false);
    expect(isEngaged("done")).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Fluxo feliz e ordem
// ---------------------------------------------------------------------------

describe("fluxo completo", () => {
  it("percorre todas as etapas na ordem e registra cada uma", () => {
    let s = reviewing();
    expect(s.stage).toBe("planReview");
    s = run(s, { type: "branchStart", repoPath: REPO, seq: 2, at: T });
    expect(s.stage).toBe("creatingBranch");
    s = run(s, { type: "branchOk", repoPath: REPO, seq: 2, baseOid: "a".repeat(40), at: T });
    expect(s.stage).toBe("importing");
    s = run(s, { type: "applyOk", repoPath: REPO, seq: 2, result: appliedResult(), at: T });
    expect(s.stage).toBe("commitReview");
    expect(s.opId).toBe("0123456789abcdef");
    expect(s.importedPaths).toEqual(["novo.txt", "src/app.ts"]);
    s = run(s, { type: "commitStart", repoPath: REPO, at: T }, { type: "commitOk", repoPath: REPO, result: commitResult(), at: T });
    expect(s.stage).toBe("committed");
    expect(s.commit?.shortOid).toBe("ccccccc");
    s = run(s, { type: "pushStart", repoPath: REPO, seq: 3, at: T });
    expect(s.stage).toBe("pushing");
    s = run(s, { type: "pushOk", repoPath: REPO, seq: 3, result: pushResult(), at: T });
    expect(s.stage).toBe("done");
    expect(s.push?.pushedCommits).toBe(1);
    const steps = s.log.map((l) => `${l.step}:${l.outcome}`);
    expect(steps).toEqual([
      "Inspeção do ZIP:started",
      "Inspeção do ZIP:ok",
      "Branch de trabalho:started",
      "Branch de trabalho:ok",
      "Importação de arquivos:ok",
      "Commit:started",
      "Commit:ok",
      "Push:started",
      "Push:ok",
    ]);
  });

  it("eventos fora de ordem são ignorados (estado idêntico, sem pular etapas)", () => {
    const sel = run(base(), choose());
    expect(importFlowReducer(sel, { type: "branchStart", repoPath: REPO, seq: 9, at: T })).toBe(sel);
    expect(importFlowReducer(sel, { type: "applyOk", repoPath: REPO, seq: 0, result: appliedResult(), at: T })).toBe(sel);
    const plan1 = reviewing();
    expect(importFlowReducer(plan1, { type: "applyOk", repoPath: REPO, seq: 1, result: appliedResult(), at: T })).toBe(plan1);
    expect(importFlowReducer(plan1, { type: "pushStart", repoPath: REPO, seq: 5, at: T })).toBe(plan1);
    expect(importFlowReducer(plan1, { type: "commitOk", repoPath: REPO, result: commitResult(), at: T })).toBe(plan1);
    const review = commitReview();
    expect(importFlowReducer(review, { type: "pushStart", repoPath: REPO, seq: 5, at: T })).toBe(review);
    expect(importFlowReducer(review, { type: "finishOk", repoPath: REPO, seq: review.seq, at: T })).toBe(review);
  });

  it("não aceita iniciar a gravação com plano bloqueado ou inexistente", () => {
    const blocked = reviewing(plan({ canApply: false, blockers: ["Há alterações locais"] }));
    expect(importFlowReducer(blocked, { type: "branchStart", repoPath: REPO, seq: 2, at: T })).toBe(blocked);
    const noZip = run(base(), { type: "inspectStart", repoPath: REPO, seq: 1, at: T });
    expect(noZip.stage).toBe("selecting");
  });
});

// ---------------------------------------------------------------------------
// Proteção contra respostas obsoletas e ações repetidas
// ---------------------------------------------------------------------------

describe("respostas obsoletas", () => {
  it("descarta resposta de outra tentativa", () => {
    const s = inspecting(5);
    expect(importFlowReducer(s, { type: "inspectOk", repoPath: REPO, seq: 4, plan: plan(), at: T })).toBe(s);
    expect(importFlowReducer(s, { type: "inspectFail", repoPath: REPO, seq: 6, error: err(), at: T })).toBe(s);
  });

  it("descarta resposta de outro repositório e depois de trocar de repositório", () => {
    const s = inspecting(5);
    expect(importFlowReducer(s, { type: "inspectOk", repoPath: OTHER, seq: 5, plan: plan({ repoPath: OTHER }), at: T })).toBe(s);
    const moved = importFlowReducer(s, { type: "reset", repoPath: OTHER });
    expect(moved.stage).toBe("selecting");
    expect(moved.zipPath).toBeNull();
    expect(moved.seq).toBe(5); // a contagem não volta para trás
    expect(importFlowReducer(moved, { type: "inspectOk", repoPath: REPO, seq: 5, plan: plan(), at: T })).toBe(moved);
  });

  it("não aceita um plano que não corresponde ao repositório ou à base atuais", () => {
    const s = inspecting(1);
    expect(importFlowReducer(s, { type: "inspectOk", repoPath: REPO, seq: 1, plan: plan({ repoPath: OTHER }), at: T })).toBe(s);
    expect(importFlowReducer(s, { type: "inspectOk", repoPath: REPO, seq: 1, plan: plan({ baseBranch: "outra" }), at: T })).toBe(s);
  });

  it("descarta a resposta de uma inspeção cancelada", () => {
    const s = inspecting(3);
    const cancelled = importFlowReducer(s, { type: "cancel", repoPath: REPO, at: T });
    expect(cancelled.stage).toBe("cancelled");
    expect(importFlowReducer(cancelled, { type: "inspectOk", repoPath: REPO, seq: 3, plan: plan(), at: T })).toBe(cancelled);
    expect(cancelled.log.at(-1)?.detail).toMatch(/nada foi alterado/i);
  });

  it("ações de repositório diferente do atual não alteram o estado", () => {
    const s = reviewing();
    expect(importFlowReducer(s, { type: "toggleExclude", repoPath: OTHER, path: "novo.txt" })).toBe(s);
    expect(importFlowReducer(s, { type: "branchStart", repoPath: OTHER, seq: 9, at: T })).toBe(s);
    expect(importFlowReducer(s, { type: "setWorkBranch", repoPath: OTHER, name: "x" })).toBe(s);
  });
});

describe("cliques repetidos", () => {
  it("ignora inspecionar, criar branch e enviar enquanto a etapa está em andamento", () => {
    const a = inspecting(1);
    expect(importFlowReducer(a, { type: "inspectStart", repoPath: REPO, seq: 2, at: T })).toBe(a);
    const b = creatingBranch();
    expect(importFlowReducer(b, { type: "branchStart", repoPath: REPO, seq: 9, at: T })).toBe(b);
    const c = run(committed(), { type: "pushStart", repoPath: REPO, seq: 7, at: T });
    expect(importFlowReducer(c, { type: "pushStart", repoPath: REPO, seq: 8, at: T })).toBe(c);
    // Durante a gravação não há "desfazer": o pedido é ignorado.
    const imp = importing();
    expect(importFlowReducer(imp, { type: "rollbackStart", repoPath: REPO, seq: 9, at: T })).toBe(imp);
  });

  it("só o resultado da tentativa corrente é aplicado (segunda resposta ignorada)", () => {
    let s = run(committed(), { type: "pushStart", repoPath: REPO, seq: 7, at: T });
    s = run(s, { type: "pushOk", repoPath: REPO, seq: 7, result: pushResult(), at: T });
    expect(s.stage).toBe("done");
    const again = importFlowReducer(s, { type: "pushFail", repoPath: REPO, seq: 7, error: err(), at: T });
    expect(again).toBe(s);
  });
});

// ---------------------------------------------------------------------------
// Seleção e plano
// ---------------------------------------------------------------------------

describe("seleção, prefixo e exclusões", () => {
  it("mudar base ou prefixo invalida o plano e exige nova inspeção", () => {
    const s = reviewing();
    const prefix = importFlowReducer(s, { type: "setPrefix", repoPath: REPO, prefix: "projeto" });
    expect(prefix.stage).toBe("selecting");
    expect(prefix.plan).toBeNull();
    expect(prefix.sourcePrefix).toBe("projeto");
    const baseChanged = importFlowReducer(s, { type: "setBase", repoPath: REPO, baseBranch: "dev" });
    expect(baseChanged.stage).toBe("selecting");
    expect(baseChanged.plan).toBeNull();
    // Mesmo valor: nada muda.
    expect(importFlowReducer(s, { type: "setPrefix", repoPath: REPO, prefix: "" })).toBe(s);
  });

  it("não permite mudar prefixo/base durante ou depois da gravação", () => {
    for (const s of [creatingBranch(), importing(), commitReview(), committed()]) {
      expect(importFlowReducer(s, { type: "setPrefix", repoPath: REPO, prefix: "x" })).toBe(s);
      expect(importFlowReducer(s, { type: "setBase", repoPath: REPO, baseBranch: "x" })).toBe(s);
      expect(importFlowReducer(s, { type: "chooseZip", repoPath: REPO, zipPath: "/outro.zip", baseBranch: null, workBranch: "x" })).toBe(s);
    }
  });

  it("escolher outro ZIP descarta plano, exclusões e erros", () => {
    const s = importFlowReducer(reviewing(), { type: "toggleExclude", repoPath: REPO, path: "novo.txt" });
    const n = importFlowReducer(s, { type: "chooseZip", repoPath: REPO, zipPath: "/zips/b.zip", baseBranch: "main", workBranch: "importacao/b" });
    expect(n.plan).toBeNull();
    expect(n.excluded).toEqual([]);
    expect(n.zipPath).toBe("/zips/b.zip");
    expect(n.workBranch).toBe("importacao/b");
  });

  it("só arquivos novos/substituídos podem ser excluídos, e só na revisão do plano", () => {
    const s = reviewing();
    const a = importFlowReducer(s, { type: "toggleExclude", repoPath: REPO, path: "src/app.ts" });
    expect(a.excluded).toEqual(["src/app.ts"]);
    const b = importFlowReducer(a, { type: "toggleExclude", repoPath: REPO, path: "novo.txt" });
    expect(b.excluded).toEqual(["novo.txt", "src/app.ts"]); // ordenado
    expect(importFlowReducer(b, { type: "toggleExclude", repoPath: REPO, path: "novo.txt" }).excluded).toEqual(["src/app.ts"]);
    expect(importFlowReducer(s, { type: "toggleExclude", repoPath: REPO, path: "igual.txt" })).toBe(s); // idêntico
    expect(importFlowReducer(s, { type: "toggleExclude", repoPath: REPO, path: "nao-existe.txt" })).toBe(s);
    const imp = importing();
    expect(importFlowReducer(imp, { type: "toggleExclude", repoPath: REPO, path: "novo.txt" })).toBe(imp);
  });

  it("calcula os arquivos autorizados e as sobrescritas depois das exclusões", () => {
    const p = plan();
    expect(authorizedEntries(p, []).map((e) => e.path)).toEqual(["novo.txt", "src/app.ts"]);
    expect(authorizedEntries(p, ["novo.txt"]).map((e) => e.path)).toEqual(["src/app.ts"]);
    expect(overwrittenEntries(p, []).map((e) => e.path)).toEqual(["src/app.ts"]);
    expect(overwrittenEntries(p, ["src/app.ts"])).toEqual([]);
    expect(authorizedEntries(null, [])).toEqual([]);
    expect(blockedEntries(plan({}, [entry("x", "blocked"), entry("y", "add")])).map((e) => e.path)).toEqual(["x"]);
  });

  it("canInspect exige ZIP, base e etapa de seleção/revisão", () => {
    expect(canInspect(base())).toBe(false);
    expect(canInspect(run(base(), choose()))).toBe(true);
    expect(canInspect(reviewing())).toBe(true);
    expect(canInspect(importing())).toBe(false);
    expect(canInspect(inspecting())).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Falhas
// ---------------------------------------------------------------------------

describe("inspeção inválida", () => {
  it("ZIP inválido leva a zipInvalid com o erro e permite tentar de novo", () => {
    const e = err("INVALID_ARCHIVE", "O arquivo não é um ZIP válido.");
    const s = run(inspecting(1), { type: "inspectFail", repoPath: REPO, seq: 1, error: e, at: T });
    expect(s.stage).toBe("zipInvalid");
    expect(s.inspectError).toEqual(e);
    expect(s.plan).toBeNull();
    const retry = run(s, { type: "inspectStart", repoPath: REPO, seq: 2, at: T });
    expect(retry.stage).toBe("inspecting");
    expect(retry.inspectError).toBeNull();
    // Não há como confirmar nada a partir de um ZIP inválido.
    expect(importFlowReducer(s, { type: "branchStart", repoPath: REPO, seq: 3, at: T })).toBe(s);
  });

  it("plano com entradas bloqueadas não pode ser confirmado", () => {
    const s = reviewing(plan({ canApply: false }, [entry("ok.txt", "add"), entry("../x", "blocked", { reason: "travessia" })]));
    const problems = confirmProblems(s, ctx({ planAck: true }));
    expect(problems.some((p) => /bloqueadas por segurança/.test(p))).toBe(true);
    expect(importFlowReducer(s, { type: "branchStart", repoPath: REPO, seq: 2, at: T })).toBe(s);
  });
});

describe("falhas na branch e na gravação", () => {
  it("falha ao criar a branch interrompe o fluxo, informa o que foi concluído e só permite recomeçar", () => {
    const s = run(creatingBranch(), { type: "branchFail", repoPath: REPO, seq: 2, error: err("INVALID_ARGUMENT", "Já existe uma branch."), at: T });
    expect(s.stage).toBe("failed");
    expect(s.failure?.at).toBe("creatingBranch");
    expect(s.failure?.manual).toBe(false);
    expect(s.failure?.completed).toContain("ZIP inspecionado");
    expect(s.failure?.advice.join(" ")).toMatch(/Nenhum arquivo do ZIP foi gravado/);
    // Nada avança a partir de uma falha, exceto recomeçar.
    expect(importFlowReducer(s, { type: "applyOk", repoPath: REPO, seq: 2, result: appliedResult(), at: T })).toBe(s);
    const r = run(s, { type: "recover", repoPath: REPO, at: T });
    expect(r.stage).toBe("selecting");
    expect(r.failure).toBeNull();
    expect(r.zipPath).toBe("/zips/pacote.zip"); // a seleção é preservada
  });

  it("branch criada mas troca falhou exige verificação manual", () => {
    const e = err("OPERATION_BLOCKED", "A branch “importacao/x” foi criada, mas não foi possível trocar para ela: x");
    expect(branchMayHaveBeenCreated(e)).toBe(true);
    expect(branchMayHaveBeenCreated(err("COMMAND_FAILED", "outra coisa"))).toBe(false);
    const s = run(creatingBranch(), { type: "branchFail", repoPath: REPO, seq: 2, error: e, at: T });
    expect(s.failure?.manual).toBe(true);
    expect(s.failure?.advice.join(" ")).toMatch(/Confira na aba Branches/);
  });

  it("erro de validação antes de gravar: nada foi alterado e a orientação diz como voltar", () => {
    const s = run(importing(), { type: "applyFail", repoPath: REPO, seq: 2, error: err("STALE_REVIEW", "O plano mudou."), at: T });
    expect(s.stage).toBe("failed");
    expect(s.failure?.at).toBe("importing");
    expect(s.failure?.advice.join(" ")).toMatch(/nenhum arquivo do ZIP foi alterado/i);
    expect(s.failure?.advice.join(" ")).toMatch(/branch-base “main”/);
    expect(s.result).toBeNull();
  });

  it("gravação parcial vai para `partial` com o resultado e não permite commit", () => {
    const s = partial();
    expect(s.stage).toBe("partial");
    expect(s.result?.failure?.path).toBe("z.txt");
    expect(s.importedPaths).toEqual(["novo.txt", "src/app.ts"]);
    expect(importFlowReducer(s, { type: "commitStart", repoPath: REPO, at: T })).toBe(s);
    expect(importFlowReducer(s, { type: "pushStart", repoPath: REPO, seq: 5, at: T })).toBe(s);
    expect(importFlowReducer(s, { type: "recover", repoPath: REPO, at: T })).toBe(s);
    expect(importFlowReducer(s, { type: "restart", repoPath: REPO })).toBe(s);
  });
});

describe("desfazer a importação", () => {
  it("a partir de uma gravação parcial: desfaz e termina em rolledBack", () => {
    let s = run(partial(), { type: "rollbackStart", repoPath: REPO, seq: 4, at: T });
    expect(s.stage).toBe("rollingBack");
    s = run(s, { type: "rollbackOk", repoPath: REPO, seq: 4, result: rollbackResult(), at: T });
    expect(s.stage).toBe("rolledBack");
    expect(s.rollback?.removed).toEqual(["novo.txt"]);
    expect(run(s, { type: "restart", repoPath: REPO }).stage).toBe("selecting");
  });

  it("se desfazer falhar, volta à etapa de origem com o problema visível (nada é dado como desfeito)", () => {
    const a = run(partial(), { type: "rollbackStart", repoPath: REPO, seq: 4, at: T }, { type: "rollbackFail", repoPath: REPO, seq: 4, error: err("OPERATION_BLOCKED", "Volte para a branch"), at: T });
    expect(a.stage).toBe("partial");
    expect(a.issue?.kind).toBe("rollback");
    const b = run(commitReview(), { type: "rollbackStart", repoPath: REPO, seq: 4, at: T }, { type: "rollbackFail", repoPath: REPO, seq: 4, error: err("TIMEOUT", "demorou"), at: T });
    expect(b.stage).toBe("commitReview");
    expect(b.issue?.unconfirmed).toBe(true);
  });

  it("não é possível desfazer depois do commit", () => {
    const s = committed();
    expect(importFlowReducer(s, { type: "rollbackStart", repoPath: REPO, seq: 9, at: T })).toBe(s);
  });
});

describe("commit e push", () => {
  it("commitStart/commitOk só existem na revisão; cancelar o commit volta à revisão", () => {
    const review = commitReview();
    const c = run(review, { type: "commitStart", repoPath: REPO, at: T });
    expect(c.stage).toBe("committing");
    expect(run(c, { type: "commitAborted", repoPath: REPO, at: T }).stage).toBe("commitReview");
    expect(importFlowReducer(partial(), { type: "commitOk", repoPath: REPO, result: commitResult(), at: T }).stage).toBe("partial");
  });

  it("push rejeitado preserva o commit, volta a `committed` e não repete sozinho", () => {
    const s = run(committed(), { type: "pushStart", repoPath: REPO, seq: 7, at: T }, { type: "pushFail", repoPath: REPO, seq: 7, error: err("PUSH_REJECTED", "rejeitado"), at: T });
    expect(s.stage).toBe("committed");
    expect(s.commit?.oid).toBe("c".repeat(40));
    expect(s.issue).toEqual({ kind: "push", error: err("PUSH_REJECTED", "rejeitado"), unconfirmed: false });
    // Não existe nova tentativa automática: é preciso um novo pushStart explícito.
    expect(s.log.at(-1)).toMatchObject({ step: "Push", outcome: "failed" });
    expect(run(s, { type: "dismissIssue" }).issue).toBeNull();
    const again = run(s, { type: "pushStart", repoPath: REPO, seq: 8, at: T });
    expect(again.stage).toBe("pushing");
    expect(again.issue).toBeNull();
  });

  it("tempo esgotado no push é marcado como não confirmado", () => {
    const s = run(committed(), { type: "pushStart", repoPath: REPO, seq: 7, at: T }, { type: "pushFail", repoPath: REPO, seq: 7, error: err("TIMEOUT", "demorou"), at: T });
    expect(s.issue?.unconfirmed).toBe(true);
    expect(s.stage).toBe("committed");
  });

  it("concluir sem enviar encerra o fluxo; falha ao encerrar mantém a etapa", () => {
    const s = committed();
    const ok = run(s, { type: "finishOk", repoPath: REPO, seq: s.seq, at: T });
    expect(ok.stage).toBe("done");
    expect(ok.push).toBeNull();
    const bad = run(s, { type: "finishFail", repoPath: REPO, seq: s.seq, error: err("OPERATION_BLOCKED", "x"), at: T });
    expect(bad.stage).toBe("committed");
    expect(bad.issue?.kind).toBe("finish");
  });
});

describe("cancelamento", () => {
  it("só cancela o que não altera nada (inspeção)", () => {
    expect(importFlowReducer(inspecting(), { type: "cancel", repoPath: REPO, at: T }).stage).toBe("cancelled");
    for (const s of [creatingBranch(), importing(), run(committed(), { type: "pushStart", repoPath: REPO, seq: 9, at: T }), partial(), commitReview(), reviewing()]) {
      expect(importFlowReducer(s, { type: "cancel", repoPath: REPO, at: T })).toBe(s);
    }
  });

  it("depois de cancelar é possível escolher outro ZIP ou recomeçar", () => {
    const c = importFlowReducer(inspecting(), { type: "cancel", repoPath: REPO, at: T });
    expect(run(c, { type: "restart", repoPath: REPO }).stage).toBe("selecting");
    expect(run(c, choose()).stage).toBe("selecting");
  });
});

// ---------------------------------------------------------------------------
// Retomada
// ---------------------------------------------------------------------------

describe("retomada de uma operação gravada", () => {
  it("mapeia o estado persistido para a etapa correta", () => {
    expect(resumeStageFor(operation("applying"))).toBe("partial");
    expect(resumeStageFor(operation("partial"))).toBe("partial");
    expect(resumeStageFor(operation("applied"))).toBe("commitReview");
    expect(resumeStageFor(operation("committed"))).toBe("committed");
    expect(resumeStageFor(operation("rolledBack"))).toBeNull();
    expect(resumeStageFor(operation("finished"))).toBeNull();
  });

  it("retoma a partir da seleção sem repetir etapas e sem reabrir o plano", () => {
    const s = run(base(), { type: "resume", repoPath: REPO, operation: operation("applied"), at: T });
    expect(s.stage).toBe("commitReview");
    expect(s.resumed).toBe(true);
    expect(s.opId).toBe("0123456789abcdef");
    expect(s.workBranch).toBe("importacao/x");
    expect(s.importedPaths).toEqual(["novo.txt", "src/app.ts"]);
    expect(s.plan).toBeNull();
    const c = run(base(), { type: "resume", repoPath: REPO, operation: operation("committed"), at: T });
    expect(c.stage).toBe("committed");
    expect(c.commitOid).toBe("c".repeat(40)); // o push confere o HEAD com este commit
    expect(committed().commitOid).toBe("c".repeat(40));
    // Depois de retomado em `committed`, o commit não pode ser criado de novo.
    expect(importFlowReducer(c, { type: "commitStart", repoPath: REPO, at: T })).toBe(c);
  });

  it("ignora operações encerradas e retomada em meio a outra etapa", () => {
    const b = base();
    expect(importFlowReducer(b, { type: "resume", repoPath: REPO, operation: operation("finished"), at: T })).toBe(b);
    const mid = importing();
    expect(importFlowReducer(mid, { type: "resume", repoPath: REPO, operation: operation("applied"), at: T })).toBe(mid);
    expect(importFlowReducer(b, { type: "resume", repoPath: OTHER, operation: operation("applied"), at: T })).toBe(b);
  });

  it("uma retomada parcial pode ser desfeita", () => {
    const s = run(base(), { type: "resume", repoPath: REPO, operation: operation("partial"), at: T }, { type: "rollbackStart", repoPath: REPO, seq: 2, at: T });
    expect(s.stage).toBe("rollingBack");
  });
});

// ---------------------------------------------------------------------------
// Confirmação do plano
// ---------------------------------------------------------------------------

function ctx(overrides: Partial<ConfirmContext> = {}): ConfirmContext {
  return { currentBranch: "main", existingBranches: ["main"], overwriteAck: true, planAck: true, ...overrides };
}

function withBranch(s: ImportFlowState, name: string): ImportFlowState {
  return importFlowReducer(s, { type: "setWorkBranch", repoPath: REPO, name });
}

describe("confirmProblems", () => {
  const ready = () => withBranch(reviewing(), "importacao/pacote-20261027");

  it("sem problemas quando tudo foi revisado e confirmado", () => {
    expect(confirmProblems(ready(), ctx())).toEqual([]);
  });

  it("exige confirmar a sobrescrita enquanto houver arquivos existentes a substituir", () => {
    expect(confirmProblems(ready(), ctx({ overwriteAck: false })).join(" ")).toMatch(/substituídos/);
    // Se o usuário exclui a substituição, a confirmação deixa de ser necessária.
    const s = importFlowReducer(ready(), { type: "toggleExclude", repoPath: REPO, path: "src/app.ts" });
    expect(confirmProblems(s, ctx({ overwriteAck: false }))).toEqual([]);
  });

  it("exige a confirmação explícita do plano e da branch", () => {
    expect(confirmProblems(ready(), ctx({ planAck: false })).join(" ")).toMatch(/Confirme o plano/);
  });

  it("bloqueia quando a branch ativa não é a base", () => {
    expect(confirmProblems(ready(), ctx({ currentBranch: "outra" })).join(" ")).toMatch(/Troque para a base/);
  });

  it("bloqueia nome inválido, igual à base ou já existente", () => {
    expect(confirmProblems(withBranch(reviewing(), "nome inválido"), ctx()).length).toBeGreaterThan(0);
    expect(confirmProblems(withBranch(reviewing(), "main"), ctx()).join(" ")).toMatch(/diferente da branch-base/);
    expect(confirmProblems(withBranch(reviewing(), "ja-existe"), ctx({ existingBranches: ["main", "ja-existe"] })).join(" ")).toMatch(/Já existe/);
  });

  it("bloqueia quando o repositório tem impedimentos ou nada será gravado", () => {
    const blocked = withBranch(reviewing(plan({ canApply: false, blockers: ["Há alterações locais não commitadas."] })), "importacao/x");
    expect(confirmProblems(blocked, ctx())).toContain("Há alterações locais não commitadas.");
    const none = withBranch(reviewing(plan({}, [entry("igual.txt", "identical")])), "importacao/x");
    expect(confirmProblems(none, ctx()).join(" ")).toMatch(/Nenhum arquivo será gravado/);
    const allExcluded = importFlowReducer(importFlowReducer(ready(), { type: "toggleExclude", repoPath: REPO, path: "novo.txt" }), { type: "toggleExclude", repoPath: REPO, path: "src/app.ts" });
    expect(confirmProblems(allExcluded, ctx()).join(" ")).toMatch(/Nenhum arquivo será gravado/);
  });

  it("fora da etapa de revisão não há o que confirmar", () => {
    expect(confirmProblems(importing(), ctx())).toEqual(["Inspecione o ZIP primeiro."]);
  });
});

describe("nome da branch de trabalho", () => {
  it("valida contra a base e as branches existentes", () => {
    expect(workBranchProblem("importacao/ok", "main", ["main"])).toBeNull();
    expect(workBranchProblem("", "main", [])).toMatch(/Informe/);
    expect(workBranchProblem("com espaço", "main", [])).toMatch(/espaços/);
    expect(workBranchProblem("-x", "main", [])).toMatch(/hífen/);
    expect(workBranchProblem("main", "main", ["main"])).toMatch(/diferente/);
    expect(workBranchProblem("dev", "main", ["main", "dev"])).toMatch(/Já existe/);
  });

  it("sugere um nome legível, sem acentos e válido (sempre editável)", () => {
    const d = new Date(2026, 9, 27);
    expect(suggestWorkBranch("C:\\Downloads\\Atualização Final (v2).zip", d)).toBe("importacao/atualizacao-final-v2-20261027");
    expect(suggestWorkBranch("/tmp/app.ZIP", d)).toBe("importacao/app-20261027");
    expect(suggestWorkBranch("/tmp/...zip", d)).toBe("importacao/zip-20261027");
    const long = suggestWorkBranch(`/tmp/${"a".repeat(100)}.zip`, d);
    expect(long.length).toBeLessThan(70);
    for (const zip of ["C:\\x\\Relatório 2026.zip", "/a/b/..zip", "/a/b/x.lock.zip"]) {
      expect(workBranchProblem(suggestWorkBranch(zip, d), "main", [])).toBeNull();
    }
  });
});

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

describe("planPush", () => {
  const WORK = "importacao/x";
  const OID = "c".repeat(40);

  it("devolve o destino exato quando tudo é inequívoco", () => {
    const p = planPush(sync(), repoStatus(), WORK, OID);
    expect(p.problems).toEqual([]);
    expect(p.target).toEqual({ branch: WORK, remote: "origin", remoteBranch: WORK });
    expect(p.protectedDestination).toBe(false);
  });

  it("sem upstream: não inventa destino", () => {
    const p = planPush(sync({ upstream: null, state: "noUpstream", ahead: null, behind: null }), repoStatus(), WORK, OID);
    expect(p.target).toBeNull();
    expect(p.problems.join(" ")).toMatch(/não tem upstream/);
  });

  it("recusa quando a branch ativa ou o HEAD não correspondem à importação", () => {
    expect(planPush(sync({ branch: "main" }), repoStatus(), WORK, OID).problems.join(" ")).toMatch(/não é a branch de trabalho/);
    expect(planPush(sync({ headOid: "d".repeat(40) }), repoStatus(), WORK, OID).problems.join(" ")).toMatch(/não é mais o commit/);
  });

  it("recusa divergência, atraso e conflitos", () => {
    expect(planPush(sync({ state: "diverged" }), repoStatus(), WORK, OID).target).toBeNull();
    expect(planPush(sync({ state: "behind", ahead: 0, behind: 2 }), repoStatus(), WORK, OID).target).toBeNull();
    expect(planPush(sync(), repoStatus({ operation: "merge" }), WORK, OID).target).toBeNull();
  });

  it("destino main/master exige confirmação extra", () => {
    const s = sync({
      branch: WORK,
      upstream: { remote: "origin", remoteBranch: "main", display: "origin/main", trackingRef: "refs/remotes/origin/main", trackingRefExists: true },
    });
    const p = planPush(s, repoStatus(), WORK, OID);
    expect(p.target?.remoteBranch).toBe("main");
    expect(p.protectedDestination).toBe(true);
  });

  it("aguarda o diagnóstico quando ainda não há dados", () => {
    expect(planPush(null, null, WORK, OID).problems).toEqual(["Aguardando o diagnóstico de sincronização."]);
  });
});

// ---------------------------------------------------------------------------
// Utilidades de exibição
// ---------------------------------------------------------------------------

describe("exibição", () => {
  it("formata tamanhos", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(20 * 1024 * 1024)).toBe("20 MiB");
  });

  it("rotula todas as ações do plano", () => {
    expect(Object.keys(ACTION_LABELS).sort()).toEqual(["add", "blocked", "identical", "ignored", "replace"]);
  });

  it("o indicador de etapas acompanha o estágio", () => {
    const at = (stage: ImportStage) => stepperFor(stage).find((s) => s.state === "current")?.id;
    expect(at("selecting")).toBe("select");
    expect(at("inspecting")).toBe("inspect");
    expect(at("planReview")).toBe("plan");
    expect(at("importing")).toBe("import");
    expect(at("partial")).toBe("import");
    expect(at("commitReview")).toBe("commit");
    expect(at("committed")).toBe("push");
    expect(at("done")).toBe("done");
    const done = stepperFor("committed").map((s) => s.state);
    expect(done).toEqual(["done", "done", "done", "done", "done", "current", "todo"]);
  });
});

describe("registro", () => {
  it("é limitado e mantém as entradas mais recentes", () => {
    let s = reviewing();
    for (let i = 0; i < 260; i += 1) {
      s = run(s, { type: "inspectStart", repoPath: REPO, seq: 10 + i, at: i }, { type: "inspectFail", repoPath: REPO, seq: 10 + i, error: err(), at: i });
    }
    expect(s.log.length).toBeLessThanOrEqual(200);
    expect(s.log.at(-1)?.at).toBe(259);
  });
});

describe("commit isolado", () => {
  const f = (path: string, extra: Record<string, unknown> = {}) => ({
    path,
    origPath: null,
    index: null,
    worktree: "modified",
    untracked: false,
    conflicted: false,
    conflictCode: null,
    submodule: false,
    ...extra,
  });
  const data = repoStatus({
    files: [f("novo.txt", { worktree: null, untracked: true }), f("src/app.ts"), f("alheio.txt"), f("preparado.txt", { index: "modified", worktree: null })] as never,
  });

  it("restrictStatus mantém só os arquivos da importação e recalcula o resumo", () => {
    const only = restrictStatus(data, new Set(["novo.txt", "src/app.ts"]));
    expect(only.files.map((x) => x.path)).toEqual(["novo.txt", "src/app.ts"]);
    expect(only.summary).toEqual({ total: 2, staged: 0, unstaged: 1, untracked: 1, conflicted: 0 });
    expect(only.isClean).toBe(false);
    expect(only.branch).toBe(data.branch); // o restante do status é preservado
    expect(restrictStatus(data, new Set()).isClean).toBe(true);
  });

  it("foreignChanges lista o que está alterado fora da importação", () => {
    expect(foreignChanges(data, new Set(["novo.txt", "src/app.ts"]))).toEqual(["alheio.txt", "preparado.txt"]);
    expect(foreignChanges(null, new Set())).toEqual([]);
  });
});
