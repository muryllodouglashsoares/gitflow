import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  commitBlocker,
  pruneSelection,
  sameSet,
  toggleInSet,
  validateCommitMessage,
} from "../lib/commit";
import type { Selection } from "../lib/changes";
import { errorFrom, gitService } from "../services/gitService";
import type { CommitResult, CommitReview, GitError } from "../types/git";
import type { RunExclusive, StatusState } from "./useRepositoryWorkspace";

export type CommitStep = "select" | "review";

export interface ReviewState {
  data: CommitReview | null;
  loading: boolean;
  error: GitError | null;
}

const EMPTY_REVIEW: ReviewState = { data: null, loading: false, error: null };

interface Params {
  repoPath: string | null;
  status: StatusState;
  runExclusive: RunExclusive;
  /** Relê o status do repositório. */
  refresh: () => void;
}

/**
 * Seleção explícita de arquivos, revisão do que será commitado e criação do commit.
 *
 * Garantias:
 * - nada entra no commit sem estar marcado; a seleção é a única fonte dos arquivos;
 * - o commit só é habilitado depois de uma revisão carregada e de uma confirmação explícita;
 * - qualquer mudança de conteúdo/branch desde a revisão derruba a confirmação e exige nova revisão
 *   (o backend também recusa o commit se a impressão digital não bater);
 * - em falha, seleção e mensagem do usuário são preservadas;
 * - o sucesso só é exibido com o resultado real devolvido pelo Git.
 */
