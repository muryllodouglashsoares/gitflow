import type { DiffState } from "../../hooks/useFileDiff";
import { selectionKey, splitPath, type Selection } from "../../lib/changes";
import type { DiffLine, DiffTarget, FileDiff, LineKind } from "../../types/git";
import { Button } from "../common/Button";
import { EmptyState } from "../common/EmptyState";
import { ErrorBanner } from "../common/ErrorBanner";
import { FileIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";

const TARGET_LABELS: Record<DiffTarget, string> = {
  staged: "Preparada (staged)",
  unstaged: "Não preparada",
  untracked: "Não rastreado (prévia do arquivo novo)",
  conflicted: "Conflito",
};

const LINE_STYLES: Record<LineKind, string> = {
  context: "",
  added: "bg-added/12 text-fg",
  removed: "bg-removed/12 text-fg",
  noNewline: "text-faint italic",
};

const MARKERS: Record<LineKind, string> = { context: " ", added: "+", removed: "-", noNewline: "" };
const MARKER_COLORS: Record<LineKind, string> = {
  context: "text-faint",
  added: "text-added",
  removed: "text-removed",
  noNewline: "text-faint",
};

interface DiffViewerProps {
  selection: Selection | null;
  diff: DiffState;
  onRetry: () => void;
}

export function DiffViewer({ selection, diff, onRetry }: DiffViewerProps) {
  if (!selection) {
    return (
      <main className="min-w-0 flex-1 bg-canvas" aria-label="Diferenças">
        <EmptyState
          icon={<FileIcon width={28} height={28} />}
          title="Selecione um arquivo"
          description="Escolha um arquivo na lista de alterações para ver as diferenças. A visualização é somente leitura."
        />
      </main>
    );
  }

  const { dir, name } = splitPath(selection.path);

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-canvas" aria-label="Diferenças">
      <div className="flex items-center gap-3 border-b border-line bg-surface px-4 py-2.5">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm" title={selection.path}>
            <span className="text-muted">{dir}</span>
            <span className="font-medium">{name}</span>
          </p>
          <p className="text-xs text-muted">
            {TARGET_LABELS[selection.target]}
            {diff.data?.renamedFrom && <span> · renomeado de {diff.data.renamedFrom}</span>}
            {diff.data?.isNew && selection.target !== "untracked" && <span> · arquivo novo</span>}
            {diff.data?.isDeleted && <span> · arquivo removido</span>}
          </p>
        </div>
        {diff.data && diff.data.content === "text" && (
          <p className="shrink-0 font-mono text-xs" aria-label={`${diff.data.additions} adições, ${diff.data.deletions} remoções`}>
            <span className="text-added">+{diff.data.additions}</span>{" "}
            <span className="text-removed">−{diff.data.deletions}</span>
          </p>
        )}
        {diff.loading && <Spinner className="text-muted" />}
        <span className="shrink-0 rounded border border-line px-1.5 py-0.5 text-[11px] text-muted">
          Somente leitura
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-auto" aria-busy={diff.loading} data-selection={selectionKey(selection)}>
        <DiffBody diff={diff} onRetry={onRetry} />
      </div>
    </main>
  );
}

function DiffBody({ diff, onRetry }: { diff: DiffState; onRetry: () => void }) {
  if (diff.error) {
    return (
      <div className="p-4">
        <ErrorBanner
          title="Não foi possível gerar as diferenças"
          error={diff.error}
          actions={<Button onClick={onRetry}>Tentar novamente</Button>}
        />
      </div>
    );
  }
  if (!diff.data) {
    return <EmptyState icon={<Spinner className="h-5 w-5" />} title="Carregando diferenças…" />;
  }

  const data = diff.data;
  if (data.content !== "text") {
    return (
      <EmptyState
        icon={<FileIcon width={28} height={28} />}
        title={data.content === "binary" ? "Arquivo binário" : "Nada para exibir"}
        description={data.notice ?? "Não há diferenças disponíveis."}
      />
    );
  }

  return <TextDiff data={data} />;
}

function TextDiff({ data }: { data: FileDiff }) {
  return (
    <div className="min-w-max font-mono text-[12.5px] leading-5">
      {data.truncated && (
        <p role="status" className="sticky left-0 border-b border-modified/40 bg-modified/10 px-4 py-2 font-sans text-xs text-modified">
          O diff é muito grande e foi cortado: apenas o início é exibido.
        </p>
      )}
      {data.hunks.map((hunk, index) => (
        <section key={index} aria-label={`Trecho ${index + 1}`}>
          <h3 className="sticky left-0 border-y border-line bg-surface px-4 py-1 font-normal text-renamed">
            {hunk.header}
          </h3>
          <table className="w-full border-collapse">
            <tbody>
              {hunk.lines.map((line, i) => (
                <Row key={i} line={line} />
              ))}
            </tbody>
          </table>
        </section>
      ))}
    </div>
  );
}

function Row({ line }: { line: DiffLine }) {
  const label =
    line.kind === "added" ? "Linha adicionada" : line.kind === "removed" ? "Linha removida" : undefined;
  return (
    <tr className={LINE_STYLES[line.kind]} aria-label={label}>
      <td className="w-12 min-w-12 px-2 text-right text-faint select-none">{line.oldNo ?? ""}</td>
      <td className="w-12 min-w-12 border-r border-line px-2 text-right text-faint select-none">
        {line.newNo ?? ""}
      </td>
      <td className={`w-5 px-1 text-center select-none ${MARKER_COLORS[line.kind]}`} aria-hidden="true">
        {MARKERS[line.kind]}
      </td>
      <td className="pr-6 pl-1 whitespace-pre">{line.text}</td>
    </tr>
  );
}
