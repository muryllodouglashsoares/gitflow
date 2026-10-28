import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { selectionExists, type Selection } from "../lib/changes";
import {
  INITIAL_IMPORT_FLOW,
  authorizedEntries,
  canEditSelection,
  canInspect,
  confirmProblems,
  foreignChanges,
  importFlowReducer,
  isRunning,
  overwrittenEntries,
  planPush,
  restrictStatus,
  suggestWorkBranch,
  type ImportStage,
} from "../lib/importFlow";
import { pickZipFile } from "../services/folderPicker";
import { errorFrom, gitService } from "../services/gitService";
import type { BranchList, CommitRequest, GitError, SyncStatus } from "../types/git";
import { useCommitFlow } from "./useCommitFlow";
import { useFileDiff } from "./useFileDiff";
import type { RunExclusive, StatusState } from "./useRepositoryWorkspace";

interface Params {
  repoPath: string | null;
  status: StatusState;
  /** Branches locais (para validar o nome da branch de trabalho). */
  branchList: BranchList | null;
  /** Diagnóstico de sincronização do repositório (para o destino do push). */
  syncStatus: SyncStatus | null;
  runExclusive: RunExclusive;
  refresh: () => void;
}

const COMMIT_STAGES: readonly ImportStage[] = ["commitReview", "committing"];

/**
 * Importação de ZIP (Fase 4): conecta a interface aos comandos do backend, etapa por etapa.
 *
 * O estado do fluxo vive em `importFlowReducer` (puro e testado). Este hook só:
 * - chama o backend na ordem do fluxo e despacha o resultado REAL de cada etapa;
 * - mantém cada etapa de escrita dentro de `runExclusive` (uma operação de escrita por vez);
 * - descarta respostas de outro repositório ou de tentativas antigas (número de sequência);
 * - reaproveita o fluxo de commit, o diff e o status existentes, restritos aos arquivos
 *   gravados pela importação;
 * - ao abrir um repositório, procura uma operação pendente gravada no disco para retomar.
 */
