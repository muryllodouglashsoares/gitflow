import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isGitInstallationError, toGitError } from "../lib/errors";
import { selectionExists, type Selection } from "../lib/changes";
import { pickFolder } from "../services/folderPicker";
import { errorFrom, gitService } from "../services/gitService";
import type {
  GitAvailability,
  GitError,
  RepositoryInfo,
  RepositoryStatus,
} from "../types/git";

export interface StatusState {
  data: RepositoryStatus | null;
  loading: boolean;
  error: GitError | null;
  /** Momento (ms) da última leitura bem-sucedida; muda a cada atualização. */
  updatedAt: number | null;
}

export interface OpenFailure {
  path: string | null;
  error: GitError;
}

export interface GitState {
  checking: boolean;
  info: GitAvailability | null;
}

/** Resultado de uma operação de escrita executada com exclusividade. */
export type ExclusiveResult<T> = { ok: true; value: T } | { ok: false; error: GitError };

/** Executa uma operação de escrita; recusa se outra ainda estiver em andamento. */
export type RunExclusive = <T>(label: string, job: () => Promise<T>) => Promise<ExclusiveResult<T>>;

const EMPTY_STATUS: StatusState = { data: null, loading: false, error: null, updatedAt: null };

/** Evita rajadas de leituras quando a janela recupera o foco várias vezes seguidas. */
const FOCUS_REFRESH_MIN_INTERVAL_MS = 2000;

/**
 * Estado do repositório aberto.
 *
 * Proteção contra resultados obsoletos: cada abertura e cada leitura recebem um número
 * de sequência; uma resposta só é aplicada se ainda for a mais recente E se pertencer ao
 * repositório atualmente selecionado. Assim, nada de um repositório anterior (ou de uma
 * leitura mais antiga) sobrescreve a seleção atual.
 */
