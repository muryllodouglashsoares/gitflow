import { describe, expect, it } from "vitest";
import {
  INITIAL_SYNC_OP,
  stepMessage,
  syncOpReducer,
  type SyncOpAction,
  type SyncOpResult,
  type SyncOpState,
} from "./syncOperation";
import type { FetchResult, SyncStatus } from "../types/git";

const sync: SyncStatus = {
  repoPath: "/r/a",
  branch: "main",
  headOid: "abc",
  remotes: [],
  upstream: null,
  state: "synced",
  ahead: 0,
  behind: 0,
  pullStrategy: "unspecified",
  lastFetchAt: null,
};
const fetchResult: SyncOpResult = {
  kind: "fetch",
  result: { repoPath: "/r/a", remote: "origin", updatedRefs: 1, output: "", sync } satisfies FetchResult,
};

function run(actions: SyncOpAction[], from: SyncOpState = { ...INITIAL_SYNC_OP, repoPath: "/r/a" }) {
  return actions.reduce(syncOpReducer, from);
}

describe("syncOpReducer", () => {
  it("passa de ociosa para em execução e para sucesso", () => {
    const running = run([{ type: "start", repoPath: "/r/a", kind: "fetch", seq: 1 }]);
    expect(running).toMatchObject({ phase: "running", kind: "fetch", seq: 1 });
    const done = syncOpReducer(running, { type: "succeed", repoPath: "/r/a", seq: 1, result: fetchResult });
    expect(done.phase).toBe("success");
    expect(done.result).toBe(fetchResult);
  });

  it("ignora cliques repetidos enquanto a operação está em execução", () => {
    const state = run([
      { type: "start", repoPath: "/r/a", kind: "push", seq: 1 },
      { type: "start", repoPath: "/r/a", kind: "pull", seq: 2 }, // segundo clique
      // o backend/`runExclusive` recusa o segundo clique com "ocupado": não pode apagar a operação real
      { type: "fail", repoPath: "/r/a", seq: 2, error: { code: "REPOSITORY_BUSY", message: "ocupado" } },
    ]);
    expect(state).toMatchObject({ phase: "running", kind: "push", seq: 1, error: null });
  });

  it("descarta a resposta de outro repositório", () => {
    const running = run([{ type: "start", repoPath: "/r/a", kind: "fetch", seq: 1 }]);
    const switched = syncOpReducer(running, { type: "reset", repoPath: "/r/b" });
    const late = syncOpReducer(switched, { type: "succeed", repoPath: "/r/a", seq: 1, result: fetchResult });
    expect(late.phase).toBe("idle");
    expect(late.repoPath).toBe("/r/b");
    expect(late.result).toBeNull();
  });

  it("descarta a resposta de uma tentativa antiga mesmo no mesmo repositório", () => {
    const state = run([
      { type: "start", repoPath: "/r/a", kind: "fetch", seq: 1 },
      { type: "succeed", repoPath: "/r/a", seq: 1, result: fetchResult },
      { type: "start", repoPath: "/r/a", kind: "pull", seq: 2 },
      { type: "fail", repoPath: "/r/a", seq: 1, error: { code: "NETWORK_ERROR", message: "x" } },
    ]);
    expect(state).toMatchObject({ phase: "running", kind: "pull", seq: 2, error: null });
  });

  it("não inicia operação para um repositório que não é o atual", () => {
    const state = run([{ type: "start", repoPath: "/r/outro", kind: "fetch", seq: 1 }]);
    expect(state.phase).toBe("idle");
  });

  it("falha comum vira falha; tempo esgotado vira resultado não confirmado", () => {
    const failed = run([
      { type: "start", repoPath: "/r/a", kind: "push", seq: 1 },
      { type: "fail", repoPath: "/r/a", seq: 1, error: { code: "PUSH_REJECTED", message: "recusado" } },
    ]);
    expect(failed).toMatchObject({ phase: "failure", result: null });
    expect(failed.error?.code).toBe("PUSH_REJECTED");

    const timeout = run([
      { type: "start", repoPath: "/r/a", kind: "push", seq: 1 },
      { type: "fail", repoPath: "/r/a", seq: 1, error: { code: "TIMEOUT", message: "tempo" } },
    ]);
    expect(timeout.phase).toBe("unconfirmed");
  });

  it("permite nova tentativa depois de uma falha e não fecha uma operação em execução", () => {
    const failed = run([
      { type: "start", repoPath: "/r/a", kind: "fetch", seq: 1 },
      { type: "fail", repoPath: "/r/a", seq: 1, error: { code: "NETWORK_ERROR", message: "x" } },
    ]);
    const retry = syncOpReducer(failed, { type: "start", repoPath: "/r/a", kind: "fetch", seq: 2 });
    expect(retry).toMatchObject({ phase: "running", seq: 2, error: null });
    expect(syncOpReducer(retry, { type: "dismiss" }).phase).toBe("running");
    expect(syncOpReducer(failed, { type: "dismiss" }).phase).toBe("idle");
  });

  it("usa mensagens de etapa, sem porcentagem inventada", () => {
    expect(stepMessage("fetch")).toBe("Atualizando referências remotas…");
    expect(stepMessage("push")).toBe("Enviando commits…");
    expect(stepMessage("pull")).not.toMatch(/%/);
  });
});
