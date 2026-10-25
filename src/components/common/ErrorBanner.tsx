import type { ReactNode } from "react";
import type { GitError } from "../../types/git";
import { AlertIcon, CloseIcon } from "./icons";

interface ErrorBannerProps {
  title?: string;
  error: GitError;
  /** Ações adicionais (ex.: "Tentar novamente"). */
  actions?: ReactNode;
  onDismiss?: () => void;
  className?: string;
}

export function ErrorBanner({ title, error, actions, onDismiss, className = "" }: ErrorBannerProps) {
  return (
    <div
      role="alert"
      className={`rounded-md border border-removed/40 bg-removed/10 p-3 text-sm ${className}`}
    >
      <div className="flex items-start gap-2.5">
        <AlertIcon className="mt-0.5 shrink-0 text-removed" />
        <div className="min-w-0 flex-1">
          {title && <p className="font-medium text-fg">{title}</p>}
          <p className={title ? "mt-0.5 text-muted" : "text-fg"}>{error.message}</p>
          {error.details && (
            <details className="mt-2 text-xs text-muted">
              <summary className="cursor-pointer select-none hover:text-fg">Detalhes técnicos</summary>
              <pre className="mt-1.5 max-h-40 overflow-auto rounded bg-canvas/60 p-2 font-mono whitespace-pre-wrap break-words">
                {error.details}
              </pre>
            </details>
          )}
          {actions && <div className="mt-2.5 flex flex-wrap gap-2">{actions}</div>}
        </div>
        {onDismiss && (
          <button
            type="button"
            onClick={onDismiss}
            aria-label="Fechar mensagem de erro"
            className="rounded p-1 text-muted hover:bg-raised hover:text-fg"
          >
            <CloseIcon />
          </button>
        )}
      </div>
    </div>
  );
}