export function useRepositoryWorkspace() {
  const [git, setGit] = useState<GitState>({ checking: true, info: null });
  const [repo, setRepo] = useState<RepositoryInfo | null>(null);
  const [opening, setOpening] = useState(false);
  const [picking, setPicking] = useState(false);
  const [openFailure, setOpenFailure] = useState<OpenFailure | null>(null);
  const [status, setStatus] = useState<StatusState>(EMPTY_STATUS);
  const [selection, setSelection] = useState<Selection | null>(null);

  const [mutating, setMutating] = useState<string | null>(null);
  const mutatingRef = useRef<string | null>(null);

  const openSeq = useRef(0);
  const statusSeq = useRef(0);
  const repoPathRef = useRef<string | null>(null);
  const statusLoadingRef = useRef(false);
  const lastLoadStartedAt = useRef(0);

  const checkGit = useCallback(async () => {
    setGit((prev) => ({ ...prev, checking: true }));
    try {
      const info = await gitService.checkGit();
      setGit({ checking: false, info });
    } catch (error) {
      setGit({
        checking: false,
        info: { available: false, version: null, programPath: null, error: errorFrom(error) },
      });
    }
  }, []);

  useEffect(() => {
    void checkGit();
  }, [checkGit]);

  const loadStatus = useCallback(async (repoPath: string) => {
    const id = ++statusSeq.current;
    statusLoadingRef.current = true;
    lastLoadStartedAt.current = Date.now();
    // Mantém os dados atuais visíveis durante a atualização (sem piscar).
    setStatus((prev) => ({ ...prev, loading: true, error: null }));

    const isCurrent = () => id === statusSeq.current && repoPathRef.current === repoPath;

    try {
      const data = await gitService.getStatus(repoPath);
      if (!isCurrent()) return;
      statusLoadingRef.current = false;
      if (data.repoPath !== repoPath) {
        setStatus({
          data: null,
          loading: false,
          error: {
            code: "FRONTEND",
            message: "A resposta recebida não corresponde ao repositório selecionado. Atualize novamente.",
          },
          updatedAt: null,
        });
        return;
      }
      setStatus({ data, loading: false, error: null, updatedAt: Date.now() });
      // Se o arquivo selecionado não existe mais (ex.: foi commitado fora do app), limpa a seleção.
      setSelection((prev) => (prev && selectionExists(data.files, prev) ? prev : null));
    } catch (error) {
      if (!isCurrent()) return;
      statusLoadingRef.current = false;
      // Em falha não mantemos dados antigos: melhor nada do que informação possivelmente incorreta.
      setStatus({ data: null, loading: false, error: errorFrom(error), updatedAt: null });
      setSelection(null);
    }
  }, []);

  const openRepository = useCallback(
    async (path: string) => {
      const id = ++openSeq.current;
      setOpening(true);
      setOpenFailure(null);
      try {
        const info = await gitService.openRepository(path);
        if (id !== openSeq.current) return;
        repoPathRef.current = info.path;
        statusSeq.current += 1; // invalida leituras do repositório anterior
        setRepo(info);
        setSelection(null);
        setStatus({ ...EMPTY_STATUS, loading: true });
        setOpening(false);
        void loadStatus(info.path);
      } catch (error) {
        if (id !== openSeq.current) return;
        const gitError = errorFrom(error);
        setOpening(false);
        setOpenFailure({ path, error: gitError });
        if (isGitInstallationError(gitError)) void checkGit();
      }
    },
    [checkGit, loadStatus],
  );

  /**
   * Garante que só uma operação de escrita (criar branch, trocar de branch, commit) rode por vez,
   * impedindo cliques repetidos ou operações concorrentes sobre o mesmo repositório. O backend
   * também protege o repositório com uma trava própria.
   */
  const runExclusive = useCallback<RunExclusive>(async (label, job) => {
    if (mutatingRef.current !== null) {
      return {
        ok: false,
        error: {
          code: "REPOSITORY_BUSY",
          message: "Outra operação ainda está em andamento. Aguarde ela terminar e tente novamente.",
        },
      };
    }
    mutatingRef.current = label;
    setMutating(label);
    try {
      return { ok: true, value: await job() };
    } catch (error) {
      return { ok: false, error: errorFrom(error) };
    } finally {
      mutatingRef.current = null;
      setMutating(null);
    }
  }, []);

  const chooseFolder = useCallback(async () => {
    if (mutatingRef.current !== null) {
      setOpenFailure({
        path: null,
        error: {
          code: "REPOSITORY_BUSY",
          message: "Aguarde a operação em andamento terminar antes de abrir outro repositório.",
        },
      });
      return;
    }
    setPicking(true);
    try {
      const path = await pickFolder();
      if (path) await openRepository(path);
    } catch (error) {
      setOpenFailure({ path: null, error: toGitError(error) });
    } finally {
      setPicking(false);
    }
  }, [openRepository]);

  const refresh = useCallback(() => {
    const path = repoPathRef.current;
    if (path) void loadStatus(path);
  }, [loadStatus]);

  // Ao voltar para a janela (ex.: depois de editar arquivos no editor), atualiza sozinho.
  useEffect(() => {
    const onFocus = () => {
      const path = repoPathRef.current;
      if (!path || statusLoadingRef.current) return;
      if (Date.now() - lastLoadStartedAt.current < FOCUS_REFRESH_MIN_INTERVAL_MS) return;
      void loadStatus(path);
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [loadStatus]);

  const activeSelection = useMemo(
    () => (status.data && selection && selectionExists(status.data.files, selection) ? selection : null),
    [status.data, selection],
  );

  return {
    git,
    checkGit,
    repo,
    status,
    selection: activeSelection,
    select: setSelection,
    busy: opening || picking,
    /** Nome da operação de escrita em andamento (ou `null`). */
    mutating,
    runExclusive,
    opening,
    openFailure,
    dismissOpenFailure: useCallback(() => setOpenFailure(null), []),
    chooseFolder,
    refresh,
  };
}
