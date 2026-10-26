import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { isRemoteUnverifiable } from "../lib/errors";
import { defaultRemote } from "../lib/sync";
import {
  INITIAL_SYNC_OP,
  syncOpReducer,
  type SyncOpKind,
  type SyncOpResult,
} from "../lib/syncOperation";
import { errorFrom, gitService } from "../services/gitService";
import type { GitError, SyncStatus, SyncTarget } from "../types/git";
import type { RunExclusive } from "./useRepositoryWorkspace";

export interface SyncDiagnosis {
  data: SyncStatus | null;
  loading: boolean;
  error: GitError | null;
}

interface Internal extends SyncDiagnosis {
  /** Repositório a que estes dados pertencem. */
  key: string | null;
}

const IDLE: SyncDiagnosis = { data: null, loading: false, error: null };
const LOADING: SyncDiagnosis = { data: null, loading: true, error: null };

/**
 * Sincronização remota do repositório aberto.
 *
 * - Diagnóstico (somente leitura, sem rede): recarregado quando `refreshToken` muda (depois de
 *   commit, troca de branch, atualização do status…) e descarta respostas obsoletas.
 * - Operações (fetch/pull/push): passam por `runExclusive` (uma operação de escrita por vez) e
 *   pelo reducer `syncOpReducer`, que ignora cliques repetidos e respostas de outro repositório
 *   ou de tentativas antigas.
 * - Nada é "otimista": depois de cada operação a tela usa o diagnóstico devolvido pelo Git e o
 *   status do repositório é lido de novo.
 */
export function useSync(
  repoPath: string | null,
  refreshToken: number | null,
  runExclusive: RunExclusive,
  onChanged: () => void,
) {
  const [diag, setDiag] = useState<Internal>({ ...IDLE, key: null });
  const [retryCount, setRetryCount] = useState(0);
  const [op, dispatch] = useReducer(syncOpReducer, INITIAL_SYNC_OP);
  const [fetchFailure, setFetchFailure] = useState<GitError | null>(null);
  const [sessionFetchAt, setSessionFetchAt] = useState<number | null>(null);
  const [remoteChoice, setRemoteChoice] = useState<string | null>(null);

  const seqRef = useRef(0);
  const repoRef = useRef(repoPath);

  // Trocar de repositório zera tudo o que pertence ao anterior.
  useEffect(() => {
    repoRef.current = repoPath;
    dispatch({ type: "reset", repoPath });
    setFetchFailure(null);
    setSessionFetchAt(null);
    setRemoteChoice(null);
  }, [repoPath]);

  useEffect(() => {
    if (!repoPath) {
      setDiag({ ...IDLE, key: null });
      return;
    }
    let cancelled = false;
    setDiag((prev) => ({
      key: repoPath,
      data: prev.key === repoPath ? prev.data : null,
      loading: true,
      error: null,
    }));

    gitService
      .getSyncStatus(repoPath)
      .then((data) => {
        if (cancelled) return;
        if (data.repoPath !== repoPath) {
          setDiag({
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
        setDiag({ key: repoPath, data, loading: false, error: null });
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setDiag({ key: repoPath, data: null, loading: false, error: errorFrom(error) });
      });

    return () => {
      cancelled = true;
    };
  }, [repoPath, refreshToken, retryCount]);

  const run = useCallback(
    async (path: string, kind: SyncOpKind, job: () => Promise<SyncOpResult>) => {
      const seq = ++seqRef.current;
      dispatch({ type: "start", repoPath: path, kind, seq });
      const res = await runExclusive(kind, job);
      if (repoRef.current !== path) return; // o repositório mudou: a resposta é descartada
      if (res.ok) {
        dispatch({ type: "succeed", repoPath: path, seq, result: res.value });
        const fresh = res.value.result.sync;
        setDiag((prev) => (prev.key === path ? { key: path, data: fresh, loading: false, error: null } : prev));
        setFetchFailure(null);
        if (kind !== "push") setSessionFetchAt(Date.now());
      } else {
        dispatch({ type: "fail", repoPath: path, seq, error: res.error });
        if (isRemoteUnverifiable(res.error)) setFetchFailure(res.error);
      }
      onChanged();
    },
    [runExclusive, onChanged],
  );

  const fetch = useCallback(
    (remote: string) => {
      if (!repoPath) return;
      void run(repoPath, "fetch", () =>
        gitService
          .fetchRemote({ repoPath, remote })
          .then((result): SyncOpResult => ({ kind: "fetch", result })),
      );
    },
    [repoPath, run],
  );

  const pull = useCallback(
    (target: SyncTarget) => {
      if (!repoPath) return;
      void run(repoPath, "pull", () =>
        gitService
          .pullBranch({ repoPath, target })
          .then((result): SyncOpResult => ({ kind: "pull", result })),
      );
    },
    [repoPath, run],
  );

  const push = useCallback(
    (target: SyncTarget, confirmProtectedBranch: boolean) => {
      if (!repoPath) return;
      void run(repoPath, "push", () =>
        gitService
          .pushBranch({ repoPath, target, confirmProtectedBranch })
          .then((result): SyncOpResult => ({ kind: "push", result })),
      );
    },
    [repoPath, run],
  );

  const visible: SyncDiagnosis =
    !repoPath ? IDLE : diag.key === repoPath ? diag : LOADING;

  const remotes = visible.data?.remotes ?? [];
  const selectedRemote =
    remoteChoice && remotes.some((r) => r.name === remoteChoice)
      ? remoteChoice
      : defaultRemote(visible.data);

  return {
    diagnosis: visible,
    reload: useCallback(() => setRetryCount((n) => n + 1), []),
    op,
    fetchFailure,
    sessionFetchAt,
    selectedRemote,
    selectRemote: setRemoteChoice,
    fetch,
    pull,
    push,
    dismiss: useCallback(() => dispatch({ type: "dismiss" }), []),
  };
}

export type SyncController = ReturnType<typeof useSync>;
