import type { StatusState } from "../../hooks/useRepositoryWorkspace";
import { groupChanges, splitPath, type ChangeRow, type Selection } from "../../lib/changes";
import { Button } from "../common/Button";
import { CheckCircleIcon } from "../common/icons";
import { EmptyState } from "../common/EmptyState";
import { ErrorBanner } from "../common/ErrorBanner";
import { Spinner } from "../common/Spinner";
import { StatusBadge } from "../common/StatusBadge";

interface ChangesPanelProps {
  status: StatusState;
  selection: Selection | null;
  onSelect: (selection: Selection) => void;
  onRefresh: () => void;
}

export function ChangesPanel({ status, selection, onSelect, onRefresh }: ChangesPanelProps) {
  const { data, loading, error } = status;

  return (
    <section
      className="flex w-[21rem] shrink-0 flex-col border-r border-line bg-surface"
      aria-label="Arquivos alterados"
    >
      <div className="flex items-center justify-between border-b border-line px-4 py-2.5">
        <h2 className="text-xs font-semibold tracking-wide text-muted uppercase">Alterações</h2>
        {data && !data.isClean && (
          <span className="rounded-full bg-raised px-2 py-0.5 text-xs text-muted">{data.summary.total}</span>
        )}
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
          <GroupedList data={data.files} selection={selection} onSelect={onSelect} />
        )}
      </div>
    </section>
  );
}

function GroupedList({
  data,
  selection,
  onSelect,
}: {
  data: Parameters<typeof groupChanges>[0];
  selection: Selection | null;
  onSelect: (selection: Selection) => void;
}) {
  const groups = groupChanges(data);
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
              />
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

function FileRow({ row, selected, onSelect }: { row: ChangeRow; selected: boolean; onSelect: () => void }) {
  const { dir, name } = splitPath(row.path);
  return (
    <li>
      <button
        type="button"
        onClick={onSelect}
        aria-current={selected ? "true" : undefined}
        title={row.path}
        className={`flex w-full items-center gap-2.5 px-4 py-1.5 text-left text-sm transition-colors ${
          selected ? "bg-accent/10 shadow-[inset_2px_0_0_var(--color-accent)]" : "hover:bg-raised/70"
        }`}
      >
        <StatusBadge badge={row.badge} />
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0">
            <span className="truncate text-muted">{dir}</span>
            <span className="max-w-full shrink-0 truncate">{name}</span>
          </span>
          {row.origPath && (
            <span className="block truncate text-xs text-faint">de {row.origPath}</span>
          )}
        </span>
      </button>
    </li>
  );
}
