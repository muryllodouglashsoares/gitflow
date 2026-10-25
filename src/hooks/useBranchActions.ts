import { useCallback, useEffect, useRef, useState } from "react";
import { gitService } from "../services/gitService";
import type { CreatedBranch, GitError, SwitchResult } from "../types/git";
import type { RunExclusive } from "./useRepositoryWorkspace";

export type BranchOutcome =
  | { kind: "created"; result: CreatedBranch }
  | { kind: "switched"; result: SwitchResult };

export type BranchPending = { kind: "create" } | { kind: "switch"; name: string } | null;

/**
 * Criar e trocar de branch. Cada ação é separada e explícita: criar NÃO troca de branch.
 * A interface só mostra sucesso depois que o backend confirma o resultado real do Git.
 */
export function useBranchActions(
  repoPath: string | null,
  runExclusive: RunExclusive,
  onChanged: () => void,
) {
  const [pending, setPending] = useState<BranchPending>(null);
  const [outcome, setOutcome] = useState<BranchOutcome | null>(null);
  const [error, setError] = useState<GitError | null>(null);

  const repoRef = useRef(repoPath);
  useEffect(() => {
    repoRef.current = repoPath;
    setOutcome(null);
    setError(null);
    setPending(null);
  }, [repoPath]);

  const createBranch = useCallback(
    async (name: string, base: string): Promise<boolean> => {
      if (!repoPath) return false;
      setPending({ kind: "create" });
      setError(null);
      setOutcome(null);
      const res = await runExclusive("create-branch", () =>
        gitService.createBranch({ repoPath, name, base }),
      );
      if (repoRef.current !== repoPath) return false;
      setPending(null);
      onChanged();
      if (res.ok) {
        setOutcome({ kind: "created", result: res.value });
        return true;
      }
      setError(res.error);
      return false;
    },
    [repoPath, runExclusive, onChanged],
  );

  const switchTo = useCallback(
    async (name: string): Promise<boolean> => {
      if (!repoPath) return false;
      setPending({ kind: "switch", name });
      setError(null);
      setOutcome(null);
      const res = await runExclusive("switch-branch", () =>
        gitService.switchBranch({ repoPath, name }),
      );
      if (repoRef.current !== repoPath) return false;
      setPending(null);
      onChanged();
      if (res.ok) {
        setOutcome({ kind: "switched", result: res.value });
        return true;
      }
      setError(res.error);
      return false;
    },
    [repoPath, runExclusive, onChanged],
  );

  const dismiss = useCallback(() => {
    setOutcome(null);
    setError(null);
  }, []);

  return { pending, outcome, error, createBranch, switchTo, dismiss };
}
