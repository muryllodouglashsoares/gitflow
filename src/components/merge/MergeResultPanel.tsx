import { operationLabel } from "../../lib/changes";
import { plural, resultSummary, shortOid } from "../../lib/merge";
import type { MergeResult } from "../../types/git";
import { Button } from "../common/Button";
import { AlertIcon, CheckCircleIcon } from "../common/icons";
import { FileList, OutputDetails, TONE_CARD, TONE_TEXT } from "./parts";

const OUTCOME_LABELS = {
  fastForward: "Avanço rápido",
  mergeCommit: "Commit de merge criado",
  conflicts: "Conflitos — merge não concluído",
} as const;

interface MergeResultPanelProps {
  result: MergeResult;
  onFinish: () => void;
  onOpenChanges: () => void;
}

/** Resultado verdadeiro da operação, lido do Git depois da execução. */
export function MergeResultPanel({ result, onFinish, onOpenChanges }: MergeResultPanelProps) {
  const summary = resultSummary(result);
  const conflicted = result.outcome === "conflicts";
  const st = result.status;
  const Icon = conflicted ? AlertIcon : CheckCircleIcon;

  return (
    <section
      aria-label="Resultado do merge"
      role={conflicted ? "alert" : "status"}
      className={`rounded-md border p-4 text-sm ${TONE_CARD[summary.tone]}`}
    >
      <div className="flex items-start gap-2.5">
        <Icon className={`mt-0.5 shrink-0 ${TONE_TEXT[summary.tone]}`} />
        <div className="min-w-0 flex-1">
          <p className="font-semibold">{summary.title}</p>
          <p className="mt-0.5 text-muted">{summary.description}</p>
        </div>
      </div>

      <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
        <dt className="text-muted">Origem</dt>
        <dd className="font-mono">{result.source}</dd>
        <dt className="text-muted">Destino</dt>
        <dd className="font-mono">{result.target}</dd>
        <dt className="text-muted">Resultado</dt>
        <dd>{OUTCOME_LABELS[result.outcome]}</dd>
        {!conflicted && (
          <>
            <dt className="text-muted">Commit resultante</dt>
            <dd className="font-mono" title={result.headAfter ?? undefined}>
              {shortOid(result.headBefore)} → {shortOid(result.headAfter)}
              {result.mergeCommit ? " (commit de merge)" : " (avanço rápido)"}
            </dd>
          </>
        )}
        <dt className="text-muted">Commits da origem</dt>
        <dd>{result.integratedCommits}</dd>
        <dt className="text-muted">Estado do repositório</dt>
        <dd>
          {st.operation ? `${operationLabel(st.operation)}` : st.isClean ? "Limpo" : "Com alterações locais"}
          {st.summary.conflicted > 0 && ` · ${plural(st.summary.conflicted, "arquivo em conflito", "arquivos em conflito")}`}
        </dd>
        <dt className="text-muted">Código de saída do Git</dt>
        <dd className="font-mono">{result.exitCode ?? "—"}</dd>
      </dl>

      {conflicted && (
        <div className="mt-4">
          <p className="font-medium">
            {plural(result.conflicts.length, "arquivo em conflito", "arquivos em conflito")}
          </p>
          <FileList
            files={result.conflicts.map((c) => c.path)}
            total={result.conflicts.length}
            label="Arquivos em conflito"
            trailing={(path) => {
              const code = result.conflicts.find((c) => c.path === path)?.code;
              return code ? <span className="text-modified">{code}</span> : null;
            }}
          />
        </div>
      )}

      <OutputDetails text={result.output} />

      <div className="mt-4 flex flex-wrap gap-2">
        {conflicted && <Button onClick={onOpenChanges}>Ver arquivos em conflito</Button>}
        <Button variant={conflicted ? "secondary" : "primary"} onClick={onFinish}>
          {conflicted ? "Fechar resultado" : "Concluir"}
        </Button>
      </div>
    </section>
  );
}
