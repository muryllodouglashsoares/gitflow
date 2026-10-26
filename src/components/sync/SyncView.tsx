import { useEffect, useState } from "react";
import type { SyncController } from "../../hooks/useSync";
import {
  countsText,
  describeSync,
  fetchBlock,
  formatAge,
  lastFetchMs,
  pullBlock,
  pushBlock,
  syncTarget,
  type SyncTone,
} from "../../lib/sync";
import type { RepositoryStatus, SyncTarget } from "../../types/git";
import { Button } from "../common/Button";
import { EmptyState } from "../common/EmptyState";
import { ErrorBanner } from "../common/ErrorBanner";
import { SyncIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";
import { OperationFeedback } from "./OperationFeedback";
import { SyncConfirm } from "./SyncConfirm";

const TONES: Record<SyncTone, string> = {
  ok: "border-added/40 bg-added/10 text-added",
  info: "border-accent/40 bg-accent/10 text-accent",
  warning: "border-modified/40 bg-modified/10 text-modified",
  danger: "border-removed/40 bg-removed/10 text-removed",
  neutral: "border-line bg-raised text-muted",
};

const targetKey = (t: SyncTarget | null) => (t ? `${t.branch}\u0000${t.remote}\u0000${t.remoteBranch}` : "");

interface SyncViewProps {
  status: RepositoryStatus | null;
  sync: SyncController;
  /** Há alguma operação de escrita em andamento (qualquer tipo). */
  locked: boolean;
}

export function SyncView({ status, sync, locked }: SyncViewProps) {
  const { diagnosis, op, selectedRemote } = sync;
  const data = diagnosis.data;
  const running = op.phase === "running";
  const busy = running || locked;

  const [confirm, setConfirm] = useState<{ kind: "pull" | "push"; target: SyncTarget } | null>(null);
  const [now, setNow] = useState(() => Date.now());

  // O relógio só alimenta o texto "há N min" e a atualidade das referências.
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(id);
  }, []);
  useEffect(() => setNow(Date.now()), [data]);

  const target = syncTarget(data);
  // Se o destino mudar enquanto a confirmação está aberta, ela perde a validade.
  useEffect(() => {
    if (confirm && targetKey(confirm.target) !== targetKey(target)) setConfirm(null);
  }, [confirm, target]);
  // Qualquer operação em andamento também fecha a confirmação.
  useEffect(() => {
    if (running) setConfirm(null);
  }, [running]);

  const summary = data
    ? describeSync(data, { fetchFailure: sync.fetchFailure, sessionFetchAt: sync.sessionFetchAt, now })
    : null;
  const lastMs = data ? lastFetchMs(data, sync.sessionFetchAt) : null;

  const fetchReason = fetchBlock(data, selectedRemote);
  const pullReason = pullBlock(data, status);
  const pushReason = pushBlock(data, status);

  return (
    <main className="min-w-0 flex-1 overflow-y-auto bg-canvas" aria-label="Sincronização remota">
      <div className="mx-auto max-w-3xl space-y-4 p-5">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold">Sincronização remota</h2>
          <Button onClick={sync.reload} disabled={busy || diagnosis.loading}>
            {diagnosis.loading ? <Spinner /> : null}
            Atualizar diagnóstico
          </Button>
        </div>

        <OperationFeedback op={op} onDismiss={sync.dismiss} />

        {diagnosis.error ? (
          <ErrorBanner title="Não foi possível ler o estado de sincronização" error={diagnosis.error} />
        ) : !data || !summary ? (
          <EmptyState
            icon={<Spinner />}
            title="Lendo o estado de sincronização…"
            description="Consultando remotos e upstream no Git (sem usar a rede)."
          />
        ) : (
          <>
            <section aria-label="Resumo de sincronização" className="rounded-md border border-line bg-surface p-4">
              <div className="flex flex-wrap items-center gap-2">
                <span
                  className={`rounded-full border px-2.5 py-0.5 text-xs font-medium ${TONES[summary.tone]}`}
                  aria-live="polite"
                >
                  {summary.label}
                </span>
                {diagnosis.loading && <Spinner className="text-muted" />}
              </div>
              <p className="mt-2 text-sm text-muted">{summary.description}</p>

              <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
                <dt className="text-muted">Branch local</dt>
                <dd className="font-mono">{data.branch ?? "(HEAD desanexado)"}</dd>
                <dt className="text-muted">Upstream</dt>
                <dd className="font-mono">{data.upstream ? data.upstream.display : "—"}</dd>
                <dt className="text-muted">Commits</dt>
                <dd>{countsText(data) ?? "desconhecido (sem referência remota atualizada)"}</dd>
                <dt className="text-muted">Última atualização do remoto</dt>
                <dd>{lastMs === null ? "nunca registrada" : formatAge(now - lastMs)}</dd>
              </dl>
            </section>

            <section aria-label="Remotos" className="rounded-md border border-line bg-surface p-4">
              <h3 className="text-sm font-semibold">Remotos</h3>
              {data.remotes.length === 0 ? (
                <p className="mt-1.5 text-sm text-muted">Nenhum remoto configurado.</p>
              ) : (
                <ul className="mt-2 space-y-1.5 text-sm">
                  {data.remotes.map((remote) => (
                    <li key={remote.name} className="flex flex-wrap items-baseline gap-x-3">
                      <span className="font-mono">{remote.name}</span>
                      <span className="min-w-0 break-all text-xs text-muted">
                        {remote.fetchUrl ?? "URL indisponível"}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section aria-label="Ações de sincronização" className="rounded-md border border-line bg-surface p-4">
              <h3 className="text-sm font-semibold">Ações</h3>

              <div className="mt-3 space-y-4">
                <div>
                  <div className="flex flex-wrap items-center gap-2">
                    {data.remotes.length > 1 && (
                      <label className="flex items-center gap-2 text-sm text-muted">
                        Remoto
                        <select
                          value={selectedRemote ?? ""}
                          onChange={(e) => sync.selectRemote(e.target.value)}
                          disabled={busy}
                          className="rounded-md border border-line bg-raised px-2 py-1 font-mono text-sm text-fg"
                        >
                          {data.remotes.map((r) => (
                            <option key={r.name} value={r.name}>
                              {r.name}
                            </option>
                          ))}
                        </select>
                      </label>
                    )}
                    <Button
                      onClick={() => selectedRemote && sync.fetch(selectedRemote)}
                      disabled={busy || fetchReason !== null}
                      aria-describedby="fetch-help"
                    >
                      {running && op.kind === "fetch" ? <Spinner /> : <SyncIcon />}
                      Atualizar referências (fetch){selectedRemote ? ` · ${selectedRemote}` : ""}
                    </Button>
                  </div>
                  <p id="fetch-help" className="mt-1 text-xs text-muted">
                    {fetchReason?.reason ??
                      "Só atualiza as referências do remoto. Não altera seus arquivos nem integra commits."}
                  </p>
                </div>

                <div>
                  <Button
                    onClick={() => target && setConfirm({ kind: "pull", target })}
                    disabled={busy || pullReason !== null || target === null}
                    aria-describedby="pull-help"
                  >
                    Baixar e integrar (pull)
                  </Button>
                  <p id="pull-help" className="mt-1 text-xs text-muted">
                    {pullReason?.reason ?? `Integra as alterações de ${data.upstream?.display ?? "o upstream"} em ${data.branch ?? "a branch atual"}.`}
                  </p>
                  {pullReason?.files && pullReason.files.length > 0 && (
                    <ul className="mt-1 max-h-24 overflow-auto font-mono text-xs break-all text-modified">
                      {pullReason.files.map((path) => (
                        <li key={path}>{path}</li>
                      ))}
                    </ul>
                  )}
                </div>

                <div>
                  <Button
                    onClick={() => target && setConfirm({ kind: "push", target })}
                    disabled={busy || pushReason !== null || target === null}
                    aria-describedby="push-help"
                  >
                    Enviar commits (push)
                  </Button>
                  <p id="push-help" className="mt-1 text-xs text-muted">
                    {pushReason?.reason ?? `Envia os commits de ${data.branch ?? "a branch atual"} para ${data.upstream?.display ?? "o upstream"}.`}
                  </p>
                </div>
              </div>
            </section>

            {confirm && (
              <SyncConfirm
                kind={confirm.kind}
                sync={data}
                target={confirm.target}
                onCancel={() => setConfirm(null)}
                onConfirm={(confirmProtected) => {
                  const { kind, target: confirmed } = confirm;
                  setConfirm(null);
                  if (kind === "pull") sync.pull(confirmed);
                  else sync.push(confirmed, confirmProtected);
                }}
              />
            )}

            <p className="text-xs text-faint">
              A autenticação é a que o seu Git já usa (Git Credential Manager ou SSH). O aplicativo não pede nem
              guarda senhas ou tokens: se o remoto exigir login, autentique uma vez fora do aplicativo.
            </p>
          </>
        )}
      </div>
    </main>
  );
}
