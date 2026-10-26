import type { SyncOpResult, SyncOpState } from "../../lib/syncOperation";
import { stepMessage } from "../../lib/syncOperation";
import { ErrorBanner } from "../common/ErrorBanner";
import { AlertIcon, CheckCircleIcon, CloseIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";

const shortOid = (oid: string | null) => (oid ? oid.slice(0, 7) : "—");

function Output({ text }: { text: string }) {
  if (!text.trim()) return null;
  return (
    <details className="mt-2 text-xs text-muted">
      <summary className="cursor-pointer select-none hover:text-fg">Saída do Git</summary>
      <pre className="mt-1.5 max-h-40 overflow-auto rounded bg-canvas/60 p-2 font-mono whitespace-pre-wrap break-words">
        {text}
      </pre>
    </details>
  );
}

function successText(outcome: SyncOpResult): { title: string; body: string; output: string } {
  switch (outcome.kind) {
    case "fetch": {
      const { result } = outcome;
      return {
        title:
          result.updatedRefs === 0
            ? `Nenhuma novidade em ${result.remote}.`
            : `${result.updatedRefs} referência(s) de ${result.remote} atualizada(s).`,
        body: "O fetch só atualiza as referências remotas: nada foi integrado à sua branch nem aos seus arquivos. Use “Baixar (pull)” para integrar.",
        output: result.output,
      };
    }
    case "pull": {
      const { result } = outcome;
      const from = `${result.target.remote}/${result.target.remoteBranch}`;
      if (result.outcome === "alreadyUpToDate") {
        return {
          title: `${result.target.branch} já estava atualizada com ${from}.`,
          body: "Nenhuma alteração foi integrada.",
          output: result.output,
        };
      }
      return {
        title: `Alterações de ${from} integradas em ${result.target.branch}.`,
        body: `HEAD: ${shortOid(result.headBefore)} → ${shortOid(result.headAfter)}${
          result.mergeCommit ? ". Foi criado um commit de merge (ainda não enviado)." : "."
        }`,
        output: result.output,
      };
    }
    case "push": {
      const { result } = outcome;
      return {
        title: `${result.pushedCommits} commit(s) enviado(s) para ${result.target.remote}/${result.target.remoteBranch}.`,
        body: `O Git concluiu o envio de ${result.target.branch} (${shortOid(result.localOid)}). O estado abaixo foi lido de novo.`,
        output: result.output,
      };
    }
  }
}

interface OperationFeedbackProps {
  op: SyncOpState;
  onDismiss: () => void;
}

/** Resultado real da última operação remota: em execução, sucesso, falha ou "não confirmado". */
export function OperationFeedback({ op, onDismiss }: OperationFeedbackProps) {
  if (op.phase === "idle") return null;

  if (op.phase === "running" && op.kind) {
    return (
      <div role="status" aria-live="polite" className="flex items-start gap-2.5 rounded-md border border-line bg-surface p-3 text-sm">
        <Spinner className="mt-0.5 shrink-0 text-accent" />
        <div>
          <p className="font-medium">{stepMessage(op.kind)}</p>
          <p className="mt-0.5 text-muted">
            Aguardando o Git. Operações de rede podem demorar; o progresso percentual não está disponível.
          </p>
        </div>
      </div>
    );
  }

  if (op.phase === "success" && op.result) {
    const text = successText(op.result);
    return (
      <div role="status" aria-live="polite" className="flex items-start gap-2.5 rounded-md border border-added/40 bg-added/10 p-3 text-sm">
        <CheckCircleIcon className="mt-0.5 shrink-0 text-added" />
        <div className="min-w-0 flex-1">
          <p className="font-medium">{text.title}</p>
          <p className="mt-0.5 text-muted">{text.body}</p>
          <Output text={text.output} />
        </div>
        <button type="button" onClick={onDismiss} aria-label="Fechar mensagem" className="rounded p-1 text-muted hover:bg-raised hover:text-fg">
          <CloseIcon />
        </button>
      </div>
    );
  }

  if (op.phase === "unconfirmed" && op.error) {
    return (
      <div role="alert" className="flex items-start gap-2.5 rounded-md border border-modified/40 bg-modified/10 p-3 text-sm">
        <AlertIcon className="mt-0.5 shrink-0 text-modified" />
        <div className="min-w-0 flex-1">
          <p className="font-medium text-modified">Resultado não confirmado</p>
          <p className="mt-0.5">{op.error.message}</p>
        </div>
        <button type="button" onClick={onDismiss} aria-label="Fechar mensagem" className="rounded p-1 text-muted hover:bg-raised hover:text-fg">
          <CloseIcon />
        </button>
      </div>
    );
  }

  if (op.phase === "failure" && op.error) {
    const title =
      op.kind === "fetch"
        ? "A atualização das referências remotas falhou"
        : op.kind === "pull"
          ? "O pull não foi concluído"
          : "O envio não foi concluído";
    return <ErrorBanner title={title} error={op.error} onDismiss={onDismiss} />;
  }

  return null;
}
