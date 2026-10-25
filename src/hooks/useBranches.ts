import { useCallback, useEffect, useState } from "react";
import { errorFrom, gitService } from "../services/gitService";
import type { BranchList, GitError } from "../types/git";

export interface BranchesState {
  data: BranchList | null;
  loading: boolean;
  error: GitError | null;
}

interface Internal extends BranchesState {
  /** Repositório a que estes dados pertencem. */
  key: string | null;
}

const IDLE: BranchesState = { data: null, loading: false, error: null };
const LOADING: BranchesState = { data: null, loading: true, error: null };

/**
 * Lista as branches locais do repositório.
 *
 * - só consulta o Git quando `enabled` (tela de branches aberta);
 * - recarrega quando `refreshToken` muda (ex.: depois de criar/trocar branch ou atualizar o status);
 * - descarta respostas obsoletas e nunca expõe dados de outro repositório.
 */
export function useBranches(repoPath: string | null, refreshToken: number | null, enabled: boolean) {
  const [state, setState] = useState<Internal>({ ...IDLE, key: null });
  const [retryCount, setRetryCount] = useState(0);

  useEffect(() => {
    if (!enabled || !repoPath) {
      setState({ ...IDLE, key: null });
      return;
    }
    let cancelled = false;
    setState((prev) => ({
      key: repoPath,
      data: prev.key === repoPath ? prev.data : null,
      loading: true,
      error: null,
    }));

    gitService
      .listBranches(repoPath)
      .then((data) => {
        if (cancelled) return;
        if (data.repoPath !== repoPath) {
          setState({
            key: repoPath,
            data: null,
            loading: false,
            error: {
              code: "FRONTEND",
              message: "A resposta recebida não corresponde ao repositório selecionado. Atualize novamente.",
            },
          });
          return;
        }
        setState({ key: repoPath, data, loading: false, error: null });
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setState({ key: repoPath, data: null, loading: false, error: errorFrom(error) });
      });

    return () => {
      cancelled = true;
    };
  }, [repoPath, refreshToken, enabled, retryCount]);

  const reload = useCallback(() => setRetryCount((n) => n + 1), []);

  const visible: BranchesState =
    !enabled || !repoPath ? IDLE : state.key === repoPath ? state : LOADING;

  return { branches: visible, reload };
}
