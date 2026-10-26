/**
 * Lógica pura da sincronização remota: rótulos do diagnóstico, atualidade das referências remotas
 * e motivos pelos quais fetch/pull/push ficam bloqueados na interface.
 *
 * Os bloqueios daqui espelham as validações do backend (`remote.rs`) para a interface explicar o
 * motivo antes do clique. O backend continua sendo a autoridade: ele revalida tudo ao executar.
 */

import type { GitError, RepositoryStatus, SyncStatus, SyncTarget } from "../types/git";
import { operationLabel } from "./changes";

/** Depois deste tempo sem fetch, as contagens deixam de ser apresentadas como atuais. */
export const STALE_AFTER_MS = 30 * 60 * 1000;

/** Destinos para os quais o envio exige uma confirmação adicional. */
export const PROTECTED_BRANCHES: readonly string[] = ["main", "master"];

export type SyncTone = "ok" | "info" | "warning" | "danger" | "neutral";

export interface SyncSummary {
  tone: SyncTone;
  label: string;
  description: string;
}

export interface ActionBlock {
  reason: string;
  /** Arquivos que causam o bloqueio (quando aplicável). */
  files?: string[];
}

export interface DescribeContext {
  /** Último fetch/pull que falhou por rede, credenciais ou tempo (`null` se não houve ou se já deu certo depois). */
  fetchFailure: GitError | null;
  /** Momento (ms) do último fetch/pull concluído com sucesso nesta sessão. */
  sessionFetchAt: number | null;
  now: number;
}

export function isProtectedDestination(remoteBranch: string): boolean {
  return PROTECTED_BRANCHES.includes(remoteBranch);
}

/** Momento (ms) da última atualização conhecida das referências remotas. */
export function lastFetchMs(sync: SyncStatus, sessionFetchAt: number | null): number | null {
  const fromGit = sync.lastFetchAt !== null ? sync.lastFetchAt * 1000 : null;
  if (fromGit === null) return sessionFetchAt;
  if (sessionFetchAt === null) return fromGit;
  return Math.max(fromGit, sessionFetchAt);
}

export function isStale(sync: SyncStatus, sessionFetchAt: number | null, now: number): boolean {
  const last = lastFetchMs(sync, sessionFetchAt);
  return last === null || now - last > STALE_AFTER_MS;
}

