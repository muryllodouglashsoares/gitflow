import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { confirmationReady, deriveStage, type MergePhase } from "../lib/merge";
import { errorFrom, gitService } from "../services/gitService";
import type { BranchList, GitError, MergePreview, MergeResult } from "../types/git";
import type { RunExclusive } from "./useRepositoryWorkspace";

interface PreviewState {
  /** Seleção a que estes dados pertencem. */
  key: string | null;
  data: MergePreview | null;
  loading: boolean;
  error: GitError | null;
}

const IDLE_PREVIEW: PreviewState = { key: null, data: null, loading: false, error: null };

const selectionKey = (repoPath: string | null, source: string | null, target: string | null) =>
  repoPath && source && target ? `${repoPath}\u0000${source}\u0000${target}` : null;

interface UseMergeArgs {
  repoPath: string | null;
  /** A aba de merge está aberta: só então a prévia (somente leitura) é consultada. */
  enabled: boolean;
  /** Muda a cada leitura do status; a prévia é refeita junto. */
  refreshToken: number | null;
  branchList: BranchList | null;
  runExclusive: RunExclusive;
  /** Relê o status do repositório (depois de qualquer operação de escrita). */
  onChanged: () => void;
  /** Relê a lista de branches. */
  reloadBranches: () => void;
  /** Troca de branch explícita (Fase 2). */
  switchTo: (name: string) => Promise<boolean>;
  dismissSwitch: () => void;
}

/**
 * Fluxo do merge manual: seleção → prévia (somente leitura) → confirmação → execução → resultado.
 *
 * - O merge NUNCA é executado automaticamente: só `execute()`, depois da confirmação, chama o
 *   backend, e este revalida tudo e exige o `token` da prévia revisada.
 * - Se a prévia mudar durante a confirmação (outro `token`), a confirmação perde a validade.
 * - Depois de executar, nada é presumido: o resultado e o estado vêm do Git.
 */
