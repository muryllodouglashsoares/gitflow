import { useMemo, useState } from "react";
import {
  ACTION_LABELS,
  actionTone,
  blockedEntries,
  formatBytes,
  workBranchProblem,
  type ActionTone,
} from "../../lib/importFlow";
import type { EntryAction, PlanEntry } from "../../types/git";
import type { ZipImportController } from "../../hooks/useZipImport";
import { Button } from "../common/Button";
import { AlertIcon } from "../common/icons";
import { Card, Check, Chip, PathList } from "./parts";

const TONE_CLASS: Record<ActionTone, string> = {
  added: "bg-added/15 text-added",
  modified: "bg-modified/15 text-modified",
  untracked: "bg-accent/15 text-accent",
  conflict: "bg-conflict/15 text-conflict",
  deleted: "bg-removed/15 text-removed",
};

type Filter = "all" | EntryAction;
const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "Todos" },
  { id: "add", label: "Novos" },
  { id: "replace", label: "Substituídos" },
  { id: "identical", label: "Idênticos" },
  { id: "blocked", label: "Bloqueados" },
];

/** Prévia completa do que o ZIP fará, antes de qualquer gravação. */
export function PlanPanel({ ctl, existingBranches }: { ctl: ZipImportController; existingBranches: string[] }) {
  const { flow } = ctl;
  const plan = flow.plan;
  const [filter, setFilter] = useState<Filter>("all");
  const rows = useMemo(
    () => (plan ? plan.entries.filter((e) => filter === "all" || e.action === filter) : []),
    [plan, filter],
  );
  if (!plan) return null;

  const blocked = blockedEntries(plan);
  const excluded = new Set(flow.excluded);
  const s = plan.summary;
  const nameProblem = workBranchProblem(flow.workBranch, flow.baseBranch, existingBranches);
  const locked = ctl.running;

  return (
    <div className="space-y-4">
      <Card title="Plano de importação">
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
          <Chip label="novos" value={s.add} tone="text-added" />
          <Chip label="substituem um arquivo" value={s.replace} tone="text-modified" />
          <Chip label="idênticos (sem alteração)" value={s.identical} />
          <Chip label="bloqueados" value={s.blocked} tone={s.blocked > 0 ? "text-conflict" : ""} />
          <Chip label="fora da pasta escolhida" value={s.ignored} />
        </div>
        <p className="mt-3 text-xs text-muted">
          A gravar: {formatBytes(s.bytesToWrite)} · conteúdo descomprimido do ZIP: {formatBytes(s.uncompressedBytes)} ·
          ZIP: {formatBytes(plan.zipSize)} · SHA-256 <span className="font-mono">{plan.zipSha256.slice(0, 12)}…</span>
        </p>
        <p className="mt-1 text-xs text-muted">
          Nada é excluído: arquivos do repositório que não estão no ZIP permanecem como estão. Nada foi extraído ainda.
        </p>

        {plan.warnings.length > 0 && (
          <ul className="mt-3 list-disc space-y-1 pl-5 text-muted">
            {plan.warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        )}

        {plan.blockers.length > 0 && (
          <div role="alert" className="mt-3 rounded-md border border-modified/40 bg-modified/10 p-3">
            <p className="flex items-center gap-2 font-medium text-modified">
              <AlertIcon /> O repositório não está pronto para a importação
            </p>
            <ul className="mt-1.5 list-disc space-y-1 pl-5">
              {plan.blockers.map((b) => (
                <li key={b}>{b}</li>
              ))}
            </ul>
          </div>
        )}

        {blocked.length > 0 && (
          <div role="alert" className="mt-3 rounded-md border border-conflict/40 bg-conflict/10 p-3">
            <p className="flex items-center gap-2 font-medium text-conflict">
              <AlertIcon /> {blocked.length} entrada(s) bloqueada(s) por segurança — a importação não pode prosseguir
            </p>
            <ul className="mt-1.5 space-y-1">
              {blocked.slice(0, 20).map((entry) => (
                <li key={`${entry.zipIndex}:${entry.path}`}>
                  <span className="font-mono text-xs break-all">{entry.path}</span>
                  <span className="text-muted"> — {entry.reason}</span>
                </li>
              ))}
              {blocked.length > 20 && <li className="text-muted">… e mais {blocked.length - 20}</li>}
            </ul>
            <p className="mt-2 text-xs text-muted">Corrija o pacote e inspecione novamente.</p>
          </div>
        )}
      </Card>

      <Card title="Arquivos do plano">
        <div className="mb-2 flex flex-wrap items-center gap-1.5" role="group" aria-label="Filtrar arquivos">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              type="button"
              onClick={() => setFilter(f.id)}
              aria-pressed={filter === f.id}
              className={`rounded-full border px-2.5 py-0.5 text-xs ${
                filter === f.id ? "border-accent bg-accent/15 text-fg" : "border-line text-muted hover:text-fg"
              }`}
            >
              {f.label}
            </button>
          ))}
          {flow.excluded.length > 0 && (
            <span className="ml-auto text-xs text-modified">{flow.excluded.length} excluído(s) por você</span>
          )}
        </div>
        <div className="max-h-96 overflow-auto rounded-md border border-line">
          <table className="w-full text-left text-xs">
            <thead className="sticky top-0 bg-raised text-muted">
              <tr>
                <th scope="col" className="w-10 px-2 py-1.5 font-medium">
                  <span className="sr-only">Importar</span>
                </th>
                <th scope="col" className="w-28 px-2 py-1.5 font-medium">Ação</th>
                <th scope="col" className="px-2 py-1.5 font-medium">Caminho no repositório</th>
                <th scope="col" className="w-20 px-2 py-1.5 text-right font-medium">Tamanho</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((entry) => (
                <Row
                  key={`${entry.zipIndex}:${entry.path}`}
                  entry={entry}
                  excluded={excluded.has(entry.path)}
                  disabled={locked}
                  onToggle={() => ctl.toggleExclude(entry.path)}
                />
              ))}
              {rows.length === 0 && (
                <tr>
                  <td colSpan={4} className="px-3 py-4 text-center text-muted">
                    Nenhum arquivo neste filtro.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="mt-2 text-xs text-muted">
          Desmarque um arquivo para não importá-lo. Idênticos e bloqueados não podem ser importados.
        </p>
      </Card>

      {ctl.overwrites.length > 0 && (
        <Card title={`Atenção: ${ctl.overwrites.length} arquivo(s) existente(s) será(ão) substituído(s)`} tone="warning">
          <p className="text-muted">
            O conteúdo atual destes arquivos será trocado pelo do ZIP. Uma cópia de segurança fica em .git/gitflow-import
            até o fim da operação, e o diff completo será mostrado depois da importação.
          </p>
          <PathList paths={ctl.overwrites.map((e) => e.path)} />
          <div className="mt-3">
            <Check checked={ctl.overwriteAck} onChange={ctl.setOverwriteAck} disabled={locked}>
              Entendo que estes arquivos existentes serão substituídos.
            </Check>
          </div>
        </Card>
      )}

      <Card title="Branch de trabalho">
        <p className="text-muted">
          A importação nunca grava na branch atual. Uma nova branch é criada a partir de{" "}
          <span className="font-mono text-fg">{flow.baseBranch}</span>
          {plan.baseOid && <span className="font-mono"> ({plan.baseOid.slice(0, 7)})</span>} e ativada; só então os
          arquivos são gravados nela.
        </p>
        <label className="mt-3 block">
          <span className="text-xs font-medium text-muted">Nome da nova branch (sugestão editável)</span>
          <input
            type="text"
            value={flow.workBranch}
            onChange={(event) => ctl.changeWorkBranch(event.target.value)}
            disabled={locked}
            spellCheck={false}
            autoComplete="off"
            aria-invalid={nameProblem !== null}
            aria-describedby="work-branch-problem"
            className="mt-1 w-full rounded-md border border-line bg-canvas px-2.5 py-1.5 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent"
          />
        </label>
        <p id="work-branch-problem" className={`mt-1 min-h-5 text-xs ${nameProblem ? "text-removed" : "text-added"}`}>
          {nameProblem ?? "Nome disponível. O Git confirma a validade ao criar a branch."}
        </p>
      </Card>

      <Card title="Confirmar e importar">
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
          <dt className="text-muted">Repositório</dt>
          <dd className="font-mono break-all">{plan.repoPath}</dd>
          <dt className="text-muted">Base → nova branch</dt>
          <dd className="font-mono">
            {flow.baseBranch} → {flow.workBranch || "…"}
          </dd>
          <dt className="text-muted">Será gravado</dt>
          <dd>
            {ctl.writes.length} arquivo(s): {ctl.writes.filter((e) => e.action === "add").length} novo(s),{" "}
            {ctl.overwrites.length} substituição(ões)
          </dd>
          <dt className="text-muted">Depois disso</dt>
          <dd>Nada é commitado nem enviado: você revisa o diff e confirma cada passo.</dd>
        </dl>
        <div className="mt-3">
          <Check checked={ctl.planAck} onChange={ctl.setPlanAck} disabled={locked}>
            Confirmo este plano e a criação da branch <span className="font-mono">{flow.workBranch || "…"}</span>.
          </Check>
        </div>
        {ctl.problems.length > 0 && (
          <ul aria-live="polite" className="mt-3 list-disc space-y-1 pl-5 text-xs text-modified">
            {ctl.problems.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
        )}
        <div className="mt-4 flex flex-wrap gap-2">
          <Button variant="primary" disabled={locked || ctl.problems.length > 0} onClick={() => void ctl.startImport()}>
            Criar branch e importar {ctl.writes.length} arquivo(s)
          </Button>
        </div>
      </Card>
    </div>
  );
}

function Row({
  entry,
  excluded,
  disabled,
  onToggle,
}: {
  entry: PlanEntry;
  excluded: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  const writable = entry.action === "add" || entry.action === "replace";
  return (
    <tr className={`border-t border-line ${excluded ? "opacity-50" : ""}`}>
      <td className="px-2 py-1.5">
        {writable ? (
          <input
            type="checkbox"
            checked={!excluded}
            disabled={disabled}
            onChange={onToggle}
            aria-label={`Importar ${entry.path}`}
            className="h-4 w-4 accent-[var(--color-accent)]"
          />
        ) : null}
      </td>
      <td className="px-2 py-1.5">
        <span className={`rounded px-1.5 py-0.5 text-[11px] font-medium ${TONE_CLASS[actionTone(entry.action)]}`}>
          {ACTION_LABELS[entry.action]}
        </span>
      </td>
      <td className="px-2 py-1.5">
        <span className="font-mono break-all">{entry.path}</span>
        {entry.reason && <span className="block text-muted">{entry.reason}</span>}
      </td>
      <td className="px-2 py-1.5 text-right text-muted">{entry.action === "blocked" ? "—" : formatBytes(entry.size)}</td>
    </tr>
  );
}
