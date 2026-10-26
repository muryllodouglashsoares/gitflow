import { describe, expect, it } from "vitest";
import {
  STALE_AFTER_MS,
  countsText,
  defaultRemote,
  describeSync,
  fetchBlock,
  formatAge,
  isProtectedDestination,
  pullBlock,
  pullStrategyNote,
  pushBlock,
  syncTarget,
} from "./sync";
import type { FileEntry, RepositoryStatus, SyncStatus } from "../types/git";

const NOW = 1_800_000_000_000;
const FRESH_SECONDS = Math.floor((NOW - 60_000) / 1000);
const OLD_SECONDS = Math.floor((NOW - STALE_AFTER_MS - 60_000) / 1000);

function sync(overrides: Partial<SyncStatus> = {}): SyncStatus {
  return {
    repoPath: "/r",
    branch: "feature",
    headOid: "abc",
    remotes: [{ name: "origin", fetchUrl: "https://h/x.git", pushUrl: "https://h/x.git" }],
    upstream: {
      remote: "origin",
      remoteBranch: "feature",
      display: "origin/feature",
      trackingRef: "refs/remotes/origin/feature",
      trackingRefExists: true,
    },
    state: "synced",
    ahead: 0,
    behind: 0,
    pullStrategy: "merge",
    lastFetchAt: FRESH_SECONDS,
    ...overrides,
  };
}

function file(overrides: Partial<FileEntry> = {}): FileEntry {
  return {
    path: "a.txt",
    origPath: null,
    index: null,
    worktree: null,
    untracked: false,
    conflicted: false,
    conflictCode: null,
    submodule: false,
    ...overrides,
  };
}

function status(files: FileEntry[] = [], overrides: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    repoPath: "/r",
    branch: {
      name: "feature",
      detached: false,
      unborn: false,
      headOid: "abc",
      upstream: "origin/feature",
      ahead: 0,
      behind: 0,
    },
    files,
    summary: {
      total: files.length,
      staged: files.filter((f) => f.index).length,
      unstaged: files.filter((f) => f.worktree).length,
      untracked: files.filter((f) => f.untracked).length,
      conflicted: files.filter((f) => f.conflicted).length,
    },
    isClean: files.length === 0,
    operation: null,
    ...overrides,
  };
}

const ctx = { fetchFailure: null, sessionFetchAt: null, now: NOW };

describe("describeSync", () => {
  it("distingue os estados sem upstream utilizável", () => {
    expect(describeSync(sync({ state: "noRemote", remotes: [], upstream: null }), ctx).label).toBe("Sem remoto");
    expect(describeSync(sync({ state: "noUpstream", upstream: null }), ctx).label).toBe("Sem upstream");
    expect(describeSync(sync({ state: "upstreamInvalid" }), ctx).label).toBe("Upstream inválido");
    expect(describeSync(sync({ state: "detachedHead", branch: null, upstream: null }), ctx).label).toBe("HEAD desanexado");
    expect(describeSync(sync({ state: "upstreamNotFetched", ahead: null, behind: null }), ctx).label).toBe(
      "Remoto ainda não verificado",
    );
  });

  it("só diz 'Sincronizada' com referências remotas recentes", () => {
    expect(describeSync(sync(), ctx)).toMatchObject({ tone: "ok", label: "Sincronizada" });
    const old = describeSync(sync({ lastFetchAt: OLD_SECONDS }), ctx);
    expect(old.label).toBe("Possivelmente desatualizada");
    expect(old.tone).toBe("warning");
    const never = describeSync(sync({ lastFetchAt: null }), ctx);
    expect(never.label).toBe("Possivelmente desatualizada");
    expect(never.description).toContain("Nenhuma atualização");
  });

  it("um fetch feito nesta sessão conta como atualização recente", () => {
    const summary = describeSync(sync({ lastFetchAt: null }), { ...ctx, sessionFetchAt: NOW - 5_000 });
    expect(summary.label).toBe("Sincronizada");
  });

  it("descreve adiantada, atrasada e divergente; avisa quando as contagens podem estar velhas", () => {
    expect(describeSync(sync({ state: "ahead", ahead: 2 }), ctx).label).toBe("Adiantada");
    expect(describeSync(sync({ state: "behind", behind: 3 }), ctx).label).toBe("Atrasada");
    const diverged = describeSync(sync({ state: "diverged", ahead: 1, behind: 1, lastFetchAt: OLD_SECONDS }), ctx);
    expect(diverged.label).toBe("Divergente");
    expect(diverged.description).toContain("fetch");
  });

  it("falha recente de rede ou credenciais torna o remoto não verificável", () => {
    const summary = describeSync(sync({ state: "ahead", ahead: 1 }), {
      ...ctx,
      fetchFailure: { code: "AUTH_FAILED", message: "credenciais inválidas" },
    });
    expect(summary).toMatchObject({ tone: "danger", label: "Remoto não verificável" });
    expect(summary.description).toContain("credenciais inválidas");
  });
});

