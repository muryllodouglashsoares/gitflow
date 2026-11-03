import type { ReactNode } from "react";
import { STAGE_LABELS, STAGE_TONES, shortOid, type MergeStage, type MergeTone } from "../../lib/merge";
import type { BranchEntry } from "../../types/git";

export const TONE_CARD: Record<MergeTone, string> = {
  ok: "border-added/40 bg-added/10",
  info: "border-accent/40 bg-accent/10",
  warning: "border-modified/40 bg-modified/10",
  danger: "border-removed/40 bg-removed/10",
  neutral: "border-line bg-raised",
};

export const TONE_TEXT: Record<MergeTone, string> = {
  ok: "text-added",
  info: "text-accent",
  warning: "text-modified",
  danger: "text-removed",
  neutral: "text-muted",
};

/** Estado atual do fluxo (anunciado a leitores de tela quando muda). */
export function StageChip({ stage }: { stage: MergeStage }) {
  const tone = STAGE_TONES[stage];
  return (
    <span
      aria-live="polite"
      className={`rounded-full border px-2.5 py-0.5 text-xs font-medium ${TONE_CARD[tone]} ${TONE_TEXT[tone]}`}
    >
      {STAGE_LABELS[stage]}
    </span>
  );
}

/** Rótulos curtos de uma branch: atual e principal. */
export function BranchTags({ branch, protectedName }: { branch: BranchEntry; protectedName: boolean }) {
  return (
    <>
      {branch.current && (
        <span className="rounded bg-accent/15 px-1.5 py-0.5 text-[11px] font-medium text-accent">atual</span>
      )}
      {protectedName && (
        <span className="rounded bg-modified/15 px-1.5 py-0.5 text-[11px] font-medium text-modified">principal</span>
      )}
    </>
  );
}

export function BranchDetails({ branch, protectedName }: { branch: BranchEntry; protectedName: boolean }) {
  return (
    <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted">
      <span className="font-mono">{shortOid(branch.oid)}</span>
      <BranchTags branch={branch} protectedName={protectedName} />
      {branch.subject && <span className="min-w-0 max-w-full truncate" title={branch.subject}>{branch.subject}</span>}
    </div>
  );
}

/** Lista de caminhos de arquivos (com “… e mais N” quando o backend limitou a lista). */
export function FileList({
  files,
  total,
  label,
  trailing,
}: {
  files: string[];
  total: number;
  label: string;
  trailing?: (path: string) => ReactNode;
}) {
  return (
    <ul aria-label={label} className="mt-2 max-h-44 space-y-0.5 overflow-auto rounded bg-canvas/60 p-2 font-mono text-xs">
      {files.map((path) => (
        <li key={path} className="flex items-baseline gap-2 break-all">
          <span>{path}</span>
          {trailing?.(path)}
        </li>
      ))}
      {total > files.length && <li className="text-muted">… e mais {total - files.length}</li>}
    </ul>
  );
}

export function OutputDetails({ text }: { text: string }) {
  if (!text.trim()) return null;
  return (
    <details className="mt-3 text-xs text-muted">
      <summary className="cursor-pointer select-none hover:text-fg">Saída do Git</summary>
      <pre className="mt-1.5 max-h-48 overflow-auto rounded bg-canvas/60 p-2 font-mono whitespace-pre-wrap break-words">
        {text}
      </pre>
    </details>
  );
}
