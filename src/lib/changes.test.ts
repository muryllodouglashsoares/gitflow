import { describe, expect, it } from "vitest";
import {
  describeBranch,
  describeTracking,
  groupChanges,
  selectionExists,
  splitPath,
  summarizeState,
} from "./changes";
import type { BranchInfo, FileEntry, RepositoryStatus } from "../types/git";

function file(overrides: Partial<FileEntry>): FileEntry {
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

const branch = (overrides: Partial<BranchInfo> = {}): BranchInfo => ({
  name: "main",
  detached: false,
  unborn: false,
  headOid: "1234567890abcdef",
  upstream: null,
  ahead: null,
  behind: null,
  ...overrides,
});

describe("groupChanges", () => {
  it("separa conflitos, preparadas, não preparadas e não rastreados nessa ordem", () => {
    const groups = groupChanges([
      file({ path: "u.txt", untracked: true }),
      file({ path: "w.txt", worktree: "modified" }),
      file({ path: "s.txt", index: "added" }),
      file({ path: "c.txt", conflicted: true, conflictCode: "UU" }),
    ]);
    expect(groups.map((g) => g.id)).toEqual(["conflicted", "staged", "unstaged", "untracked"]);
    expect(groups[1]?.rows[0]?.badge.symbol).toBe("A");
    expect(groups[2]?.rows[0]?.badge.symbol).toBe("M");
    expect(groups[3]?.rows[0]?.badge.tone).toBe("untracked");
  });

  it("mostra o mesmo arquivo nas duas áreas quando há alteração preparada e não preparada", () => {
    const groups = groupChanges([file({ path: "x.txt", index: "added", worktree: "modified" })]);
    expect(groups.map((g) => g.id)).toEqual(["staged", "unstaged"]);
    const keys = groups.flatMap((g) => g.rows.map((r) => r.key));
    expect(new Set(keys).size).toBe(2);
  });

  it("preserva o caminho original só na área preparada (renomeação)", () => {
    const groups = groupChanges([
      file({ path: "novo.txt", origPath: "antigo.txt", index: "renamed", worktree: "modified" }),
    ]);
    expect(groups.find((g) => g.id === "staged")?.rows[0]?.origPath).toBe("antigo.txt");
    expect(groups.find((g) => g.id === "unstaged")?.rows[0]?.origPath).toBeNull();
  });

  it("omite grupos vazios", () => {
    expect(groupChanges([])).toEqual([]);
  });
});

describe("selectionExists", () => {
  const files = [file({ path: "a.txt", worktree: "modified" })];
  it("reconhece seleção válida e rejeita área/arquivo inexistente", () => {
    expect(selectionExists(files, { path: "a.txt", origPath: null, target: "unstaged" })).toBe(true);
    expect(selectionExists(files, { path: "a.txt", origPath: null, target: "staged" })).toBe(false);
    expect(selectionExists(files, { path: "b.txt", origPath: null, target: "unstaged" })).toBe(false);
    expect(selectionExists(files, null)).toBe(false);
  });
});

describe("splitPath", () => {
  it("separa diretório e nome, inclusive com espaços e pasta aninhada", () => {
    expect(splitPath("src/app/main file.ts")).toEqual({ dir: "src/app/", name: "main file.ts" });
    expect(splitPath("README.md")).toEqual({ dir: "", name: "README.md" });
    expect(splitPath("aninhado/")).toEqual({ dir: "", name: "aninhado/" });
  });
});

describe("branch", () => {
  it("usa o nome real da branch e trata HEAD desanexado", () => {
    expect(describeBranch(branch({ name: "trunk" }))).toBe("trunk");
    expect(describeBranch(branch({ name: null, detached: true }))).toBe("HEAD desanexado (1234567)");
  });

  it("descreve upstream e sincronização", () => {
    expect(describeTracking(branch({ unborn: true }))).toBe("Sem commits ainda");
    expect(describeTracking(branch())).toBe("Sem branch remota associada");
    expect(
      describeTracking(branch({ upstream: "origin/main", ahead: 2, behind: 1 })),
    ).toBe("origin/main · 2 à frente, 1 atrás");
    expect(describeTracking(branch({ upstream: "origin/main", ahead: 0, behind: 0 }))).toBe(
      "origin/main · sincronizada",
    );
  });
});

describe("summarizeState", () => {
  const base: RepositoryStatus = {
    repoPath: "/r",
    branch: branch(),
    files: [],
    summary: { total: 0, staged: 0, unstaged: 0, untracked: 0, conflicted: 0 },
    isClean: true,
    operation: null,
  };
  it("resume limpo, alterado e em conflito", () => {
    expect(summarizeState(base)).toEqual({ tone: "clean", label: "Repositório limpo" });
    expect(
      summarizeState({ ...base, isClean: false, summary: { ...base.summary, total: 1 } }).label,
    ).toBe("1 arquivo alterado");
    expect(
      summarizeState({
        ...base,
        isClean: false,
        operation: "merge",
        summary: { ...base.summary, total: 2, conflicted: 2 },
      }),
    ).toEqual({ tone: "warning", label: "Merge em andamento · 2 conflitos" });
  });
});
