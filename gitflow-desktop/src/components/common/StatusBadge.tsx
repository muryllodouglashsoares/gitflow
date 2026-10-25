import type { Badge, BadgeTone } from "../../lib/changes";

const TONES: Record<BadgeTone, string> = {
  added: "bg-added/15 text-added",
  modified: "bg-modified/15 text-modified",
  deleted: "bg-removed/15 text-removed",
  renamed: "bg-renamed/15 text-renamed",
  untracked: "bg-accent/15 text-accent",
  conflict: "bg-conflict/15 text-conflict",
};

export function StatusBadge({ badge }: { badge: Badge }) {
  return (
    <span
      role="img"
      aria-label={badge.label}
      title={badge.label}
      className={`inline-flex h-5 w-5 shrink-0 items-center justify-center rounded font-mono text-[11px] font-semibold ${TONES[badge.tone]}`}
    >
      {badge.symbol}
    </span>
  );
}
