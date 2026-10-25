import { MAX_COMMIT_MESSAGE_CHARS, subjectWarning } from "../../lib/commit";
import type { ReviewState } from "../../hooks/useCommitFlow";
import type { GitError } from "../../types/git";
import { Button } from "../common/Button";
import { ErrorBanner } from "../common/ErrorBanner";
import { Spinner } from "../common/Spinner";

interface CommitFormProps {
  review: ReviewState;
  message: string;
  messageProblem: string | null;
  confirmed: boolean;
  committing: boolean;
  canSubmit: boolean;
  notice: string | null;
  error: GitError | null;
  onMessage: (value: string) => void;
  onConfirm: (value: boolean) => void;
  onSubmit: () => void;
  onCancel: () => void;
  onDismissError: () => void;
}

/** Mensagem, confirmação explícita e botão de commit. Cancelar não altera o repositório. */
export function CommitForm(props: CommitFormProps) {
  const { review, message, messageProblem, confirmed, committing, canSubmit, notice, error } = props;
  const data = review.data;
  const warning = subjectWarning(message);
  const touched = message.length > 0;
  const busy = committing;

  return (
    <form
      className="border-t border-line bg-surface p-4"
      aria-label="Criar commit"
      onSubmit={(event) => {
        event.preventDefault();
        if (canSubmit) props.onSubmit();
      }}
    >
      {notice && (
        <p role="alert" className="mb-3 rounded-md border border-modified/40 bg-modified/10 px-3 py-2 text-sm text-modified">
          {notice}
        </p>
      )}
      {error && (
        <ErrorBanner
          className="mb-3"
          title="O commit não foi criado"
          error={error}
          onDismiss={props.onDismissError}
        />
      )}

      <p className="mb-2 text-sm">
        {data ? (
          <>
            Registrar <strong>{data.files.length}</strong> {data.files.length === 1 ? "arquivo" : "arquivos"} no
            histórico da branch <span className="font-mono text-accent">{data.branch}</span>
            <span className="text-muted"> (somente local; nada é enviado ao GitHub)</span>
          </>
        ) : (
          <span className="text-muted">Aguardando a revisão…</span>
        )}
      </p>

      <label htmlFor="commit-message" className="sr-only">
        Mensagem do commit
      </label>
      <textarea
        id="commit-message"
        value={message}
        onChange={(event) => props.onMessage(event.target.value)}
        disabled={busy}
        rows={3}
        maxLength={MAX_COMMIT_MESSAGE_CHARS + 500}
        placeholder="Mensagem do commit (a primeira linha resume a mudança)"
        aria-invalid={touched && messageProblem !== null}
        className="w-full resize-y rounded-md border border-line bg-canvas px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent disabled:opacity-60"
      />
      <div className="mt-1 flex justify-between text-xs">
        <span className={touched && messageProblem ? "text-removed" : "text-muted"}>
          {touched && messageProblem ? messageProblem : (warning ?? "")}
        </span>
        <span className="text-faint">{message.trim().length}/{MAX_COMMIT_MESSAGE_CHARS}</span>
      </div>

      <label className="mt-3 flex items-start gap-2 text-sm">
        <input
          type="checkbox"
          className="mt-0.5 h-3.5 w-3.5 accent-[var(--color-accent)]"
          checked={confirmed}
          disabled={!data || review.loading || busy}
          onChange={(event) => props.onConfirm(event.target.checked)}
        />
        <span>
          Revisei as diferenças acima e quero registrar estas alterações no histórico do Git.
        </span>
      </label>

      <div className="mt-3 flex items-center justify-end gap-2">
        {review.loading && (
          <span className="mr-auto flex items-center gap-2 text-xs text-muted">
            <Spinner /> Atualizando a revisão…
          </span>
        )}
        <Button onClick={props.onCancel} disabled={busy}>
          Cancelar
        </Button>
        <Button type="submit" variant="primary" disabled={!canSubmit}>
          {busy ? <Spinner /> : null}
          {busy ? "Criando commit…" : "Criar commit"}
        </Button>
      </div>
    </form>
  );
}
