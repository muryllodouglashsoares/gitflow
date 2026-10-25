import type { ReviewState } from "../../hooks/useCommitFlow";
import { splitPath } from "../../lib/changes";
import type { ReviewFile } from "../../types/git";
import { Button } from "../common/Button";
import { ErrorBanner } from "../common/ErrorBanner";
import { EmptyState } from "../common/EmptyState";
import { Spinner } from "../common/Spinner";

interface ReviewFileListProps {
  review: ReviewState;
  focusPath: string | null;
  onFocus: (path: string) => void;
  onBack: () => void;
  disabled: boolean;
}

function fileNote(file: ReviewFile): string | null {
  if (file.untracked) return "arquivo novo: será adicionado ao Git";
  if (file.hasStaged && file.hasUnstaged) return "parte preparada e parte não: o commit usa o conteúdo atual";
  if (file.origPath) return `renomeado de ${file.origPath}`;
  return null;
}

/** Lista (somente leitura) dos arquivos que entrarão no commit. Substitui a lista de alterações. */
export function ReviewFileList({ review, focusPath, onFocus, onBack, disabled }: ReviewFileListProps) {
  const data = review.data;
  return (
    <section
      className="flex w-[21rem] shrink-0 flex-col border-r border-line bg-surface"
      aria-label="Arquivos do commit"
    >
      <div className="flex items-center justify-between border-b border-line px-4 py-2.5">
        <h2 className="text-xs font-semibold tracking-wide text-muted uppercase">Revisão do commit</h2>
        <Button onClick={onBack} disabled={disabled}>
          Voltar
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto" aria-busy={review.loading}>
        {review.error ? (
          <div className="p-3">
            <ErrorBanner title="Não foi possível preparar a revisão" error={review.error} />
          </div>
        ) : !data ? (
          <EmptyState icon={<Spinner className="h-5 w-5" />} title="Preparando a revisão…" />
        ) : (
          <>
            <p className="px-4 pt-3 pb-1 text-xs font-medium text-muted">
              Entrarão no commit ({data.files.length})
              {review.loading && <Spinner className="ml-2 inline h-3 w-3" />}
            </p>
            <ul>
              {data.files.map((file) => {
                const { dir, name } = splitPath(file.path);
                const note = fileNote(file);
                const active = file.path === focusPath;
                return (
                  <li key={file.path}>
                    <button
                      type="button"
                      onClick={() => onFocus(file.path)}
                      aria-current={active ? "true" : undefined}
                      title={file.path}
                      className={`block w-full px-4 py-1.5 text-left text-sm transition-colors ${
                        active ? "bg-accent/10 shadow-[inset_2px_0_0_var(--color-accent)]" : "hover:bg-raised/70"
                      }`}
                    >
                      <span className="flex min-w-0">
                        <span className="truncate text-muted">{dir}</span>
                        <span className="max-w-full shrink-0 truncate">{name}</span>
                      </span>
                      {note && <span className="block truncate text-xs text-faint">{note}</span>}
                    </button>
                  </li>
                );
              })}
            </ul>
            {data.preservedStaged.length > 0 && (
              <div className="mx-4 mt-3 mb-4 rounded-md border border-line bg-raised/50 p-2.5 text-xs text-muted">
                <p className="font-medium text-fg">
                  Fora do commit, mantidos como estão ({data.preservedStaged.length})
                </p>
                <p className="mt-1">
                  Estes arquivos já estavam preparados (staged) e não foram selecionados. Eles continuam
                  preparados depois do commit:
                </p>
                <ul className="mt-1 max-h-28 overflow-auto font-mono break-all">
                  {data.preservedStaged.map((path) => (
                    <li key={path}>{path}</li>
                  ))}
                </ul>
              </div>
            )}
          </>
        )}
      </div>
    </section>
  );
}