export function formatAge(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return "agora há pouco";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `há ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `há ${hours} h`;
  const days = Math.floor(hours / 24);
  return `há ${days} ${days === 1 ? "dia" : "dias"}`;
}

/** "2 a enviar · 1 a receber" (ou `null` quando as contagens não são conhecidas). */
export function countsText(sync: SyncStatus): string | null {
  if (sync.ahead === null || sync.behind === null) return null;
  return `${sync.ahead} a enviar · ${sync.behind} a receber`;
}

const WITH_COUNTS = new Set(["synced", "ahead", "behind", "diverged", "upstreamNotFetched"]);

/** Resumo do estado. Nunca afirma "sincronizada" sem referências remotas recentes e verificáveis. */
export function describeSync(sync: SyncStatus, ctx: DescribeContext): SyncSummary {
  const branch = sync.branch ?? "a branch atual";
  const upstream = sync.upstream?.display ?? "o upstream";

  switch (sync.state) {
    case "noRemote":
      return {
        tone: "neutral",
        label: "Sem remoto",
        description:
          "Este repositório não tem remoto configurado. Adicione um fora do aplicativo (por exemplo, “git remote add origin <url>”) para poder sincronizar.",
      };
    case "detachedHead":
      return {
        tone: "warning",
        label: "HEAD desanexado",
        description: "Não há branch atual para sincronizar. Troque para uma branch.",
      };
    case "unborn":
      return {
        tone: "neutral",
        label: "Sem commits",
        description: "A branch atual ainda não tem commits para sincronizar.",
      };
    case "noUpstream":
      return {
        tone: "warning",
        label: "Sem upstream",
        description: `A branch ${branch} não acompanha nenhuma branch remota, e o aplicativo não define upstream sozinho. Defina-o fora do aplicativo (por exemplo, “git branch --set-upstream-to=<remoto>/<branch>”). Você ainda pode buscar (fetch) as referências do remoto.`,
      };
    case "upstreamInvalid":
      return {
        tone: "danger",
        label: "Upstream inválido",
        description: `O upstream configurado para ${branch} aponta para um remoto inexistente ou para uma branch local. Corrija a configuração fora do aplicativo.`,
      };
    default:
      break;
  }

  // A partir daqui existe upstream válido. Falha recente de rede/credenciais: não verificável.
  if (ctx.fetchFailure) {
    return {
      tone: "danger",
      label: "Remoto não verificável",
      description: `A última tentativa de falar com o remoto falhou (${ctx.fetchFailure.message}). As contagens abaixo vêm das últimas referências locais e podem estar desatualizadas.`,
    };
  }

  const stale = WITH_COUNTS.has(sync.state) && isStale(sync, ctx.sessionFetchAt, ctx.now);
  const last = lastFetchMs(sync, ctx.sessionFetchAt);
  const staleNote =
    last === null
      ? " Nenhuma atualização do remoto foi registrada: faça um fetch para confirmar."
      : ` Última atualização ${formatAge(ctx.now - last)}: faça um fetch para confirmar.`;

  switch (sync.state) {
    case "upstreamNotFetched":
      return {
        tone: "warning",
        label: "Remoto ainda não verificado",
        description: `${upstream} ainda não existe localmente (nunca foi buscada ou foi removida no remoto). Execute um fetch para conhecer o estado do remoto.`,
      };
    case "synced":
      return stale
        ? {
            tone: "warning",
            label: "Possivelmente desatualizada",
            description: `Pelas últimas referências conhecidas, ${branch} está igual a ${upstream}, mas elas podem estar velhas.${staleNote}`,
          }
        : {
            tone: "ok",
            label: "Sincronizada",
            description: `${branch} está igual a ${upstream} segundo a última atualização do remoto.`,
          };
    case "ahead":
      return {
        tone: "info",
        label: "Adiantada",
        description: `${branch} tem ${sync.ahead ?? 0} commit(s) que ${upstream} ainda não tem. Use “Enviar (push)”.${stale ? staleNote : ""}`,
      };
    case "behind":
      return {
        tone: "info",
        label: "Atrasada",
        description: `${upstream} tem ${sync.behind ?? 0} commit(s) que ${branch} ainda não tem. Use “Baixar (pull)” para integrá-los.${stale ? staleNote : ""}`,
      };
    case "diverged":
      return {
        tone: "warning",
        label: "Divergente",
        description: `${branch} e ${upstream} têm commits que o outro lado não tem (${sync.ahead ?? 0} a enviar, ${sync.behind ?? 0} a receber). É preciso integrar as alterações antes de enviar.${stale ? staleNote : ""}`,
      };
    default:
      return { tone: "neutral", label: "Desconhecido", description: "Estado de sincronização desconhecido." };
  }
}

/** Remoto sugerido: o do upstream, senão `origin`, senão o primeiro. */
export function defaultRemote(sync: SyncStatus | null): string | null {
  if (!sync || sync.remotes.length === 0) return null;
  const fromUpstream = sync.upstream?.remote;
  if (fromUpstream && sync.remotes.some((r) => r.name === fromUpstream)) return fromUpstream;
  return sync.remotes.find((r) => r.name === "origin")?.name ?? sync.remotes[0]?.name ?? null;
}

/** Destino exato de pull/push (branch local, remoto e branch remota), se for inequívoco. */
export function syncTarget(sync: SyncStatus | null): SyncTarget | null {
  if (!sync || !sync.branch || !sync.upstream) return null;
  if (sync.state === "upstreamInvalid") return null;
  return {
    branch: sync.branch,
    remote: sync.upstream.remote,
    remoteBranch: sync.upstream.remoteBranch,
  };
}

const MAX_FILES = 12;

function unusableUpstream(sync: SyncStatus, action: string): ActionBlock | null {
  switch (sync.state) {
    case "noRemote":
      return { reason: `Sem remoto configurado: não há para onde ${action}.` };
    case "detachedHead":
      return { reason: `HEAD desanexado: troque para uma branch antes de ${action}.` };
    case "unborn":
      return { reason: `A branch ainda não tem commits: crie um commit antes de ${action}.` };
    case "noUpstream":
      return {
        reason: `A branch atual não tem upstream (o aplicativo não o define sozinho). Defina-o fora do aplicativo antes de ${action}.`,
      };
    case "upstreamInvalid":
      return { reason: `O upstream configurado é inválido. Corrija-o fora do aplicativo antes de ${action}.` };
    default:
      return null;
  }
}

function operationBlock(status: RepositoryStatus, action: string): ActionBlock | null {
  if (status.operation) {
    return { reason: `${operationLabel(status.operation)}. Conclua ou cancele antes de ${action}.` };
  }
  if (status.summary.conflicted > 0) {
    return { reason: `Há arquivos em conflito. Resolva-os antes de ${action}.` };
  }
  return null;
}

/** O fetch só precisa de um remoto válido (não altera nada local). */
export function fetchBlock(sync: SyncStatus | null, remote: string | null): ActionBlock | null {
  if (!sync) return { reason: "Aguarde o diagnóstico de sincronização." };
  if (sync.remotes.length === 0) return { reason: "Sem remoto configurado: não há o que buscar." };
  if (!remote || !sync.remotes.some((r) => r.name === remote)) {
    return { reason: "Escolha um dos remotos configurados." };
  }
  return null;
}

export function pullBlock(sync: SyncStatus | null, status: RepositoryStatus | null): ActionBlock | null {
  if (!sync || !status) return { reason: "Aguarde o diagnóstico de sincronização." };
  const action = "baixar alterações";
  const unusable = unusableUpstream(sync, action);
  if (unusable) return unusable;
  const op = operationBlock(status, action);
  if (op) return op;

  const atRisk = status.files.filter((f) => f.index !== null || f.worktree !== null).map((f) => f.path);
  if (atRisk.length > 0) {
    return {
      reason: `Há ${atRisk.length} arquivo(s) com alterações não commitadas. Faça o commit ou guarde-as manualmente antes do pull; o aplicativo não descarta trabalho local.`,
      files: atRisk.slice(0, MAX_FILES),
    };
  }
  if (sync.state === "diverged") {
    if (sync.pullStrategy === "unspecified") {
      return {
        reason:
          "A branch divergiu e nenhuma estratégia de pull está configurada. O aplicativo não escolhe entre merge e rebase: defina “pull.rebase” fora do aplicativo ou integre manualmente.",
      };
    }
    if (sync.pullStrategy === "fastForwardOnly") {
      return {
        reason:
          "A branch divergiu e o Git está configurado só para avanço rápido (pull.ff = only), o que é impossível aqui. Integre manualmente.",
      };
    }
  }
  return null;
}

export function pushBlock(sync: SyncStatus | null, status: RepositoryStatus | null): ActionBlock | null {
  if (!sync || !status) return { reason: "Aguarde o diagnóstico de sincronização." };
  const action = "enviar commits";
  const unusable = unusableUpstream(sync, action);
  if (unusable) return unusable;
  const op = operationBlock(status, action);
  if (op) return op;

  switch (sync.state) {
    case "upstreamNotFetched":
      return { reason: "A branch remota ainda não existe localmente. Faça um fetch antes de enviar." };
    case "behind":
      return { reason: "O remoto tem commits que você não tem e não há nada local a enviar. Baixe (pull) primeiro." };
    case "diverged":
      return { reason: "A branch divergiu do remoto, que rejeitaria o envio. Integre as alterações (pull) antes." };
    case "synced":
      return { reason: "Não há commits locais para enviar (segundo a última atualização do remoto)." };
    default:
      return null;
  }
}

/** Como o pull será feito, para constar na confirmação. */
export function pullStrategyNote(sync: SyncStatus): string {
  switch (sync.pullStrategy) {
    case "merge":
      return "Estratégia configurada no seu Git: merge. Se houver commits dos dois lados, será criado um commit de merge e podem surgir conflitos.";
    case "rebase":
      return "Estratégia configurada no seu Git: rebase. Se houver commits dos dois lados, seus commits locais serão reaplicados e podem surgir conflitos.";
    case "fastForwardOnly":
      return "Estratégia configurada no seu Git: somente avanço rápido (sem merge nem rebase).";
    case "unspecified":
      return "Nenhuma estratégia de pull está configurada: o pull só será feito se for um avanço rápido, sem criar merge nem rebase.";
  }
}
