import {
  BLOCKER_TITLES,
  canSwitchToTarget,
  forecastText,
  plural,
  relationSummary,
  shortOid,
} from "../../lib/merge";
import type { MergePreview } from "../../types/git";
import { Button } from "../common/Button";
import { AlertIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";
import { FileList, TONE_CARD, TONE_TEXT } from "./parts";

interface MergePreviewPanelProps {
  preview: MergePreview;
  refreshing: boolean;
  /** Há alguma operação de escrita em andamento (qualquer tipo). */
  locked: boolean;
  switching: boolean;
  onSwitchToTarget: () => void;
}

/** Diagnóstico completo da integração: relação das branches, previsão de conflitos e bloqueios. */
export function MergePreviewPanel({ preview, refreshing, locked, switching, onSwitchToTarget }: MergePreviewPanelProps) {
  const summary = relationSummary(preview);
  const forecast = forecastText(preview);
  const s = preview.summary;
  const dirtyParts = [
    s.staged > 0 ? `${plural(s.staged, "preparado", "preparados")}` : null,
    s.unstaged > 0 ? `${plural(s.unstaged, "modificado", "modificados")}` : null,
    s.untracked > 0 ? `${plural(s.untracked, "não rastreado", "não rastreados")}` : null,
    s.conflicted > 0 ? `${plural(s.conflicted, "em conflito", "em conflito")}` : null,
  ].filter(Boolean);

  const hiddenCommits = preview.sourceOnly - preview.sourceCommits.length;

  return (
    <section aria-label="Resumo da integração" className="space-y-3">
      <div className={`rounded-md border p-4 ${TONE_CARD[summary.tone]}`}>
        <div className="flex items-center gap-2">
          <p className={`text-sm font-semibold ${TONE_TEXT[summary.tone]}`}>{summary.title}</p>
          {refreshing && <Spinner className="text-muted" />}
        </div>
        <p className="mt-1 text-sm">{summary.description}</p>
        {forecast && (
          <p className={`mt-2 text-sm ${preview.forecast.state === "conflicts" ? "text-modified" : "text-muted"}`}>
            {forecast}
          </p>
        )}
        {preview.forecast.state === "conflicts" && (
          <FileList
            files={preview.forecast.files}
            total={preview.forecast.files.length}
            label="Arquivos com conflito previsto"
          />
        )}
      </div>

      <div className="rounded-md border border-line bg-surface p-4">
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
          <dt className="text-muted">Branch atual</dt>
          <dd className="font-mono">{preview.currentBranch ?? "(HEAD desanexado)"}</dd>
          <dt className="text-muted">Commits só na origem</dt>
          <dd>
            {preview.sourceOnly}
            <span className="text-muted"> (serão integrados)</span>
          </dd>
          <dt className="text-muted">Commits só no destino</dt>
          <dd>{preview.targetOnly}</dd>
          {preview.mergeBase && (
            <>
              <dt className="text-muted">Base comum</dt>
              <dd className="font-mono">{shortOid(preview.mergeBase)}</dd>
            </>
          )}
          <dt className="text-muted">Estado do repositório</dt>
          <dd>
            {dirtyParts.length === 0 ? "Limpo" : dirtyParts.join(" · ")}
            {preview.operation && <span className="text-modified"> · operação em andamento</span>}
          </dd>
        </dl>

        {preview.sourceCommits.length > 0 && (
          <div className="mt-3">
            <p className="text-xs font-medium text-muted">Commits que serão integrados (mais recentes primeiro)</p>
            <ul aria-label="Commits da origem" className="mt-1.5 space-y-1 text-sm">
              {preview.sourceCommits.map((c) => (
                <li key={c.oid} className="flex gap-2">
                  <span className="shrink-0 font-mono text-xs text-muted">{shortOid(c.oid)}</span>
                  <span className="min-w-0 truncate" title={c.subject}>
                    {c.subject || "(sem mensagem)"}
                  </span>
                </li>
              ))}
              {hiddenCommits > 0 && <li className="text-xs text-muted">… e mais {hiddenCommits}</li>}
            </ul>
          </div>
        )}

        {preview.notes.length > 0 && (
          <ul className="mt-3 list-disc space-y-1 pl-5 text-xs text-muted">
            {preview.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        )}
      </div>

      {preview.blockers.length > 0 && (
        <div role="alert" className="rounded-md border border-modified/40 bg-modified/10 p-4">
          <p className="flex items-center gap-2 text-sm font-semibold text-modified">
            <AlertIcon className="shrink-0" />
            Não é possível integrar agora
          </p>
          <ul className="mt-2 space-y-3 text-sm">
            {preview.blockers.map((b) => (
              <li key={b.kind}>
                <p className="font-medium">{BLOCKER_TITLES[b.kind]}</p>
                <p className="mt-0.5 text-muted">{b.message}</p>
                {b.files.length > 0 && <FileList files={b.files} total={b.filesTotal} label={BLOCKER_TITLES[b.kind]} />}
              </li>
            ))}
          </ul>
          {canSwitchToTarget(preview) && (
            <div className="mt-3 flex flex-wrap items-center gap-3 border-t border-modified/30 pt-3">
              <Button onClick={onSwitchToTarget} disabled={locked || switching}>
                {switching && <Spinner />}
                Trocar para <span className="font-mono">{preview.target.name}</span>
              </Button>
              <p className="text-xs text-muted">
                Troca explícita de branch (nunca forçada). Nenhum merge será feito agora; depois da troca a
                integração é revisada de novo.
              </p>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
