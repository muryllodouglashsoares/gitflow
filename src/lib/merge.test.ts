import { describe, expect, it } from "vitest";
import {
  canSwitchToTarget,
  confirmationReady,
  deriveStage,
  forecastText,
  relationSummary,
  resultSummary,
  selectionProblem,
  type StageInput,
} from "./merge";
import type {
  BranchEntry,
  MergeBlocker,
  MergePreview,
  MergeResult,
  RepositoryStatus,
} from "../types/git";

const entry = (name: string): BranchEntry => ({
  name,
  current: false,
  oid: "abcdef1234567890",
  upstream: null,
  subject: null,
});

function preview(o: Partial<MergePreview> = {}): MergePreview {
  return {
    repoPath: "/r",
    source: entry("feature/login"),
    target: entry("main"),
    targetProtected: true,
    requiredConfirmation: "main",
    currentBranch: "main",
    headOid: "abcdef1234567890",
    targetIsCurrent: true,
    relation: "fastForward",
    strategy: "fastForward",
    mergeBase: "abcdef1234567890",
    sourceOnly: 2,
    targetOnly: 0,
    sourceCommits: [],
    forecast: { state: "notApplicable", files: [] },
    summary: { total: 0, staged: 0, unstaged: 0, untracked: 0, conflicted: 0 },
    operation: null,
    blockers: [],
    notes: [],
    canExecute: true,
    token: "t",
    ...o,
  };
}

const blocker = (kind: MergeBlocker["kind"]): MergeBlocker => ({ kind, message: "m", files: [], filesTotal: 0 });

function input(o: Partial<StageInput> = {}): StageInput {
  return {
    source: "feature/login",
    target: "main",
    preview: null,
    previewLoading: false,
    previewError: null,
    phase: "idle",
    result: null,
    runError: null,
    ...o,
  };
}

const status = {} as RepositoryStatus;

function result(o: Partial<MergeResult> = {}): MergeResult {
  return {
    repoPath: "/r",
    source: "feature/login",
    target: "main",
    outcome: "fastForward",
    sourceOid: "1111111aaaa",
    headBefore: "0000000bbbb",
    headAfter: "1111111aaaa",
    mergeCommit: false,
    integratedCommits: 2,
    conflicts: [],
    exitCode: 0,
    output: "",
    status,
    ...o,
  };
}

describe("deriveStage", () => {
  it("aguarda a seleção enquanto faltar origem/destino ou forem iguais", () => {
    expect(deriveStage(input({ source: null }))).toBe("selecting");
    expect(deriveStage(input({ target: null }))).toBe("selecting");
    expect(deriveStage(input({ source: "main", target: "main" }))).toBe("selecting");
  });

  it("valida enquanto a prévia não chegou", () => {
    expect(deriveStage(input({ previewLoading: true }))).toBe("validating");
    expect(deriveStage(input())).toBe("validating");
  });

  it("fica pronto, bloqueado ou sem nada a integrar conforme a prévia", () => {
    expect(deriveStage(input({ preview: preview() }))).toBe("ready");
    expect(
      deriveStage(input({ preview: preview({ canExecute: false, blockers: [blocker("stagedChanges")] }) })),
    ).toBe("blocked");
    expect(
      deriveStage(input({ preview: preview({ canExecute: false, relation: "alreadyIntegrated", strategy: null }) })),
    ).toBe("nothing");
    expect(deriveStage(input({ previewError: { code: "INVALID_ARGUMENT", message: "x" } }))).toBe("blocked");
  });

  it("segue a fase da confirmação e da execução", () => {
    expect(deriveStage(input({ preview: preview(), phase: "confirming" }))).toBe("confirming");
    expect(deriveStage(input({ preview: preview(), phase: "running" }))).toBe("running");
  });

  it("nunca declara sucesso quando o resultado tem conflitos", () => {
    expect(deriveStage(input({ phase: "done", result: result() }))).toBe("completed");
    expect(
      deriveStage(input({ phase: "done", result: result({ outcome: "conflicts", conflicts: [{ path: "a", code: "UU" }] }) })),
    ).toBe("conflicts");
  });

  it("mostra falha da execução", () => {
    expect(deriveStage(input({ runError: { code: "STALE_REVIEW", message: "x" } }))).toBe("failed");
  });
});