describe("helpers", () => {
  it("formata contagens, idade e destinos protegidos", () => {
    expect(countsText(sync({ ahead: 2, behind: 1 }))).toBe("2 a enviar · 1 a receber");
    expect(countsText(sync({ ahead: null, behind: null }))).toBeNull();
    expect(formatAge(30_000)).toBe("agora há pouco");
    expect(formatAge(5 * 60_000)).toBe("há 5 min");
    expect(formatAge(3 * 3_600_000)).toBe("há 3 h");
    expect(formatAge(2 * 86_400_000)).toBe("há 2 dias");
    expect(isProtectedDestination("main")).toBe(true);
    expect(isProtectedDestination("master")).toBe(true);
    expect(isProtectedDestination("feature")).toBe(false);
  });

  it("escolhe o remoto padrão e o destino exato", () => {
    expect(defaultRemote(null)).toBeNull();
    expect(defaultRemote(sync({ remotes: [] }))).toBeNull();
    const two = sync({
      remotes: [
        { name: "fork", fetchUrl: null, pushUrl: null },
        { name: "origin", fetchUrl: null, pushUrl: null },
      ],
      upstream: null,
    });
    expect(defaultRemote(two)).toBe("origin");
    expect(syncTarget(sync())).toEqual({ branch: "feature", remote: "origin", remoteBranch: "feature" });
    expect(syncTarget(sync({ state: "noUpstream", upstream: null }))).toBeNull();
    expect(syncTarget(sync({ state: "upstreamInvalid" }))).toBeNull();
  });

  it("explica a estratégia de pull", () => {
    expect(pullStrategyNote(sync({ pullStrategy: "unspecified" }))).toContain("avanço rápido");
    expect(pullStrategyNote(sync({ pullStrategy: "merge" }))).toContain("merge");
    expect(pullStrategyNote(sync({ pullStrategy: "rebase" }))).toContain("rebase");
  });
});

describe("bloqueio de ações incompatíveis", () => {
  it("fetch exige um remoto configurado e escolhido", () => {
    expect(fetchBlock(null, "origin")?.reason).toContain("diagnóstico");
    expect(fetchBlock(sync({ remotes: [] }), "origin")?.reason).toContain("Sem remoto");
    expect(fetchBlock(sync(), null)?.reason).toContain("Escolha");
    expect(fetchBlock(sync(), "outro")?.reason).toContain("Escolha");
    expect(fetchBlock(sync({ state: "noUpstream", upstream: null }), "origin")).toBeNull();
  });

  it("pull é bloqueado sem upstream, durante operação/conflito e com alterações rastreadas", () => {
    expect(pullBlock(sync(), status())).toBeNull();
    expect(pullBlock(sync({ state: "noUpstream", upstream: null }), status())?.reason).toContain("upstream");
    expect(pullBlock(sync({ state: "noRemote", remotes: [], upstream: null }), status())?.reason).toContain("remoto");
    expect(pullBlock(sync(), status([], { operation: "merge" }))?.reason).toContain("Merge em andamento");
    expect(pullBlock(sync(), status([file({ conflicted: true })]))?.reason).toContain("conflito");

    const dirty = pullBlock(sync(), status([file({ path: "x.ts", worktree: "modified" })]));
    expect(dirty?.reason).toContain("não commitadas");
    expect(dirty?.files).toEqual(["x.ts"]);
    expect(pullBlock(sync(), status([file({ path: "y.ts", index: "added" })]))?.files).toEqual(["y.ts"]);
  });

  it("pull não é bloqueado por arquivos apenas não rastreados", () => {
    expect(pullBlock(sync(), status([file({ path: "novo.txt", untracked: true })]))).toBeNull();
  });

  it("pull divergente depende da estratégia configurada", () => {
    const base = { state: "diverged", ahead: 1, behind: 1 } as const;
    expect(pullBlock(sync({ ...base, pullStrategy: "unspecified" }), status())?.reason).toContain("pull.rebase");
    expect(pullBlock(sync({ ...base, pullStrategy: "fastForwardOnly" }), status())?.reason).toContain("avanço rápido");
    expect(pullBlock(sync({ ...base, pullStrategy: "merge" }), status())).toBeNull();
    expect(pullBlock(sync({ ...base, pullStrategy: "rebase" }), status())).toBeNull();
  });

  it("push só é liberado quando há commits locais e o remoto não está à frente", () => {
    expect(pushBlock(sync({ state: "ahead", ahead: 1 }), status())).toBeNull();
    expect(pushBlock(sync({ state: "synced" }), status())?.reason).toContain("Não há commits locais");
    expect(pushBlock(sync({ state: "behind", behind: 1 }), status())?.reason).toContain("pull");
    expect(pushBlock(sync({ state: "diverged", ahead: 1, behind: 1 }), status())?.reason).toContain("divergiu");
    expect(pushBlock(sync({ state: "upstreamNotFetched", ahead: null, behind: null }), status())?.reason).toContain("fetch");
    expect(pushBlock(sync({ state: "noUpstream", upstream: null }), status())?.reason).toContain("upstream");
    expect(pushBlock(sync({ state: "detachedHead", branch: null }), status())?.reason).toContain("desanexado");
    expect(pushBlock(sync({ state: "ahead", ahead: 1 }), status([], { operation: "rebase" }))?.reason).toContain("Rebase");
  });

  it("push não é bloqueado por alterações locais não commitadas (só commits são enviados)", () => {
    expect(
      pushBlock(sync({ state: "ahead", ahead: 1 }), status([file({ worktree: "modified" })])),
    ).toBeNull();
  });
});
