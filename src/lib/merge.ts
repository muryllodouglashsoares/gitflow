/**
 * Lógica pura do merge manual (Fase 5): estágio do fluxo, textos da prévia e do resultado e as
 * condições da confirmação.
 *
 * Nada aqui decide se um merge é permitido: o backend (`merge.rs`) é a autoridade e revalida tudo
 * ao executar. Estas funções só traduzem o que o backend informou para a interface.
 */

import type {
  BlockerKind,
  GitError,
  MergePreview,
  MergeResult,
} from "../types/git";

export type MergePhase = "idle" | "confirming" | "running" | "done";

export type MergeStage =
  | "selecting"
  | "validating"
  | "ready"
  | "blocked"
  | "nothing"
  | "confirming"
  | "running"
  | "completed"
  | "conflicts"
  | "failed";

export const STAGE_LABELS: Record<MergeStage, string> = {
  selecting: "Aguardando seleção",
  validating: "Validando",
  ready: "Pronto para integrar",
  blocked: "Operação bloqueada",
  nothing: "Nada a integrar",
  confirming: "Aguardando confirmação",
  running: "Executando merge",
  completed: "Merge concluído",
  conflicts: "Conflitos encontrados",
  failed: "Falha na operação",
};

export type MergeTone = "ok" | "info" | "warning" | "danger" | "neutral";

export const STAGE_TONES: Record<MergeStage, MergeTone> = {
  selecting: "neutral",
  validating: "neutral",
  ready: "info",
  blocked: "warning",
  nothing: "neutral",
  confirming: "info",
  running: "info",
  completed: "ok",
  conflicts: "warning",
  failed: "danger",
};

export interface StageInput {
  source: string | null;
  target: string | null;
  /** Prévia que corresponde à seleção atual (ou `null`). */
  preview: MergePreview | null;
  previewLoading: boolean;
  previewError: GitError | null;
  phase: MergePhase;
  result: MergeResult | null;
  runError: GitError | null;
}

export function deriveStage(i: StageInput): MergeStage {
  if (i.phase === "running") return "running";
  if (i.phase === "done" && i.result) return i.result.outcome === "conflicts" ? "conflicts" : "completed";
  if (i.runError) return "failed";
  if (!i.source || !i.target || i.source === i.target) return "selecting";
  if (i.previewError) return "blocked";
  if (!i.preview) return "validating";
  if (i.phase === "confirming") return "confirming";
  if (i.preview.canExecute) return "ready";
  if (
    i.preview.blockers.length === 0 &&
    (i.preview.relation === "identical" || i.preview.relation === "alreadyIntegrated")
  ) {
    return "nothing";
  }
  return "blocked";
}

/** Problema com a seleção em si (antes de consultar o Git). */
export function selectionProblem(source: string | null, target: string | null): string | null {
  if (source && target && source === target) {
    return "A origem e o destino são a mesma branch. Escolha duas branches diferentes.";
  }
  return null;
}

export const shortOid = (oid: string | null | undefined) => (oid ? oid.slice(0, 7) : "—");

export const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export interface Summary {
  tone: MergeTone;
  title: string;
  description: string;
}

export function forecastText(p: MergePreview): string | null {
  switch (p.forecast.state) {
    case "clean":
      return "Simulação do Git: nenhum conflito previsto.";
    case "conflicts":
      return `Simulação do Git: conflito previsto em ${plural(p.forecast.files.length, "arquivo", "arquivos")}.`;
    case "unavailable":
      return "Não foi possível simular a integração (requer Git 2.38 ou mais recente). Conflitos só serão conhecidos ao executar.";
    case "notApplicable":
      return null;
  }
}

