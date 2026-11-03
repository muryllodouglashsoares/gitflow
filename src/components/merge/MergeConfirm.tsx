import { useEffect, useRef } from "react";
import { plural } from "../../lib/merge";
import type { MergePreview } from "../../types/git";
import { Button } from "../common/Button";
import { AlertIcon } from "../common/icons";

interface MergeConfirmProps {
  preview: MergePreview;
  acknowledged: boolean;
  typed: string;
  ready: boolean;
  onAcknowledge: (value: boolean) => void;
  onType: (value: string) => void;
  onCancel: () => void;
  onConfirm: () => void;
}

/** Confirmação explícita (inline) com origem, destino e o resultado esperado. */
export function MergeConfirm({
  preview,
  acknowledged,
  typed,
  ready,
  onAcknowledge,
  onType,
  onCancel,
  onConfirm,
}: MergeConfirmProps) {
  const ref = useRef<HTMLDivElement>(null);
  const src = preview.source.name;
  const dst = preview.target.name;

  useEffect(() => {
    ref.current?.focus();
  }, []);

  return (
    <div
      ref={ref}
      tabIndex={-1}
      role="group"
      aria-labelledby="merge-confirm-title"
      className="rounded-md border border-accent/40 bg-surface p-4 text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent"
    >
      <h3 id="merge-confirm-title" className="font-semibold">
        Confirmar merge
      </h3>

      <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
        <dt className="text-muted">Origem</dt>
        <dd className="font-mono">{src}</dd>
        <dt className="text-muted">Destino</dt>
        <dd className="font-mono">{dst}</dd>
        <dt className="text-muted">Commits a integrar</dt>
        <dd>{preview.sourceOnly}</dd>
        <dt className="text-muted">Resultado esperado</dt>
        <dd>
          {preview.strategy === "fastForward"
            ? "Avanço rápido: o destino apenas avança, sem commit de merge."
            : "Um commit de merge será criado no destino."}
        </dd>
      </dl>

      <ul className="mt-3 list-disc space-y-1 pl-5 text-muted">
        <li>
          Os commits de <span className="font-mono text-fg">{src}</span> serão integrados em{" "}
          <span className="font-mono text-fg">{dst}</span>. Só <span className="font-mono text-fg">{dst}</span> será modificada.
        </li>
        <li>O estado foi verificado e será conferido de novo no momento de executar; se algo mudar, nada é feito.</li>
        <li>Nenhuma alteração local é guardada, descartada ou forçada.</li>
        <li>Em caso de conflito, o merge fica em andamento e nada é resolvido automaticamente.</li>
        {preview.forecast.state === "conflicts" && (
          <li className="text-modified">
            A simulação prevê conflito em {plural(preview.forecast.files.length, "arquivo", "arquivos")}.
          </li>
        )}
      </ul>

      {preview.requiredConfirmation !== null && (
        <div className="mt-3 rounded-md border border-modified/40 bg-modified/10 p-3 text-modified">
          <p className="flex items-start gap-2">
            <AlertIcon className="mt-0.5 shrink-0" />
            <span>
              Esta operação modificará a branch principal <span className="font-mono">{preview.requiredConfirmation}</span>.
            </span>
          </p>
          <label htmlFor="merge-confirm-typed" className="mt-2 block text-fg">
            Para autorizar, digite <span className="font-mono">{preview.requiredConfirmation}</span>:
          </label>
          <input
            id="merge-confirm-typed"
            type="text"
            value={typed}
            onChange={(e) => onType(e.target.value)}
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            aria-describedby="merge-confirm-typed-help"
            className="mt-1.5 w-full rounded-md border border-line bg-canvas px-2.5 py-1.5 font-mono text-fg outline-none focus-visible:border-accent"
          />
          <p id="merge-confirm-typed-help" className="mt-1 text-xs text-muted">
            Diferencia maiúsculas de minúsculas. A conferência também é feita no backend.
          </p>
        </div>
      )}

      <label className="mt-3 flex items-start gap-2">
        <input
          type="checkbox"
          checked={acknowledged}
          onChange={(e) => onAcknowledge(e.target.checked)}
          className="mt-0.5"
        />
        <span>
          Entendo que os commits de <span className="font-mono">{src}</span> serão integrados em{" "}
          <span className="font-mono">{dst}</span>.
        </span>
      </label>

      <div className="mt-4 flex gap-2">
        <Button variant="primary" onClick={onConfirm} disabled={!ready}>
          Integrar {src} em {dst}
        </Button>
        <Button onClick={onCancel}>Cancelar</Button>
      </div>
    </div>
  );
}