describe("selectionProblem", () => {
  it("recusa a mesma branch nos dois lados", () => {
    expect(selectionProblem("a", "a")).not.toBeNull();
    expect(selectionProblem("a", "b")).toBeNull();
    expect(selectionProblem(null, "b")).toBeNull();
  });
});

describe("confirmationReady", () => {
  it("exige o aceite explícito", () => {
    expect(confirmationReady(preview({ requiredConfirmation: null, targetProtected: false }), false, "")).toBe(false);
    expect(confirmationReady(preview({ requiredConfirmation: null, targetProtected: false }), true, "")).toBe(true);
  });

  it("exige digitar exatamente o nome da branch principal", () => {
    const p = preview();
    expect(confirmationReady(p, true, "")).toBe(false);
    expect(confirmationReady(p, true, "Main")).toBe(false);
    expect(confirmationReady(p, true, "main ")).toBe(false);
    expect(confirmationReady(p, true, "main")).toBe(true);
    expect(confirmationReady(p, false, "main")).toBe(false);
  });

  it("nunca libera quando a prévia não permite executar", () => {
    expect(confirmationReady(preview({ canExecute: false }), true, "main")).toBe(false);
    expect(confirmationReady(null, true, "main")).toBe(false);
  });
});

describe("canSwitchToTarget", () => {
  it("oferece a troca só quando o destino não atual é o único impedimento", () => {
    expect(canSwitchToTarget(preview({ canExecute: false, blockers: [blocker("targetNotCurrent")] }))).toBe(true);
    expect(
      canSwitchToTarget(
        preview({ canExecute: false, blockers: [blocker("detachedHead"), blocker("targetNotCurrent")] }),
      ),
    ).toBe(true);
    expect(
      canSwitchToTarget(
        preview({ canExecute: false, blockers: [blocker("stagedChanges"), blocker("targetNotCurrent")] }),
      ),
    ).toBe(false);
    expect(canSwitchToTarget(preview())).toBe(false);
  });
});

describe("textos", () => {
  it("descreve a direção e a estratégia", () => {
    expect(relationSummary(preview()).title).toBe("Avanço rápido");
    const diverged = relationSummary(
      preview({ relation: "diverged", strategy: "mergeCommit", sourceOnly: 1, targetOnly: 3 }),
    );
    expect(diverged.title).toContain("commit de merge");
    expect(diverged.description).toContain("1 commit em feature/login");
    expect(diverged.description).toContain("3 commits em main");
    expect(relationSummary(preview({ relation: "unrelated" })).tone).toBe("danger");
  });

  it("a previsão de conflito é apresentada sem presumir quando indisponível", () => {
    expect(forecastText(preview())).toBeNull();
    expect(forecastText(preview({ forecast: { state: "clean", files: [] } }))).toContain("nenhum conflito");
    expect(forecastText(preview({ forecast: { state: "conflicts", files: ["a", "b"] } }))).toContain("2 arquivos");
    expect(forecastText(preview({ forecast: { state: "unavailable", files: [] } }))).toContain("Conflitos só");
  });

  it("o resultado com conflito diz que o merge NÃO foi concluído", () => {
    const text = resultSummary(
      result({ outcome: "conflicts", conflicts: [{ path: "a.txt", code: "UU" }], headAfter: "0000000bbbb" }),
    );
    expect(text.tone).toBe("warning");
    expect(text.title).toContain("NÃO concluído");
    expect(text.description).toContain("manualmente");
  });

  it("o resultado diferencia avanço rápido de commit de merge", () => {
    expect(resultSummary(result()).title).toContain("avanço rápido");
    const merge = resultSummary(result({ outcome: "mergeCommit", mergeCommit: true, headAfter: "9999999cccc" }));
    expect(merge.title).toContain("commit de merge");
    expect(merge.description).toContain("9999999");
  });
});