/** Resumo da relação entre origem e destino, em linguagem direta. */
export function relationSummary(p: MergePreview): Summary {
  const src = p.source.name;
  const dst = p.target.name;
  switch (p.relation) {
    case "identical":
      return {
        tone: "neutral",
        title: "Branches idênticas",
        description: `${src} e ${dst} apontam para o mesmo commit. Não há nada a integrar.`,
      };
    case "alreadyIntegrated":
      return {
        tone: "neutral",
        title: "Já integrada",
        description:
          `Tudo o que ${src} tem já está em ${dst}. Não há nada a integrar` +
          (p.targetOnly > 0 ? ` (${dst} tem ${plural(p.targetOnly, "commit", "commits")} a mais).` : "."),
      };
    case "fastForward":
      return {
        tone: "info",
        title: "Avanço rápido",
        description:
          `${dst} só avançará até o último commit de ${src} (${plural(p.sourceOnly, "commit", "commits")}). ` +
          "Nenhum commit de merge será criado.",
      };
    case "diverged":
      return {
        tone: p.forecast.state === "conflicts" ? "warning" : "info",
        title: "Será criado um commit de merge",
        description:
          `Os dois lados têm commits próprios: ${plural(p.sourceOnly, "commit", "commits")} em ${src} e ` +
          `${plural(p.targetOnly, "commit", "commits")} em ${dst}. O Git criará um commit de merge em ${dst}.`,
      };
    case "unrelated":
      return {
        tone: "danger",
        title: "Sem histórico em comum",
        description: "As branches não compartilham nenhum commit. O aplicativo não integra históricos independentes.",
      };
  }
}

export const BLOCKER_TITLES: Record<BlockerKind, string> = {
  unbornHead: "Repositório sem commits",
  detachedHead: "HEAD desanexado",
  operationInProgress: "Operação em andamento",
  conflicts: "Conflitos pendentes",
  stagedChanges: "Alterações preparadas (staged)",
  unstagedChanges: "Alterações não preparadas",
  untrackedOverwrite: "Arquivos não rastreados seriam sobrescritos",
  unrelatedHistories: "Históricos independentes",
  targetNotCurrent: "O destino não é a branch atual",
};

/**
 * A interface oferece "Trocar para o destino" quando esse é o único impedimento (ou o HEAD
 * desanexado, que a troca também resolve). A troca em si continua sendo a ação explícita e
 * segura da Fase 2, que o backend valida de novo.
 */
export function canSwitchToTarget(p: MergePreview): boolean {
  const kinds = p.blockers.map((b) => b.kind);
  return (
    kinds.includes("targetNotCurrent") &&
    kinds.every((k) => k === "targetNotCurrent" || k === "detachedHead")
  );
}

/** A confirmação está completa na interface? (O backend exige as mesmas condições.) */
export function confirmationReady(p: MergePreview | null, acknowledged: boolean, typed: string): boolean {
  if (!p || !p.canExecute || !acknowledged) return false;
  return p.requiredConfirmation === null || typed === p.requiredConfirmation;
}

export function resultSummary(r: MergeResult): Summary {
  const into = `${r.source} → ${r.target}`;
  switch (r.outcome) {
    case "fastForward":
      return {
        tone: "ok",
        title: `Merge concluído (avanço rápido): ${into}`,
        description:
          `${r.target} avançou para ${shortOid(r.headAfter)} e agora contém ` +
          `${plural(r.integratedCommits, "commit", "commits")} de ${r.source}. Nenhum commit de merge foi criado.`,
      };
    case "mergeCommit":
      return {
        tone: "ok",
        title: `Merge concluído (commit de merge): ${into}`,
        description:
          `Foi criado o commit de merge ${shortOid(r.headAfter)} em ${r.target}, integrando ` +
          `${plural(r.integratedCommits, "commit", "commits")} de ${r.source}.`,
      };
    case "conflicts":
      return {
        tone: "warning",
        title: `Merge NÃO concluído: conflitos em ${plural(r.conflicts.length, "arquivo", "arquivos")}`,
        description:
          `O Git parou a integração ${into}. ${r.target} continua em ${shortOid(r.headBefore)} e o merge ` +
          "está em andamento. Nada foi resolvido nem descartado automaticamente: resolva os conflitos " +
          "manualmente e conclua com “git commit”, ou cancele com “git merge --abort”.",
      };
  }
}