export function useCommitFlow({ repoPath, status, runExclusive, refresh }: Params) {
  const files = status.data?.files ?? null;

  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [step, setStep] = useState<CommitStep>("select");
  const [review, setReview] = useState<ReviewState>(EMPTY_REVIEW);
  const [reviewVersion, setReviewVersion] = useState(0);
  const [focusPath, setFocusPath] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [committing, setCommitting] = useState(false);
  const [result, setResult] = useState<CommitResult | null>(null);
  const [commitError, setCommitError] = useState<GitError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const repoRef = useRef(repoPath);
  const reviewSeq = useRef(0);
  const stepRef = useRef<CommitStep>("select");
  const reviewPathsRef = useRef<string[]>([]);
  const reviewedBranchRef = useRef<string | null>(null);
  const reviewedFingerprintRef = useRef<string | null>(null);
  const handledStatusAt = useRef<number | null>(null);
  const committingRef = useRef(false);

  const resetAll = useCallback(() => {
    reviewSeq.current += 1;
    stepRef.current = "select";
    reviewPathsRef.current = [];
    reviewedBranchRef.current = null;
    reviewedFingerprintRef.current = null;
    setSelected(new Set());
    setStep("select");
    setReview(EMPTY_REVIEW);
    setFocusPath(null);
    setMessage("");
    setConfirmed(false);
    setCommitting(false);
    setResult(null);
    setCommitError(null);
    setNotice(null);
  }, []);

  // Troca de repositório: nada da seleção/revisão anterior sobrevive.
  useEffect(() => {
    repoRef.current = repoPath;
    resetAll();
  }, [repoPath, resetAll]);

  // Mantém só arquivos que continuam existindo e elegíveis.
  useEffect(() => {
    if (!files) return;
    setSelected((prev) => {
      const next = pruneSelection(prev, files);
      return sameSet(prev, next) ? prev : next;
    });
  }, [files]);

  const loadReview = useCallback(
    async (paths: string[], silent: boolean) => {
      if (!repoPath) return;
      const id = ++reviewSeq.current;
      setReview((prev) => ({ data: silent ? prev.data : null, loading: true, error: null }));
      try {
        const data = await gitService.prepareCommitReview({ repoPath, files: paths });
        if (id !== reviewSeq.current || repoRef.current !== repoPath) return;
        if (data.repoPath !== repoPath) {
          setReview({
            data: null,
            loading: false,
            error: { code: "FRONTEND", message: "A resposta recebida não corresponde ao repositório selecionado." },
          });
          return;
        }
        const previousBranch = reviewedBranchRef.current;
        const changed =
          reviewedFingerprintRef.current !== null && reviewedFingerprintRef.current !== data.fingerprint;
        reviewedFingerprintRef.current = data.fingerprint;
        reviewedBranchRef.current = data.branch;
        setReview({ data, loading: false, error: null });
        setReviewVersion((v) => v + 1);
        setFocusPath((prev) =>
          prev && data.files.some((f) => f.path === prev) ? prev : (data.files[0]?.path ?? null),
        );
        if (changed) {
          setConfirmed(false);
          setNotice(
            previousBranch !== null && previousBranch !== data.branch
              ? `A branch ativa mudou de “${previousBranch}” para “${data.branch}” desde a revisão. Revise novamente.`
              : "O conteúdo dos arquivos mudou desde a revisão. O diff foi atualizado: revise e confirme novamente.",
          );
        }
      } catch (error) {
        if (id !== reviewSeq.current || repoRef.current !== repoPath) return;
        // Sem revisão válida não há commit possível.
        setReview({ data: null, loading: false, error: errorFrom(error) });
        setConfirmed(false);
      }
    },
    [repoPath],
  );

  const startReview = useCallback(() => {
    if (selected.size === 0) return;
    const paths = [...selected].sort();
    reviewPathsRef.current = paths;
    reviewedBranchRef.current = null;
    reviewedFingerprintRef.current = null;
    handledStatusAt.current = status.updatedAt;
    stepRef.current = "review";
    setStep("review");
    setNotice(null);
    setCommitError(null);
    setResult(null);
    setConfirmed(false);
    setFocusPath(null);
    void loadReview(paths, false);
  }, [selected, status.updatedAt, loadReview]);

  // Se o status foi atualizado durante a revisão (ex.: voltou o foco para a janela),
  // a revisão é refeita, para nunca mostrar um diff antigo como se fosse o atual.
  useEffect(() => {
    if (stepRef.current !== "review" || committingRef.current) return;
    if (handledStatusAt.current === status.updatedAt) return;
    handledStatusAt.current = status.updatedAt;
    void loadReview(reviewPathsRef.current, true);
  }, [status.updatedAt, loadReview]);

  const backToSelection = useCallback(() => {
    reviewSeq.current += 1;
    stepRef.current = "select";
    reviewedFingerprintRef.current = null;
    reviewedBranchRef.current = null;
    setStep("select");
    setReview(EMPTY_REVIEW);
    setFocusPath(null);
    setConfirmed(false);
    setCommitError(null);
    setNotice(null);
  }, []);

  const toggle = useCallback((path: string) => {
    setSelected((prev) => toggleInSet(prev, path));
  }, []);

  const setMany = useCallback((paths: readonly string[], on: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      for (const path of paths) {
        if (on) next.add(path);
        else next.delete(path);
      }
      return next;
    });
  }, []);

  const clearSelection = useCallback(() => setSelected(new Set()), []);

  const changeMessage = useCallback((value: string) => {
    setMessage(value);
  }, []);

  const messageProblem = validateCommitMessage(message);
  const canSubmit =
    step === "review" &&
    review.data !== null &&
    !review.loading &&
    !committing &&
    confirmed &&
    messageProblem === null;

  const submit = useCallback(async () => {
    const data = review.data;
    if (!repoPath || !data || committingRef.current || !canSubmit) return;
    committingRef.current = true;
    setCommitting(true);
    setCommitError(null);
    setNotice(null);

    const paths = data.files.map((f) => f.path);
    const outcome = await runExclusive("commit", () =>
      gitService.createCommit({
        repoPath,
        files: paths,
        message,
        expectedBranch: data.branch,
        fingerprint: data.fingerprint,
      }),
    );
    committingRef.current = false;
    if (repoRef.current !== repoPath) return;
    setCommitting(false);

    if (outcome.ok) {
      // Sucesso confirmado pelo Git: só agora a seleção e a mensagem são limpas.
      reviewSeq.current += 1;
      stepRef.current = "select";
      reviewedFingerprintRef.current = null;
      reviewedBranchRef.current = null;
      setResult(outcome.value);
      setSelected(new Set());
      setStep("select");
      setReview(EMPTY_REVIEW);
      setFocusPath(null);
      setMessage("");
      setConfirmed(false);
      refresh();
      return;
    }

    // Falha: nada é descartado (seleção e mensagem permanecem).
    setConfirmed(false);
    if (outcome.error.code === "STALE_REVIEW") {
      setNotice(outcome.error.message);
      void loadReview(paths, true);
    } else {
      setCommitError(outcome.error);
      refresh();
    }
  }, [review.data, repoPath, canSubmit, runExclusive, message, loadReview, refresh]);

  const dismissResult = useCallback(() => setResult(null), []);
  const dismissCommitError = useCallback(() => setCommitError(null), []);

  const reviewSelection: Selection | null = useMemo(() => {
    const file = review.data?.files.find((f) => f.path === focusPath);
    return file ? { path: file.path, origPath: file.origPath, target: file.diffTarget } : null;
  }, [review.data, focusPath]);

  const blocker = commitBlocker(status.data);

  return {
    selected,
    step,
    review,
    reviewVersion,
    reviewSelection,
    focusPath,
    focus: setFocusPath,
    message,
    changeMessage,
    messageProblem,
    confirmed,
    setConfirmed,
    committing,
    canSubmit,
    result,
    commitError,
    notice,
    blocker,
    toggle,
    setMany,
    clearSelection,
    startReview,
    backToSelection,
    submit,
    dismissResult,
    dismissCommitError,
  };
}
