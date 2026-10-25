import type { StatusState } from "../../hooks/useRepositoryWorkspace";
import { groupChanges, splitPath, type ChangeRow, type Selection } from "../../lib/changes";
import { commitEligibility, eligiblePaths, fileMarks } from "../../lib/commit";
import type { FileEntry } from "../../types/git";
import { Button } from "../common/Button";
import { CheckCircleIcon } from "../common/icons";
import { EmptyState } from "../common/EmptyState";
import { ErrorBanner } from "../common/ErrorBanner";
import { Spinner } from "../common/Spinner";
import { StatusBadge } from "../common/StatusBadge";

/** Seleção de arquivos para o commit (Fase 2). */
export interface CommitSelectionProps {
  selected: ReadonlySet<string>;
  onToggle: (path: string) => void;
  onSetMany: (paths: readonly string[], on: boolean) => void;
  /** Motivo pelo qual nenhum commit é possível agora, ou `null`. */
  blocker: string | null;
  /** Há uma operação de escrita em andamento. */
  locked: boolean;
  onReview: () => void;
}

interface ChangesPanelProps {
  status: StatusState;
  selection: Selection | null;
  onSelect: (selection: Selection) => void;
  onRefresh: () => void;
  commit: CommitSelectionProps;
}

export function ChangesPanel({ status, selection, onSelect, onRefresh, commit }: ChangesPanelProps) {
  const { data, loading, error } = status;
  const allEligible = data ? eligiblePaths(data.files) : [];
  const allSelected = allEligible.length > 0 && allEligible.every((p) => commit.selected.has(p));

  return (
    <section
      className="flex w-[21rem] shrink-0 flex-col border-r border-line bg-surface"
      aria-label="Arquivos alterados"
    >
      <div className="flex items-center justify-between border-b border-line px-4 py-2.5">
        <h2 className="text-xs font-semibold tracking-wide text-muted uppercase">Alterações</h2>
        <div className="flex items-center gap-2">
          {allEligible.length > 0 && (
            <button
              type="button"
              disabled={commit.locked}
              onClick={() => commit.onSetMany(allEligible, !allSelected)}
              className="rounded px-1.5 py-0.5 text-xs text-accent hover:bg-raised disabled:opacity-60"
            >
              {allSelected ? "Limpar seleção" : "Selecionar todos"}
            </button>
          )}
          {data && !data.isClean && (
            <span className="rounded-full bg-raised px-2 py-0.5 text-xs text-muted">{data.summary.total}</span>
          )}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto" aria-busy={loading}>
        {error ? (
          <div className="p-3">
            <ErrorBanner
              title="Não foi possível ler o repositório"
              error={error}
              actions={<Button onClick={onRefresh}>Tentar novamente</Button>}
            />
          </div>
        ) : !data ? (
          <EmptyState icon={<Spinner className="h-5 w-5" />} title="Carregando alterações…" />
        ) : data.isClean ? (
          <EmptyState
            icon={<CheckCircleIcon width={28} height={28} className="text-added" />}
            title="Nenhuma alteração"
            description="O repositório está limpo: não há arquivos modificados, adicionados, removidos ou não rastreados."
          />
        ) : (
          <GroupedList data={data.files} selection={selection} onSelect={onSelect} commit={commit} />
        )}
      </div>

      <CommitBar commit={commit} />
    </section>
  );
}

function CommitBar({ commit }: { commit: CommitSelectionProps }) {
  const count = commit.selected.size;
  const disabled = count === 0 || commit.blocker !== null || commit.locked;
  return (
    <div className="border-t border-line p-3">
      <p className="mb-2 text-xs text-muted" aria-live="polite">
        {count === 0
          ? "Marque os arquivos que entrarão no commit. Nada é incluído automaticamente."
          : `${count} ${count === 1 ? "arquivo selecionado" : "arquivos selecionados"} para o commit.`}
      </p>
      {commit.blocker && <p className="mb-2 text-xs text-conflict">{commit.blocker}</p>}
      <Button variant="primary" className="w-full" disabled={disabled} onClick={commit.onReview}>
        Revisar e commitar…
      </Button>
    </div>
  );
}

function GroupedList({
  data,
  selection,
  onSelect,
  commit,
}: {
  data: readonly FileEntry[];
  selection: Selection | null;
  onSelect: (selection: Selection) => void;
  commit: CommitSelectionProps;
}) {
  const groups = groupChanges(data);
  const byPath = new Map(data.map((f) => [f.path, f] as const));
  return (
    <div className="pb-3">
      {groups.map((group) => (
        <div key={group.id} className="pt-3">
          <h3 className="flex items-center justify-between px-4 pb-1 text-xs font-medium text-muted">
            <span>{group.title}</span>
            <span className="text-faint">{group.rows.length}</span>
          </h3>
          <ul>
            {group.rows.map((row) => (
              <FileRow
                key={row.key}
                row={row}
                selected={selection?.path === row.path && selection.target === row.target}
                onSelect={() => onSelect({ path: row.path, origPath: row.origPath, target: row.target })}
                file={byPath.get(row.path)}
                commit={commit}
              />
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

function FileRow({
  row,
  selected,
  onSelect,
  file,
  commit,
}: {
  row: ChangeRow;
  selected: boolean;
  onSelect: () => void;
  file: FileEntry | undefined;
  commit: CommitSelectionProps;
}) {
  const { dir, name } = splitPath(row.path);
  const eligibility = file ? commitEligibility(file) : { eligible: false, reason: null };
  const marks = file ? fileMarks(file) : null;
  const checked = commit.selected.has(row.path);
  return (
    <li
      className={`flex items-center ${
        selected ? "bg-accent/10 shadow-[inset_2px_0_0_var(--color-accent)]" : "hover:bg-raised/70"
      }`}
    >
      <label
        className="flex shrink-0 items-center py-1.5 pr-1 pl-4"
        title={eligibility.eligible ? "Incluir no commit" : (eligibility.reason ?? "Indisponível")}
      >
        <input
          type="checkbox"
          className="h-3.5 w-3.5 accent-[var(--color-accent)]"
          checked={checked}
          disabled={!eligibility.eligible || commit.locked}
          onChange={() => commit.onToggle(row.path)}
          aria-label={`Incluir ${row.path} no commit`}
        />
      </label>
      <button
        type="button"
        onClick={onSelect}
        aria-current={selected ? "true" : undefined}
        title={row.path}
        className="flex min-w-0 flex-1 items-center gap-2.5 py-1.5 pr-4 pl-2 text-left text-sm transition-colors"
      >
        <StatusBadge badge={row.badge} />
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0">
            <span className="truncate text-muted">{dir}</span>
            <span className="max-w-full shrink-0 truncate">{name}</span>
          </span>
          {row.origPath && <span className="block truncate text-xs text-faint">de {row.origPath}</span>}
          {!eligibility.eligible && eligibility.reason && (
            <span className="block truncate text-xs text-conflict">{eligibility.reason}</span>
          )}
          {marks?.partial && (
            <span className="block truncate text-xs text-faint">
              parte preparada e parte não preparada
            </span>
          )}
        </span>
      </button>
    </li>
  );
}
