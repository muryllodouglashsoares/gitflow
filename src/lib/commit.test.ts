import { describe, expect, it } from "vitest";
import {
  commitBlocker,
  commitEligibility,
  eligiblePaths,
  fileMarks,
  pruneSelection,
  sameSet,
  stagedOutsideSelection,
  subjectWarning,
  toggleInSet,
  validateCommitMessage,
} from "./commit";
import type { BranchInfo, FileEntry, RepositoryStatus } from "../types/git";

function file(overrides: Partial<FileEntry>): FileEntry {
  return {
    path: "a.txt",
    origPath: null,
    index: null,
    worktree: "modified",
    untracked: false,
    conflicted: false,
    conflictCode: null,
    submodule: false,
    ...overrides,
  };
}

const branch = (o: Partial<BranchInfo> = {}): BranchInfo => ({
  name: "main",
  detached: false,
  unborn: false,
  headOid: "1234567890abcdef",
  upstream: null,
  ahead: null,
  behind: null,
  ...o,
});

function status(o: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    repoPath: "/r",
    branch: branch(),
    files: [],
    summary: { total: 0, staged: 0, unstaged: 0, untracked: 0, conflicted: 0 },
    isClean: true,
    operation: null,
    ...o,
  };
}

describe("commitEligibility", () => {
  it("aceita arquivos comuns, novos e removidos", () => {
    expect(commitEligibility(file({})).eligible).toBe(true);
    expect(commitEligibility(file({ untracked: true, worktree: null })).eligible).toBe(true);
    expect(commitEligibility(file({ worktree: "deleted" })).eligible).toBe(true);
  });

  it("bloqueia conflitos, submódulos e pastas/repositórios aninhados com um motivo", () => {
    for (const f of [
      file({ conflicted: true, worktree: null }),
      file({ submodule: true }),
      file({ path: "aninhado/", untracked: true, worktree: null }),
    ]) {
      const result = commitEligibility(f);
      expect(result.eligible).toBe(false);
      expect(typeof result.reason).toBe("string");
    }
  });
});

describe("seleção", () => {
  const files = [
    file({ path: "a.txt" }),
    file({ path: "c.txt", conflicted: true, worktree: null }),
    file({ path: "n ovo.txt", untracked: true, worktree: null }),
  ];

  it("lista somente caminhos elegíveis", () => {
    expect(eligiblePaths(files)).toEqual(["a.txt", "n ovo.txt"]);
  });

  it("remove da seleção o que sumiu ou deixou de ser elegível", () => {
    const next = pruneSelection(new Set(["a.txt", "c.txt", "sumiu.txt"]), files);
    expect([...next]).toEqual(["a.txt"]);
  });

  it("alterna sem mutar o conjunto original", () => {
    const original = new Set(["a.txt"]);
    const added = toggleInSet(original, "b.txt");
    expect(original.size).toBe(1);
    expect(sameSet(added, new Set(["a.txt", "b.txt"]))).toBe(true);
    expect(toggleInSet(added, "a.txt").has("a.txt")).toBe(false);
  });

  it("identifica arquivos preparados fora da seleção (que serão preservados)", () => {
    const list = [
      file({ path: "s1.txt", index: "modified", worktree: null }),
      file({ path: "s2.txt", index: "added", worktree: null }),
      file({ path: "w.txt" }),
    ];
    expect(stagedOutsideSelection(list, new Set(["s1.txt"]))).toEqual(["s2.txt"]);
  });

  it("marca arquivos com alterações preparadas e não preparadas", () => {
    expect(fileMarks(file({ index: "added", worktree: "modified" })).partial).toBe(true);
    expect(fileMarks(file({ index: null, worktree: "modified" })).partial).toBe(false);
    expect(fileMarks(file({ untracked: true, worktree: null })).untracked).toBe(true);
  });
});

describe("validateCommitMessage", () => {
  it("rejeita vazia ou só com espaços", () => {
    expect(validateCommitMessage("")).not.toBeNull();
    expect(validateCommitMessage("   \n\t ")).not.toBeNull();
  });
  it("aceita mensagens válidas, inclusive várias linhas", () => {
    expect(validateCommitMessage("ajusta a tela")).toBeNull();
    expect(validateCommitMessage("título\n\ncorpo")).toBeNull();
  });
  it("rejeita NUL e mensagens longas demais", () => {
    expect(validateCommitMessage("a\u0000b")).not.toBeNull();
    expect(validateCommitMessage("x".repeat(8001))).not.toBeNull();
  });
  it("avisa (sem bloquear) quando a primeira linha é longa", () => {
    expect(subjectWarning("curta")).toBeNull();
    expect(subjectWarning("x".repeat(80))).not.toBeNull();
  });
});

describe("commitBlocker", () => {
  it("não bloqueia repositório normal, nem com alterações", () => {
    expect(commitBlocker(status())).toBeNull();
  });
  it("bloqueia operação em andamento, conflitos e HEAD desanexado", () => {
    expect(commitBlocker(status({ operation: "merge" }))).not.toBeNull();
    expect(
      commitBlocker(status({ summary: { total: 1, staged: 0, unstaged: 0, untracked: 0, conflicted: 1 } })),
    ).not.toBeNull();
    expect(commitBlocker(status({ branch: branch({ name: null, detached: true }) }))).not.toBeNull();
    expect(commitBlocker(null)).not.toBeNull();
  });
});