export function useMerge({
  repoPath,
  enabled,
  refreshToken,
  branchList,
  runExclusive,
  onChanged,
  reloadBranches,
  switchTo,
  dismissSwitch,
}: UseMergeArgs) {
  const [source, setSourceState] = useState<string | null>(null);
  const [target, setTargetState] = useState<string | null>(null);
  const [phase, setPhase] = useState<MergePhase>("idle");
  const [acknowledged, setAcknowledged] = useState(false);
  const [typed, setTyped] = useState("");
  const [result, setResult] = useState<MergeResult | null>(null);
  const [runError, setRunError] = useState<GitError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [preview, setPreview] = useState<PreviewState>(IDLE_PREVIEW);
  const [recheckCount, setRecheckCount] = useState(0);

  const repoRef = useRef(repoPath);
  const reviewedToken = useRef<string | null>(null);
  const seqRef = useRef(0);

  const resetConfirmation = useCallback(() => {
    reviewedToken.current = null;
    setAcknowledged(false);
    setTyped("");
  }, []);

  // Trocar de repositório zera tudo o que pertence ao anterior.
  useEffect(() => {
    repoRef.current = repoPath;
    setSourceState(null);
    setTargetState(null);
    setPhase("idle");
    setResult(null);
    setRunError(null);
    setNotice(null);
    setPreview(IDLE_PREVIEW);
    resetConfirmation();
  }, [repoPath, resetConfirmation]);

  // Destino inicial: a branch atual (único destino que o Git aceita integrar sem trocar de branch).
  const currentBranch = branchList?.current ?? null;
  useEffect(() => {
    if (target === null && currentBranch && branchList?.branches.some((b) => b.name === currentBranch)) {
      setTargetState(currentBranch);
    }
  }, [target, currentBranch, branchList]);

  // Uma branch escolhida que deixou de existir é removida da seleção.
  useEffect(() => {
    if (!branchList) return;
    const exists = (name: string | null) => name === null || branchList.branches.some((b) => b.name === name);
    if (!exists(source)) setSourceState(null);
    if (!exists(target)) setTargetState(null);
  }, [branchList, source, target]);

  const frozen = phase === "running" || phase === "done";
  const key = selectionKey(repoPath, source, target);

  // Prévia: somente leitura, refeita quando a seleção ou o estado do repositório mudam.
  useEffect(() => {
    if (!enabled) return;
    if (!repoPath || !source || !target || source === target || frozen) {
      setPreview(IDLE_PREVIEW);
      return;
    }
    const seq = ++seqRef.current;
    const thisKey = selectionKey(repoPath, source, target);
    setPreview((prev) => ({
      key: thisKey,
      data: prev.key === thisKey ? prev.data : null,
      loading: true,
      error: null,
    }));

    gitService
      .previewMerge({ repoPath, source, target })
      .then((data) => {
        if (seq !== seqRef.current) return;
        if (data.repoPath !== repoPath) {
          setPreview({
            key: thisKey,
            data: null,
            loading: false,
            error: {
              code: "FRONTEND",
              message: "A resposta recebida não corresponde ao repositório selecionado. Verifique novamente.",
            },
          });
          return;
        }
        setPreview({ key: thisKey, data, loading: false, error: null });
      })
      .catch((error: unknown) => {
        if (seq !== seqRef.current) return;
        setPreview({ key: thisKey, data: null, loading: false, error: errorFrom(error) });
      });
  }, [enabled, repoPath, source, target, refreshToken, recheckCount, frozen]);

  const current: PreviewState = preview.key === key && key !== null ? preview : IDLE_PREVIEW;
  const previewData = current.data;

  // Se o que foi revisado mudou enquanto o usuário confirmava, a confirmação deixa de valer.
  useEffect(() => {
    if (phase !== "confirming" || !previewData) return;
    if (previewData.token !== reviewedToken.current || !previewData.canExecute) {
      setPhase("idle");
      resetConfirmation();
      setNotice("O repositório ou as branches mudaram durante a confirmação. Revise a integração de novo.");
    }
  }, [phase, previewData, resetConfirmation]);

  const clearFeedback = useCallback(() => {
    setRunError(null);
    setNotice(null);
  }, []);

  const setSource = useCallback(
    (name: string | null) => {
      setSourceState(name);
      setPhase("idle");
      resetConfirmation();
      clearFeedback();
    },
    [resetConfirmation, clearFeedback],
  );

  const setTarget = useCallback(
    (name: string | null) => {
      setTargetState(name);
      setPhase("idle");
      resetConfirmation();
      clearFeedback();
    },
    [resetConfirmation, clearFeedback],
  );

  /** Inverte origem e destino (a direção continua explícita na tela). */
  const swap = useCallback(() => {
    setSourceState(target);
    setTargetState(source);
    setPhase("idle");
    resetConfirmation();
    clearFeedback();
  }, [source, target, resetConfirmation, clearFeedback]);

  /** Relê branches e estado e refaz a prévia. Não altera nada no repositório. */
  const recheck = useCallback(() => {
    clearFeedback();
    reloadBranches();
    onChanged();
    setRecheckCount((n) => n + 1);
  }, [clearFeedback, reloadBranches, onChanged]);

  /** Abre a confirmação (nenhuma alteração acontece aqui). */
  const prepare = useCallback(() => {
    if (!previewData?.canExecute || phase !== "idle") return;
    clearFeedback();
    resetConfirmation();
    reviewedToken.current = previewData.token;
    setPhase("confirming");
  }, [previewData, phase, clearFeedback, resetConfirmation]);

  const cancelConfirmation = useCallback(() => {
    setPhase("idle");
    resetConfirmation();
  }, [resetConfirmation]);

  const ready = confirmationReady(previewData, acknowledged, typed);

  /** Executa o merge revisado. O backend revalida tudo e recusa se algo mudou. */
  const execute = useCallback(async () => {
    if (!repoPath || phase !== "confirming" || !previewData || !ready) return;
    if (previewData.token !== reviewedToken.current) return;
    const p = previewData;
    setPhase("running");
    setRunError(null);
    setNotice(null);

    const res = await runExclusive("merge", () =>
      gitService.mergeBranches({
        repoPath,
        source: p.source.name,
        target: p.target.name,
        token: p.token,
        confirmed: true,
        confirmTarget: p.requiredConfirmation !== null ? typed : null,
      }),
    );
    if (repoRef.current !== repoPath) return;

    // Qualquer que seja o desfecho, o estado real é lido de novo.
    onChanged();
    reloadBranches();
    resetConfirmation();
    if (res.ok) {
      setResult(res.value);
      setPhase("done");
    } else {
      setRunError(res.error);
      setPhase("idle");
      setRecheckCount((n) => n + 1);
    }
  }, [repoPath, phase, previewData, ready, typed, runExclusive, onChanged, reloadBranches, resetConfirmation]);

  /** Troca explícita para a branch de destino (ação da Fase 2; o backend valida de novo). */
  const switchToTarget = useCallback(async () => {
    if (!target) return;
    clearFeedback();
    const ok = await switchTo(target);
    if (ok) dismissSwitch();
    setRecheckCount((n) => n + 1);
  }, [target, switchTo, dismissSwitch, clearFeedback]);

  /** Encerra o resultado e volta à seleção (o destino é mantido). */
  const finish = useCallback(() => {
    setResult(null);
    setPhase("idle");
    setSourceState(null);
    resetConfirmation();
    clearFeedback();
    setRecheckCount((n) => n + 1);
  }, [resetConfirmation, clearFeedback]);

  const dismissRunError = useCallback(() => setRunError(null), []);
  const dismissNotice = useCallback(() => setNotice(null), []);

  const stage = useMemo(
    () =>
      deriveStage({
        source,
        target,
        preview: previewData,
        previewLoading: current.loading,
        previewError: current.error,
        phase,
        result,
        runError,
      }),
    [source, target, previewData, current.loading, current.error, phase, result, runError],
  );

  return {
    source,
    target,
    setSource,
    setTarget,
    swap,
    preview: current,
    stage,
    phase,
    acknowledged,
    setAcknowledged,
    typed,
    setTyped,
    ready,
    result,
    runError,
    notice,
    recheck,
    prepare,
    cancelConfirmation,
    execute,
    switchToTarget,
    finish,
    dismissRunError,
    dismissNotice,
  };
}

export type MergeController = ReturnType<typeof useMerge>;
