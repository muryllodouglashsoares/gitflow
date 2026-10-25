import type { CommitResult } from "../../types/git";
import { CheckCircleIcon, CloseIcon } from "../common/icons";

const STATUS_LABELS: Record<string, string> = {
  A: "adicionado",
  M: "modificado",
  D: "removido",
  R: "renomeado",
  C: "copiado",
  T: "tipo alterado",
};

/** Resultado REAL do Git depois de um commit bem-sucedido. */
export function CommitResultBanner({ result, onDismiss }: { result: CommitResult; onDismiss: () => void }) {
  return (
    <div role="status" className="m-3 mb-0 rounded-md border border-added/40 bg-added/10 p-3 text-sm">
      <div className="flex items-start gap-2.5">
        <CheckCircleIcon className="mt-0.5 shrink-0 text-added" />
        <div className="min-w-0 flex-1">
          <p className="font-medium">
            Commit <span className="font-mono">{result.shortOid}</span> criado na branch{" "}
            <span className="font-mono">{result.branch}</span>
          </p>
          <p className="mt-0.5 truncate text-muted" title={result.subject}>
            {result.subject}
          </p>
          <p className="mt-0.5 font-mono text-xs break-all text-faint">{result.oid}</p>
          {result.summary && <p className="mt-1 text-xs text-muted">{result.summary}</p>}
          {result.files.length > 0 && (
            <details className="mt-1.5 text-xs text-muted">
              <summary className="cursor-pointer select-none hover:text-fg">
                Arquivos registrados ({result.files.length})
              </summary>
              <ul className="mt-1 max-h-32 overflow-auto font-mono break-all">
                {result.files.map((file) => (
                  <li key={`${file.status}:${file.path}`}>
                    {STATUS_LABELS[file.status] ?? file.status}: {file.origPath ? `${file.origPath} → ` : ""}
                    {file.path}
                  </li>
                ))}
              </ul>
            </details>
          )}
          <p className="mt-1.5 text-xs text-muted">Commit local: nada foi enviado ao GitHub.</p>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Fechar mensagem"
          className="rounded p-1 text-muted hover:bg-raised hover:text-fg"
        >
          <CloseIcon />
        </button>
      </div>
    </div>
  );
}
