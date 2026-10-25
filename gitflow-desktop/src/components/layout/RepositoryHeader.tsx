import type { StatusState } from "../../hooks/useRepositoryWorkspace";
import { describeBranch, describeTracking, summarizeState, type StateTone } from "../../lib/changes";
import type { RepositoryInfo } from "../../types/git";
import { Button } from "../common/Button";
import { BranchIcon, RefreshIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";

const CHIP_TONES: Record<StateTone, string> = {
  clean: "border-added/40 bg-added/10 text-added",
  dirty: "border-modified/40 bg-modified/10 text-modified",
  warning: "border-conflict/40 bg-conflict/10 text-conflict",
};

interface RepositoryHeaderProps {
  repo: RepositoryInfo;
  status: StatusState;
  onRefresh: () => void;
}

export function RepositoryHeader({ repo, status, onRefresh }: RepositoryHeaderProps) {
  const data = status.data;
  const state = data ? summarizeState(data) : null;
  const tracking = data ? describeTracking(data.branch) : null;

  return (
    <header className="border-b border-line bg-surface px-5 py-3">
      <div className="flex items-center gap-3">
        <h1 className="min-w-0 truncate text-base font-semibold" title={repo.name}>
          {repo.name}
        </h1>

        <span
          className="inline-flex min-w-0 items-center gap-1.5 rounded-md border border-line bg-raised px-2 py-0.5 text-sm"
          title={data ? `Branch atual: ${describeBranch(data.branch)}` : undefined}
        >
          <BranchIcon className="shrink-0 text-accent" />
          <span className="truncate font-mono text-[13px]">
            {data ? describeBranch(data.branch) : status.loading ? "Carregando…" : "Indisponível"}
          </span>
        </span>

        {tracking && <span className="hidden truncate text-xs text-muted lg:inline">{tracking}</span>}

        <div className="ml-auto flex shrink-0 items-center gap-3">
          {state && (
            <span
              className={`rounded-full border px-2.5 py-0.5 text-xs font-medium ${CHIP_TONES[state.tone]}`}
              aria-live="polite"
            >
              {state.label}
            </span>
          )}
          {status.updatedAt && !status.loading && (
            <span className="hidden text-xs text-muted xl:inline">
              Atualizado às {new Date(status.updatedAt).toLocaleTimeString("pt-BR")}
            </span>
          )}
          <Button onClick={onRefresh} disabled={status.loading} aria-label="Atualizar informações do repositório">
            {status.loading ? <Spinner /> : <RefreshIcon />}
            Atualizar
          </Button>
        </div>
      </div>

      <p className="mt-1 truncate text-xs text-muted" title={repo.path}>
        {repo.path}
        {repo.resolvedFromSubfolder && (
          <span className="text-faint"> · raiz do repositório detectada a partir de “{repo.selectedPath}”</span>
        )}
      </p>
    </header>
  );
}
