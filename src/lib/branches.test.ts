import { describe, expect, it } from "vitest";
import { quickBranchNameProblem, sortBranches, suggestBase, switchBlock, untrackedCount } from "./branches";
import type { BranchEntry, BranchList, FileEntry, RepositoryStatus } from "../types/git";

const entry = (name: string, current = false): BranchEntry => ({
  name,
  current,
  oid: "abcdef1234567",
  upstream: null,
  subject: null,
});

function file(o: Partial<FileEntry>): FileEntry {
  return {
    path: "a.txt",
    origPath: null,
    index: null,
    worktree: null,
    untracked: false,
    conflicted: false,
    conflictCode: null,
    submodule: false,
    ...o,
  };
}

function status(files: FileEntry[], o: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    repoPath: "/r",
    branch: { name: "main", detached: false, unborn: false, headOid: "1", upstream: null, ahead: null, behind: null },
    files,
    summary: {
      total: files.length,
      staged: files.filter((f) => f.index !== null).length,
      unstaged: files.filter((f) => f.worktree !== null).length,
      untracked: files.filter((f) => f.untracked).length,
      conflicted: files.filter((f) => f.conflicted).length,
    },
    isClean: files.length === 0,
    operation: null,
    ...o,
  };
}

describe("sortBranches", () => {
  it("coloca a atual primeiro e ordena as demais", () => {
    const sorted = sortBranches([entry("zeta"), entry("beta"), entry("main", true), entry("alfa")]);
    expect(sorted.map((b) => b.name)).toEqual(["main", "alfa", "beta", "zeta"]);
  });
});

describe("quickBranchNameProblem", () => {
  it("rejeita nomes vazios, com espaço, com hífen inicial, HEAD e controle", () => {
    for (const name of ["", "  ", "a b", " a", "a ", "-x", "--force", "HEAD", "a\nb", "a\u0000b"]) {
      expect(quickBranchNameProblem(name)).not.toBeNull();
    }
    expect(quickBranchNameProblem("x".repeat(201))).not.toBeNull();
  });
  it("aceita nomes usuais", () => {
    for (const name of ["feature/login", "fix-1", "release_1.2", "ação"]) {
      expect(quickBranchNameProblem(name)).toBeNull();
    }
  });
});

describe("suggestBase", () => {
  const list = (current: string | null, names: string[]): BranchList => ({
    repoPath: "/r",
    current,
    detached: current === null,
    unborn: false,
    headOid: "1",
    branches: names.map((n) => entry(n, n === current)),
  });
  it("sugere a branch atual, ou a primeira se estiver desanexado", () => {
    expect(suggestBase(list("dev", ["main", "dev"]))).toBe("dev");
    expect(suggestBase(list(null, ["main", "dev"]))).toBe("main");
    expect(suggestBase(list(null, []))).toBeNull();
  });
});

describe("switchBlock", () => {
  it("permite a troca em repositório limpo", () => {
    expect(switchBlock(status([]))).toBeNull();
  });
  it("permite a troca quando só há arquivos não rastreados", () => {
    expect(switchBlock(status([file({ path: "novo.txt", untracked: true })]))).toBeNull();
    expect(untrackedCount(status([file({ untracked: true })]))).toBe(1);
  });
  it("bloqueia com alterações preparadas ou não preparadas e lista os arquivos", () => {
    const block = switchBlock(
      status([file({ path: "a.txt", worktree: "modified" }), file({ path: "b.txt", index: "added" })]),
    );
    expect(block).not.toBeNull();
    expect(block?.files).toEqual(["a.txt", "b.txt"]);
  });
  it("bloqueia com conflitos, operação em andamento e status indisponível", () => {
    expect(switchBlock(status([file({ conflicted: true })]))).not.toBeNull();
    expect(switchBlock(status([], { operation: "rebase" }))).not.toBeNull();
    expect(switchBlock(null)).not.toBeNull();
  });
});