export function useZipImport({ repoPath, status, branchList, syncStatus, runExclusive, refresh }: Params) {
  const [flow, dispatch] = useReducer(importFlowReducer, INITIAL_IMPORT_FLOW);
  const flowRef = useRef(flow);
  const repoRef = useRef(repoPath);
  const seqRef = useRef(0);
  const busyRef = useRef(false);

  const [overwriteAck, setOverwriteAck] = useState(false);
  const [planAck, setPlanAck] = useState(false);
  const [pushAck, setPushAck] = useState(false);
  const [protectedAck, setProtectedAck] = useState(false);
  const [picking, setPicking] = useState(false);
  const [pickError, setPickError] = useState<GitError | null>(null);
  const [recoverError, setRecoverError] = useState<GitError | null>(null);
  const [selection, setSelection] = useState<Selection | null>(null);

  useEffect(() => {
    flowRef.current = flow;
  }, [flow]);

  // Trocar de repositório zera o fluxo; depois procura uma operação pendente para retomar.
  useEffect(() => {
    repoRef.current = repoPath;
    busyRef.current = false;
    dispatch({ type: "reset", repoPath });
    setOverwriteAck(false);
    setPlanAck(false);
    setPushAck(false);
    setProtectedAck(false);
    setPickError(null);
    setRecoverError(null);
    setSelection(null);
    if (!repoPath) return;
    let cancelled = false;
    gitService
      .findImportOperation(repoPath)
      .then((operation) => {
        if (cancelled || repoRef.current !== repoPath || !operation) return;
        dispatch({ type: "resume", repoPath, operation, at: Date.now() });
      })
      .catch(() => {
        // Sem registro legível não há o que retomar: o fluxo começa normalmente.
      });
    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  // Cada plano novo exige confirmações novas.
  const planId = flow.plan?.planId ?? null;
  useEffect(() => {
    setOverwriteAck(false);
    setPlanAck(false);
  }, [planId]);

  // As confirmações do push valem só enquanto o usuário está na etapa de push.
  useEffect(() => {
    if (flow.stage !== "committed") {
      setPushAck(false);
      setProtectedAck(false);
    }
  }, [flow.stage]);

  // ------------------------------------------------------------------ seleção

  const currentBranch = status.data?.branch.name ?? null;

  const chooseZip = useCallback(async () => {
    if (!repoPath || picking) return;
    setPicking(true);
    setPickError(null);
    try {
      const path = await pickZipFile();
      if (!path || repoRef.current !== repoPath) return;
      dispatch({
        type: "chooseZip",
        repoPath,
        zipPath: path,
        baseBranch: flowRef.current.baseBranch ?? currentBranch,
        workBranch: suggestWorkBranch(path, new Date()),
      });
    } catch (error) {
      if (repoRef.current === repoPath) setPickError(errorFrom(error));
    } finally {
      setPicking(false);
    }
  }, [repoPath, picking, currentBranch]);

  const runInspect = useCallback(
    async (zipPath: string, sourcePrefix: string, baseBranch: string) => {
      if (!repoPath || busyRef.current) return;
      busyRef.current = true;
      const seq = ++seqRef.current;
      dispatch({ type: "inspectStart", repoPath, seq, at: Date.now() });
      try {
        const plan = await gitService.inspectZip({ repoPath, zipPath, sourcePrefix, baseBranch });
        dispatch({ type: "inspectOk", repoPath, seq, plan, at: Date.now() });
      } catch (error) {
        dispatch({ type: "inspectFail", repoPath, seq, error: errorFrom(error), at: Date.now() });
      } finally {
        busyRef.current = false;
      }
    },
    [repoPath],
  );

  const inspect = useCallback(() => {
    const f = flowRef.current;
    if (!canInspect(f) || !f.zipPath || !f.baseBranch) return;
    void runInspect(f.zipPath, f.sourcePrefix, f.baseBranch);
  }, [runInspect]);

  // Mudar a base ou a pasta do pacote invalida o plano; como a inspeção é somente leitura, ela
  // é refeita na hora para a tela nunca mostrar um plano que não corresponde às escolhas.
  const changePrefix = useCallback(
    (prefix: string) => {
      const f = flowRef.current;
      if (!repoPath || !f.zipPath || !f.baseBranch || busyRef.current || !canEditSelection(f.stage)) return;
      dispatch({ type: "setPrefix", repoPath, prefix });
      void runInspect(f.zipPath, prefix, f.baseBranch);
    },
    [repoPath, runInspect],
  );

  const changeBase = useCallback(
    (baseBranch: string) => {
      const f = flowRef.current;
      if (!repoPath || busyRef.current || !canEditSelection(f.stage)) return;
      dispatch({ type: "setBase", repoPath, baseBranch });
      if (f.zipPath) void runInspect(f.zipPath, f.sourcePrefix, baseBranch);
    },
    [repoPath, runInspect],
  );

  const changeWorkBranch = useCallback(
    (name: string) => {
      if (repoPath) dispatch({ type: "setWorkBranch", repoPath, name });
    },
    [repoPath],
  );

  const toggleExclude = useCallback(
    (path: string) => {
      if (repoPath) dispatch({ type: "toggleExclude", repoPath, path });
    },
    [repoPath],
  );

  const cancel = useCallback(() => {
    if (!repoPath) return;
    dispatch({ type: "cancel", repoPath, at: Date.now() });
    busyRef.current = false; // a resposta da inspeção será descartada pelo reducer
  }, [repoPath]);

  // ------------------------------------------------------------------ confirmação

  const existingBranches = useMemo(() => branchList?.branches.map((b) => b.name) ?? [], [branchList]);

  const problems = useMemo(() => {
    const list = confirmProblems(flow, { currentBranch, existingBranches, overwriteAck, planAck });
    return branchList ? list : ["Carregando as branches do repositório…", ...list];
  }, [flow, currentBranch, existingBranches, overwriteAck, planAck, branchList]);

  const overwrites = useMemo(() => overwrittenEntries(flow.plan, flow.excluded), [flow.plan, flow.excluded]);
  const writes = useMemo(() => authorizedEntries(flow.plan, flow.excluded), [flow.plan, flow.excluded]);

  /**
   * Cria a branch de trabalho e grava os arquivos do plano confirmado. São dois comandos do
   * backend, cada um revalidando o plano; o resultado de cada um é registrado separadamente.
   */
  const startImport = useCallback(async () => {
    const f = flowRef.current;
    if (!repoPath || busyRef.current || problems.length > 0) return;
    if (f.stage !== "planReview" || !f.plan || !f.zipPath || !f.baseBranch) return;
    busyRef.current = true;
    const plan = f.plan;
    const common = {
      repoPath,
      zipPath: f.zipPath,
      sourcePrefix: f.sourcePrefix,
      baseBranch: f.baseBranch,
      workBranch: f.workBranch,
      planId: plan.planId,
    };
    const seq = ++seqRef.current;
    dispatch({ type: "branchStart", repoPath, seq, at: Date.now() });
    try {
      const prepared = await runExclusive("Criando a branch de trabalho", () =>
        gitService.prepareImportBranch(common),
      );
      if (repoRef.current !== repoPath) return;
      if (!prepared.ok) {
        dispatch({ type: "branchFail", repoPath, seq, error: prepared.error, at: Date.now() });
        refresh();
        return;
      }
      dispatch({ type: "branchOk", repoPath, seq, baseOid: prepared.value.baseOid, at: Date.now() });
      refresh();

      const applied = await runExclusive("Importando arquivos do ZIP", () =>
        gitService.applyImport({
          ...common,
          excludedPaths: [...f.excluded],
          confirmOverwrites: overwriteAck,
        }),
      );
      if (repoRef.current !== repoPath) return;
      if (!applied.ok) {
        dispatch({ type: "applyFail", repoPath, seq, error: applied.error, at: Date.now() });
      } else {
        dispatch({ type: "applyOk", repoPath, seq, result: applied.value, at: Date.now() });
      }
      refresh();
    } finally {
      busyRef.current = false;
    }
  }, [repoPath, problems, runExclusive, refresh, overwriteAck]);

  // ------------------------------------------------------------------ desfazer

  const rollback = useCallback(async () => {
    const f = flowRef.current;
    if (!repoPath || busyRef.current || !f.opId) return;
    if (f.stage !== "partial" && f.stage !== "commitReview") return;
    busyRef.current = true;
    const seq = ++seqRef.current;
    const opId = f.opId;
    dispatch({ type: "rollbackStart", repoPath, seq, at: Date.now() });
    try {
      const out = await runExclusive("Desfazendo a importação", () =>
        gitService.rollbackImport({ repoPath, opId }),
      );
      if (repoRef.current !== repoPath) return;
      if (!out.ok) {
        dispatch({ type: "rollbackFail", repoPath, seq, error: out.error, at: Date.now() });
      } else {
        dispatch({ type: "rollbackOk", repoPath, seq, result: out.value, at: Date.now() });
        // Limpa temporários e backups só se nada precisa deles (arquivos editados pelo usuário
        // ou problemas ao restaurar mantêm o backup para recuperação manual).
        if (out.value.keptModified.length === 0 && out.value.problems.length === 0) {
          void gitService.finishImport({ repoPath, opId }).catch(() => undefined);
        }
      }
      refresh();
    } finally {
      busyRef.current = false;
    }
  }, [repoPath, runExclusive, refresh]);

  // ------------------------------------------------------------------ commit

  const createImportCommit = useCallback(
    (request: CommitRequest) => {
      const opId = flowRef.current.opId;
      if (!opId) {
        return Promise.reject(new Error("A operação de importação não foi identificada."));
      }
      return gitService.createImportCommit({ opId, ...request });
    },
    [],
  );

  const importedSet = useMemo(() => new Set(flow.importedPaths), [flow.importedPaths]);
  const engagedForCommit = COMMIT_STAGES.includes(flow.stage);

  // Só os arquivos gravados pela importação podem ser selecionados para o commit.
  const restrictedStatus: StatusState = useMemo(
    () =>
      status.data && importedSet.size > 0
        ? { ...status, data: restrictStatus(status.data, importedSet) }
        : status,
    [status, importedSet],
  );

  const commit = useCommitFlow({
    repoPath: engagedForCommit ? repoPath : null,
    status: restrictedStatus,
    runExclusive,
    refresh,
    createCommit: createImportCommit,
  });

  // O resultado é declarado ANTES de "committing": no mesmo ciclo, o sucesso tem prioridade.
  useEffect(() => {
    if (commit.result && repoPath) {
      dispatch({ type: "commitOk", repoPath, result: commit.result, at: Date.now() });
    }
  }, [commit.result, repoPath]);

  useEffect(() => {
    if (!repoPath) return;
    if (commit.committing) dispatch({ type: "commitStart", repoPath, at: Date.now() });
    else dispatch({ type: "commitAborted", repoPath, at: Date.now() });
  }, [commit.committing, repoPath]);

  // Saiu da etapa de commit (ex.: desfez a importação): a revisão aberta deixa de valer.
  const { step: commitStep, backToSelection } = commit;
  useEffect(() => {
    if (!engagedForCommit && commitStep === "review") backToSelection();
  }, [engagedForCommit, commitStep, backToSelection]);

  const reviewing = commit.step === "review";
  const effectiveSelection =
    selection && restrictedStatus.data && selectionExists(restrictedStatus.data.files, selection)
      ? selection
      : null;
  const { diff, retry: retryDiff } = useFileDiff(
    engagedForCommit ? repoPath : null,
    reviewing ? commit.reviewSelection : effectiveSelection,
    reviewing ? commit.reviewVersion : status.updatedAt,
  );

  const foreign = useMemo(() => foreignChanges(status.data, importedSet), [status.data, importedSet]);

  // ------------------------------------------------------------------ push

  const pushStage = flow.stage === "committed" || flow.stage === "pushing";
  const pushPlan = useMemo(
    () =>
      pushStage
        ? planPush(syncStatus, status.data, flow.workBranch, flow.commitOid)
        : { target: null, problems: [], protectedDestination: false },
    [pushStage, syncStatus, status.data, flow.workBranch, flow.commitOid],
  );

  const canPush =
    flow.stage === "committed" &&
    pushPlan.target !== null &&
    pushPlan.problems.length === 0 &&
    pushAck &&
    (!pushPlan.protectedDestination || protectedAck);

  const push = useCallback(async () => {
    const f = flowRef.current;
    if (!repoPath || busyRef.current || !canPush || !pushPlan.target || !f.opId) return;
    busyRef.current = true;
    const seq = ++seqRef.current;
    const opId = f.opId;
    const target = pushPlan.target;
    const confirmProtectedBranch = pushPlan.protectedDestination && protectedAck;
    dispatch({ type: "pushStart", repoPath, seq, at: Date.now() });
    try {
      const out = await runExclusive("Enviando ao remoto", () =>
        gitService.pushImportBranch({ opId, repoPath, target, confirmProtectedBranch }),
      );
      if (repoRef.current !== repoPath) return;
      if (!out.ok) dispatch({ type: "pushFail", repoPath, seq, error: out.error, at: Date.now() });
      else dispatch({ type: "pushOk", repoPath, seq, result: out.value, at: Date.now() });
      refresh();
    } finally {
      busyRef.current = false;
    }
  }, [repoPath, canPush, pushPlan, protectedAck, runExclusive, refresh]);

  const finishWithoutPush = useCallback(async () => {
    const f = flowRef.current;
    if (!repoPath || busyRef.current || f.stage !== "committed" || !f.opId) return;
    busyRef.current = true;
    const opId = f.opId;
    const seq = f.seq;
    try {
      const out = await runExclusive("Encerrando a importação", () =>
        gitService.finishImport({ repoPath, opId }),
      );
      if (repoRef.current !== repoPath) return;
      if (!out.ok) dispatch({ type: "finishFail", repoPath, seq, error: out.error, at: Date.now() });
      else dispatch({ type: "finishOk", repoPath, seq, at: Date.now() });
    } finally {
      busyRef.current = false;
    }
  }, [repoPath, runExclusive]);

  // ------------------------------------------------------------------ recuperação

  /**
   * Recomeça depois de uma falha. Se a falha ocorreu depois de a branch de trabalho ficar ativa,
   * volta primeiro para a branch-base (troca segura: o Git recusa se houver alterações em risco).
   */
  const recover = useCallback(
    async (switchToBase: boolean) => {
      const f = flowRef.current;
      if (!repoPath || busyRef.current || f.stage !== "failed") return;
      setRecoverError(null);
      if (switchToBase && f.baseBranch) {
        busyRef.current = true;
        try {
          const base = f.baseBranch;
          const out = await runExclusive("Voltando para a branch-base", () =>
            gitService.switchBranch({ repoPath, name: base }),
          );
          if (repoRef.current !== repoPath) return;
          if (!out.ok) {
            setRecoverError(out.error);
            return;
          }
          refresh();
        } finally {
          busyRef.current = false;
        }
      }
      dispatch({ type: "recover", repoPath, at: Date.now() });
    },
    [repoPath, runExclusive, refresh],
  );

  const restart = useCallback(() => {
    if (!repoPath) return;
    dispatch({ type: "restart", repoPath });
    commit.dismissResult();
    setOverwriteAck(false);
    setPlanAck(false);
    setPickError(null);
    setRecoverError(null);
    refresh();
  }, [repoPath, commit, refresh]);

  const dismissIssue = useCallback(() => dispatch({ type: "dismissIssue" }), []);

  return {
    flow,
    running: isRunning(flow.stage),
    // seleção e inspeção
    picking,
    pickError,
    chooseZip,
    inspect,
    cancel,
    changeBase,
    changePrefix,
    changeWorkBranch,
    toggleExclude,
    // confirmação
    problems,
    overwrites,
    writes,
    overwriteAck,
    setOverwriteAck,
    planAck,
    setPlanAck,
    startImport,
    // desfazer / recuperar
    rollback,
    recover,
    recoverError,
    restart,
    dismissIssue,
    // commit isolado
    commit,
    restrictedStatus,
    selection: effectiveSelection,
    select: setSelection,
    diff,
    retryDiff,
    foreign,
    // push
    pushPlan,
    pushAck,
    setPushAck,
    protectedAck,
    setProtectedAck,
    canPush,
    push,
    finishWithoutPush,
    currentBranch,
  };
}

export type ZipImportController = ReturnType<typeof useZipImport>;
