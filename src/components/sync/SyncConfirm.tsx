import { useEffect, useRef, useState } from "react";
import { isProtectedDestination, pullStrategyNote } from "../../lib/sync";
import type { SyncStatus, SyncTarget } from "../../types/git";
import { Button } from "../common/Button";

interface SyncConfirmProps {
  kind: "pull" | "push";
  sync: SyncStatus;
  target: SyncTarget;
  onCancel: () => void;
  onConfirm: (confirmProtectedBranch: boolean) => void;
}

/** Confirmação contextual (inline) de pull e push, sempre com o destino exato. */
export function SyncConfirm({ kind, sync, target, onCancel, onConfirm }: SyncConfirmProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [protectedOk, setProtectedOk] = useState(false);
  const protectedDestination = kind === "push" && isProtectedDestination(target.remoteBranch);
  const remoteRef = `${target.remote}/${target.remoteBranch}`;

  useEffect(() => {
    ref.current?.focus();
  }, []);

  return (
    <div
      ref={ref}
      tabIndex={-1}
      role="group"
      aria-labelledby="sync-confirm-title"
      className="rounded-md border border-accent/40 bg-surface p-4 text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent"
    >
      <h3 id="sync-confirm-title" className="font-semibold">
        {kind === "pull" ? "Confirmar pull (baixar e integrar)" : "Confirmar push (enviar commits)"}
      </h3>

      <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
        <dt className="text-muted">Branch local</dt>
        <dd className="font-mono">{target.branch}</dd>
        <dt className="text-muted">{kind === "pull" ? "Origem (upstream)" : "Destino (remoto)"}</dt>
        <dd className="font-mono">{remoteRef}</dd>
        {kind === "push" && sync.ahead !== null && (
          <>
            <dt className="text-muted">Commits a enviar</dt>
            <dd>{sync.ahead} (segundo a última atualização do remoto)</dd>
          </>
        )}
        {kind === "pull" && sync.behind !== null && (
          <>
            <dt className="text-muted">Commits a receber</dt>
            <dd>{sync.behind} (segundo a última atualização do remoto)</dd>
          </>
        )}
      </dl>

      {kind === "pull" ? (
        <ul className="mt-3 list-disc space-y-1 pl-5 text-muted">
          <li>Não há alterações locais em arquivos rastreados (verificado antes e de novo ao executar).</li>
          <li>{pullStrategyNote(sync)}</li>
          <li>O pull consulta o remoto antes de integrar; o resultado real será exibido. Conflitos não são resolvidos automaticamente.</li>
          <li>Nada é forçado e nenhum trabalho local é descartado.</li>
        </ul>
      ) : (
        <ul className="mt-3 list-disc space-y-1 pl-5 text-muted">
          <li>Só a branch atual é enviada, exatamente para o destino acima.</li>
          <li>Sem força (--force), sem criar ou alterar o upstream, sem enviar outras branches ou tags.</li>
          <li>Se o remoto recusar, nada será enviado e o motivo será exibido.</li>
        </ul>
      )}

      {protectedDestination && (
        <label className="mt-3 flex items-start gap-2 rounded-md border border-modified/40 bg-modified/10 p-2.5 text-modified">
          <input
            type="checkbox"
            checked={protectedOk}
            onChange={(e) => setProtectedOk(e.target.checked)}
            className="mt-0.5"
          />
          <span>
            Estou enviando diretamente para <span className="font-mono">{target.remoteBranch}</span>, uma branch
            principal. Confirmo que é isso mesmo que quero.
          </span>
        </label>
      )}

      <div className="mt-4 flex gap-2">
        <Button
          variant="primary"
          onClick={() => onConfirm(protectedOk)}
          disabled={protectedDestination && !protectedOk}
        >
          {kind === "pull" ? "Confirmar pull" : "Confirmar push"}
        </Button>
        <Button onClick={onCancel}>Cancelar</Button>
      </div>
    </div>
  );
}
