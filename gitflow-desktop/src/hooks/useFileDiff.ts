import { useCallback, useEffect, useState } from "react";
import { selectionKey, type Selection } from "../lib/changes";
import { errorFrom, gitService } from "../services/gitService";
import type { FileDiff, GitError } from "../types/git";

export interface DiffState {
  data: FileDiff | null;
  loading: boolean;
  error: GitError | null;
}

interface Internal extends DiffState {
  /** Identifica a seleção a que estes dados pertencem. */
  key: string | null;
}

const IDLE: DiffState = { data: null, loading: false, error: null };
const LOADING: DiffState = { data: null, loading: true, error: null };

/**
 * Carrega o diff do arquivo selecionado.
 *
 * - `refreshToken` muda quando o status é atualizado, para reler o diff do arquivo aberto;
 * - respostas de seleções anteriores são descartadas (cancelamento + conferência de chave);
 * - ao atualizar o mesmo arquivo, o diff anterior permanece visível até chegar o novo.
 */
export function useFileDiff(
  repoPath: string | null,
  selection: Selection | null,
  refreshToken: number | null,
) {
  const [state, setState] = useState<Internal>({ ...IDLE, key: null });
  const [retryCount, setRetryCount] = useState(0);

  const path = selection?.path ?? null;
  const target = selection?.target ?? null;
  const origPath = selection?.origPath ?? null;
  const key = repoPath && path && target ? `${repoPath}\u0000${selectionKey({ path, target })}` : null;

  useEffect(() => {
    if (!repoPath || !path || !target || !key) {
      setState({ ...IDLE, key: null });
      return;
    }
    let cancelled = false;
    setState((prev) => ({
      key,
      data: prev.key === key ? prev.data : null,
      loading: true,
      error: null,
    }));

    gitService
      .getFileDiff({ repoPath, path, origPath, target })
      .then((data) => {
        if (cancelled) return;
        if (data.repoPath !== repoPath || data.path !== path || data.target !== target) {
          setState({
            key,
            data: null,
            loading: false,
            error: {
              code: "FRONTEND",
              message: "A resposta recebida não corresponde ao arquivo selecionado.",
            },
          });
          return;
        }
        setState({ key, data, loading: false, error: null });
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setState({ key, data: null, loading: false, error: errorFrom(error) });
      });

    return () => {
      cancelled = true;
    };
  }, [repoPath, path, target, origPath, key, refreshToken, retryCount]);

  const retry = useCallback(() => setRetryCount((n) => n + 1), []);

  // Nunca expõe dados que pertençam a outra seleção.
  const visible: DiffState =
    key === null ? IDLE : state.key === key ? state : LOADING;

  return { diff: visible, retry };
}
