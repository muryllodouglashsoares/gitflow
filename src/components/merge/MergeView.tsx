import { useMemo } from "react";
import type { BranchesState } from "../../hooks/useBranches";
import type { MergeController } from "../../hooks/useMerge";
import { sortBranches } from "../../lib/branches";
import { selectionProblem } from "../../lib/merge";
import { PROTECTED_BRANCHES } from "../../lib/sync";
import type { BranchEntry, GitError } from "../../types/git";
import { Button } from "../common/Button";
import { EmptyState } from "../common/EmptyState";
import { ErrorBanner } from "../common/ErrorBanner";
import { AlertIcon, ArrowRightIcon, MergeIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";
import { MergeConfirm } from "./MergeConfirm";
import { MergePreviewPanel } from "./MergePreviewPanel";
import { MergeResultPanel } from "./MergeResultPanel";
import { BranchDetails, StageChip } from "./parts";

const isProtected = (name: string) => PROTECTED_BRANCHES.includes(name);

interface BranchSelectProps {
  id: string;
  label: string;
  hint: string;
  value: string | null;
  branches: BranchEntry[];
  /** Branch escolhida do outro lado: não pode ser escolhida aqui. */
  exclude: string | null;
  disabled: boolean;
  onChange: (name: string | null) => void;
}

function BranchSelect({ id, label, hint, value, branches, exclude, disabled, onChange }: BranchSelectProps) {
  const selected = branches.find((b) => b.name === value) ?? null;
  return (
    <div className="min-w-0">
      <label htmlFor={id} className="block text-xs font-medium text-muted">
        {label}
      </label>
      <select
        id={id}
        value={value ?? ""}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
        className="mt-1 w-full rounded-md border border-line bg-raised px-2.5 py-1.5 font-mono text-sm text-fg outline-none focus-visible:border-accent disabled:opacity-60"
      >
        <option value="">Selecione uma branch…</option>
        {branches.map((b) => (
          <option key={b.name} value={b.name} disabled={b.name === exclude}>
            {b.name}
            {b.current ? " (atual)" : ""}
            {isProtected(b.name) ? " (principal)" : ""}
          </option>
        ))}
      </select>
      <p className="mt-1 text-xs text-faint">{hint}</p>
      {selected && <BranchDetails branch={selected} protectedName={isProtected(selected.name)} />}
    </div>
  );
}

interface MergeViewProps {
  branches: BranchesState;
  ctl: MergeController;
  /** Há alguma operação de escrita em andamento (qualquer tipo). */
  locked: boolean;
  switching: boolean;
  switchError: GitError | null;
  onDismissSwitchError: () => void;
  onReloadBranches: () => void;
  onOpenChanges: () => void;
}

export function MergeView({
  branches,
  ctl,
  locked,
  switching,
  switchError,
  onDismissSwitchError,
  onReloadBranches,
  onOpenChanges,
}: MergeViewProps) {
  const list = branches.data;
  const sorted = useMemo(() => (list ? sortBranches(list.branches) : []), [list]);
  const { source, target, preview, stage, phase } = ctl;
  const running = phase === "running";
  const busy = running || locked;
  const problem = selectionProblem(source, target);
  const p = preview.data;
  const bothChosen = source !== null && target !== null && source !== target;

  return (
    <main className="min-w-0 flex-1 overflow-y-auto bg-canvas" aria-label="Merge entre branches">
      <div className="mx-auto max-w-3xl space-y-4 p-5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2.5">
            <h2 className="text-sm font-semibold">Merge entre branches</h2>
            <StageChip stage={stage} />
          </div>
          {!ctl.result && (
            <Button
              onClick={() => {
                ctl.recheck();
                onReloadBranches();
              }}
              disabled={busy || branches.loading || preview.loading}
            >
              {branches.loading || preview.loading ? <Spinner /> : null}
              Verificar novamente
            </Button>
          )}
        </div>

        {ctl.result ? (
          <MergeResultPanel result={ctl.result} onFinish={ctl.finish} onOpenChanges={onOpenChanges} />
        ) : branches.error ? (
          <ErrorBanner title="Não foi possível listar as branches" error={branches.error} />
        ) : !list ? (
          <EmptyState icon={<Spinner />} title="Lendo as branches…" description="Consultando as branches locais no Git." />
        ) : list.unborn || sorted.length < 2 ? (
          <EmptyState
            icon={<MergeIcon width={22} height={22} />}
            title="É preciso ter pelo menos duas branches"
            description={
              list.unborn
                ? "Este repositório ainda não tem commits. Crie o primeiro commit antes de integrar branches."
                : "O merge integra os commits de uma branch em outra. Crie outra branch na aba Branches."
            }
          />
        ) : (
          <>
            {ctl.notice && (
              <div role="status" className="flex items-start gap-2.5 rounded-md border border-modified/40 bg-modified/10 p-3 text-sm">
                <AlertIcon className="mt-0.5 shrink-0 text-modified" />
                <p className="min-w-0 flex-1">{ctl.notice}</p>
                <button
                  type="button"
                  onClick={ctl.dismissNotice}
                  className="rounded px-1.5 text-xs text-muted hover:bg-raised hover:text-fg"
                >
                  Fechar
                </button>
              </div>
            )}
            {ctl.runError && (
              <ErrorBanner title="A integração não foi executada" error={ctl.runError} onDismiss={ctl.dismissRunError} />
            )}
            {switchError && (
              <ErrorBanner title="Não foi possível trocar de branch" error={switchError} onDismiss={onDismissSwitchError} />
            )}

            <section aria-label="Seleção de branches" className="rounded-md border border-line bg-surface p-4">
              <div className="grid gap-4 sm:grid-cols-[1fr_auto_1fr] sm:items-start">
                <BranchSelect
                  id="merge-source"
                  label="Origem"
                  hint="Seus commits serão integrados."
                  value={source}
                  branches={sorted}
                  exclude={target}
                  disabled={busy || phase === "confirming"}
                  onChange={ctl.setSource}
                />
                <div className="flex items-center justify-center sm:pt-6">
                  <button
                    type="button"
                    onClick={ctl.swap}
                    disabled={busy || phase === "confirming" || (!source && !target)}
                    aria-label="Inverter origem e destino"
                    title="Inverter origem e destino"
                    className="rounded-md border border-line bg-raised p-2 text-muted hover:border-faint hover:text-fg disabled:opacity-60"
                  >
                    <ArrowRightIcon />
                  </button>
                </div>
                <BranchSelect
                  id="merge-target"
                  label="Destino"
                  hint="Recebe os commits; só ele é modificado."
                  value={target}
                  branches={sorted}
                  exclude={source}
                  disabled={busy || phase === "confirming"}
                  onChange={ctl.setTarget}
                />
              </div>

              {problem && (
                <p role="alert" className="mt-3 text-sm text-removed">
                  {problem}
                </p>
              )}

              {bothChosen && (
                <div className="mt-4 rounded-md border border-line bg-canvas px-3 py-2.5" aria-label="Direção da integração">
                  <p className="flex flex-wrap items-center gap-2 font-mono text-sm">
                    <span>{source}</span>
                    <ArrowRightIcon className="text-accent" />
                    <span className={isProtected(target) ? "text-modified" : ""}>{target}</span>
                  </p>
                  <p className="mt-1 text-xs text-muted">
                    Os commits de <span className="font-mono">{source}</span> serão integrados em{" "}
                    <span className="font-mono">{target}</span>.
                  </p>
                </div>
              )}
            </section>

            {!bothChosen && !problem && (
              <p className="text-sm text-muted">
                Escolha a origem e o destino. Nada é alterado enquanto você apenas seleciona: o aplicativo só
                analisa o repositório e mostra o que aconteceria.
              </p>
            )}

            {bothChosen && preview.error && (
              <ErrorBanner title="Não foi possível analisar a integração" error={preview.error} />
            )}
            {bothChosen && !p && !preview.error && (
              <EmptyState icon={<Spinner />} title="Validando…" description="Verificando branches, alterações locais e divergência." />
            )}

            {bothChosen && p && (
              <>
                {p.targetProtected && (
                  <div className="flex items-start gap-2.5 rounded-md border border-modified/40 bg-modified/10 p-3 text-sm text-modified">
                    <AlertIcon className="mt-0.5 shrink-0" />
                    <p>
                      O destino é a branch principal <span className="font-mono">{p.target.name}</span>. A integração a
                      modificará e exige uma confirmação específica, também verificada no backend.
                    </p>
                  </div>
                )}

                <MergePreviewPanel
                  preview={p}
                  refreshing={preview.loading}
                  locked={busy}
                  switching={switching}
                  onSwitchToTarget={() => void ctl.switchToTarget()}
                />

                {running ? (
                  <div role="status" aria-live="polite" className="flex items-start gap-2.5 rounded-md border border-line bg-surface p-3 text-sm">
                    <Spinner className="mt-0.5 shrink-0 text-accent" />
                    <div>
                      <p className="font-medium">Executando merge…</p>
                      <p className="mt-0.5 text-muted">
                        Aguardando o Git. O resultado será conferido no repositório antes de ser exibido.
                      </p>
                    </div>
                  </div>
                ) : phase === "confirming" ? (
                  <MergeConfirm
                    preview={p}
                    acknowledged={ctl.acknowledged}
                    typed={ctl.typed}
                    ready={ctl.ready && !busy}
                    onAcknowledge={ctl.setAcknowledged}
                    onType={ctl.setTyped}
                    onCancel={ctl.cancelConfirmation}
                    onConfirm={() => void ctl.execute()}
                  />
                ) : (
                  <div className="flex items-center gap-3">
                    <Button variant="primary" onClick={ctl.prepare} disabled={!p.canExecute || busy || preview.loading}>
                      Preparar integração
                    </Button>
                    <p className="text-xs text-muted">
                      Abre a confirmação. O merge só é executado depois que você confirmar.
                    </p>
                  </div>
                )}
              </>
            )}
          </>
        )}
      </div>
    </main>
  );
}
