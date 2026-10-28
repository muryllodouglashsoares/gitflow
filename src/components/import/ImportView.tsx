import type { ReactNode } from "react";
import { STAGE_LABELS, isEngaged, type ImportStage } from "../../lib/importFlow";
import type { ZipImportController } from "../../hooks/useZipImport";
import type { BranchList, SyncStatus } from "../../types/git";
import { ChangesPanel } from "../changes/ChangesPanel";
import { Button } from "../common/Button";
import { ErrorBanner } from "../common/ErrorBanner";
import { CheckCircleIcon, ImportIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";
import { CommitForm } from "../commit/CommitForm";
import { ReviewFileList } from "../commit/ReviewFileList";
import { DiffViewer } from "../diff/DiffViewer";
import { PlanPanel } from "./PlanPanel";
import { Card, Check, ConfirmButton, OperationLog, PathList, Stepper, Working } from "./parts";

interface ImportViewProps {
  repoPath: string;
  ctl: ZipImportController;
  branches: BranchList | null;
  /** Há outra operação de escrita em andamento fora deste fluxo. */
  locked: boolean;
  syncStatus: SyncStatus | null;
  onRefresh: () => void;
}

const SELECTION_STAGES: readonly ImportStage[] = ["selecting", "inspecting", "zipInvalid", "planReview", "cancelled"];

export function ImportView({ repoPath, ctl, branches, locked, syncStatus, onRefresh }: ImportViewProps) {
  const { flow } = ctl;
  const stage = flow.stage;
  const commitStage = stage === "commitReview" || stage === "committing";

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="border-b border-line bg-surface px-4 py-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="flex items-center gap-2 text-sm font-semibold">
            <ImportIcon className="text-accent" /> Importar atualização por ZIP
          </h2>
          <span
            role="status"
            aria-live="polite"
            className="flex items-center gap-2 rounded-full bg-raised px-2.5 py-0.5 text-xs text-muted"
          >
            {ctl.running && <Spinner className="h-3 w-3" />}
            {failureLabel(ctl)}
          </span>
        </div>
        <div className="mt-2">
          <Stepper stage={stage} />
        </div>
      </header>

      {locked && !ctl.running && (
        <p role="status" className="mx-4 mt-3 rounded-md border border-line bg-raised px-3 py-2 text-sm text-muted">
          Outra operação do repositório está em andamento. As ações de escrita ficam indisponíveis até ela terminar.
        </p>
      )}

      {commitStage ? (
        <CommitStage ctl={ctl} locked={locked} onRefresh={onRefresh} />
      ) : (
        <div className="min-h-0 flex-1 overflow-auto p-4">
          <div className="mx-auto max-w-4xl space-y-4">
            {ctl.pickError && <ErrorBanner title="Não foi possível abrir o seletor" error={ctl.pickError} />}
            {flow.issue && (
              <ErrorBanner
                title={issueTitle(flow.issue.kind)}
                error={flow.issue.error}
                onDismiss={ctl.dismissIssue}
                actions={
                  flow.issue.unconfirmed ? (
                    <span className="text-xs text-modified">
                      Resultado não confirmado: o servidor pode ter concluído a operação. Verifique o remoto antes de repetir.
                    </span>
                  ) : undefined
                }
              />
            )}

            {SELECTION_STAGES.includes(stage) && (
              <SelectionCard ctl={ctl} repoPath={repoPath} branches={branches} locked={locked} />
            )}
            {stage === "planReview" && (
              <PlanPanel ctl={ctl} existingBranches={branches?.branches.map((b) => b.name) ?? []} />
            )}

            {stage === "creatingBranch" && (
              <Working title="Criando a branch de trabalho…">
                Validando o repositório e criando <span className="font-mono">{flow.workBranch}</span> a partir de{" "}
                <span className="font-mono">{flow.baseBranch}</span>. Nenhum arquivo foi gravado ainda.
              </Working>
            )}
            {stage === "importing" && (
              <Working title="Importando arquivos…">
                Gravando só os arquivos do plano confirmado em <span className="font-mono">{flow.workBranch}</span>. Não
                feche o aplicativo; o resultado real aparece ao terminar.
              </Working>
            )}
            {stage === "rollingBack" && (
              <Working title="Desfazendo a importação…">
                Restaurando os arquivos substituídos e removendo os novos que ainda estão como foram importados.
              </Working>
            )}

            {stage === "partial" && <PartialCard ctl={ctl} locked={locked} />}
            {stage === "failed" && <FailedCard ctl={ctl} locked={locked} />}
            {stage === "rolledBack" && <RolledBackCard ctl={ctl} />}
            {(stage === "committed" || stage === "pushing") && (
              <PublishCard ctl={ctl} locked={locked} syncStatus={syncStatus} />
            )}
            {stage === "done" && <DoneCard ctl={ctl} />}

            <OperationLog log={flow.log} />
          </div>
        </div>
      )}
    </div>
  );
}

function failureLabel(ctl: ZipImportController): string {
  const { flow } = ctl;
  if (flow.stage === "failed") {
    return flow.failure?.manual ? "Falha que exige intervenção manual" : "Falha recuperável";
  }
  return STAGE_LABELS[flow.stage];
}

function issueTitle(kind: "push" | "rollback" | "finish" | "apply"): string {
  switch (kind) {
    case "push":
      return "O envio ao remoto não foi concluído";
    case "rollback":
      return "Não foi possível desfazer a importação";
    case "finish":
      return "Não foi possível encerrar a operação";
    case "apply":
      return "A importação não foi concluída";
  }
}

// ---------------------------------------------------------------------------
// Seleção e inspeção
// ---------------------------------------------------------------------------

function SelectionCard({
  ctl,
  repoPath,
  branches,
  locked,
}: {
  ctl: ZipImportController;
  repoPath: string;
  branches: BranchList | null;
  locked: boolean;
}) {
  const { flow } = ctl;
  const inspecting = flow.stage === "inspecting";
  const busy = ctl.running || locked || ctl.picking;
  const names = branches?.branches.map((b) => b.name) ?? [];
  const base = flow.baseBranch ?? ctl.currentBranch ?? "";
  const baseMismatch = ctl.currentBranch !== null && base !== "" && ctl.currentBranch !== base;
  const directories = flow.plan?.layout.directories ?? [];

  return (
    <Card title="1. Origem e destino">
      <dl className="grid grid-cols-[8.5rem_1fr] items-center gap-x-4 gap-y-3">
        <dt className="text-muted">Arquivo ZIP</dt>
        <dd className="flex flex-wrap items-center gap-2">
          {flow.zipPath ? (
            <span className="font-mono text-xs break-all">{flow.zipPath}</span>
          ) : (
            <span className="text-muted">Nenhum arquivo selecionado.</span>
          )}
          <Button variant="secondary" disabled={busy} onClick={() => void ctl.chooseZip()}>
            {flow.zipPath ? "Trocar ZIP…" : "Escolher ZIP…"}
          </Button>
        </dd>

        <dt className="text-muted">Repositório de destino</dt>
        <dd>
          <span className="font-mono text-xs break-all">{repoPath}</span>
          <span className="mt-0.5 block text-xs text-muted">
            Para importar em outro repositório, abra-o pelo botão “Abrir repositório”.
          </span>
        </dd>

        <dt className="text-muted">
          <label htmlFor="import-base">Branch-base</label>
        </dt>
        <dd>
          <select
            id="import-base"
            value={base}
            onChange={(event) => ctl.changeBase(event.target.value)}
            disabled={busy || names.length === 0}
            className="rounded-md border border-line bg-canvas px-2 py-1.5 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            {names.length === 0 && <option value={base}>{base || "—"}</option>}
            {names.map((name) => (
              <option key={name} value={name}>
                {name}
                {name === ctl.currentBranch ? " (atual)" : ""}
              </option>
            ))}
          </select>
          {baseMismatch && (
            <span role="alert" className="mt-1 block text-xs text-modified">
              A branch ativa é “{ctl.currentBranch}”. Para importar sobre “{base}”, troque para ela primeiro na aba
              Branches (o plano é calculado sobre os arquivos da base ativa).
            </span>
          )}
        </dd>

        {flow.plan && (
          <>
            <dt className="text-muted">
              <label htmlFor="import-prefix">Pasta do pacote</label>
            </dt>
            <dd>
              <select
                id="import-prefix"
                value={flow.sourcePrefix}
                onChange={(event) => ctl.changePrefix(event.target.value)}
                disabled={busy}
                className="max-w-full rounded-md border border-line bg-canvas px-2 py-1.5 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent"
              >
                <option value="">Raiz do ZIP (usar os caminhos como estão)</option>
                {directories.map((dir) => (
                  <option key={dir} value={dir}>
                    {dir}/
                  </option>
                ))}
              </select>
              <span className="mt-1 block text-xs text-muted">
                Escolha a pasta do ZIP que corresponde à raiz do projeto. Nenhum nível é removido sem a sua escolha.
              </span>
            </dd>
          </>
        )}
      </dl>

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button
          variant="primary"
          disabled={busy || !flow.zipPath || !base || baseMismatch}
          onClick={ctl.inspect}
        >
          {inspecting ? <Spinner /> : null}
          {flow.plan ? "Inspecionar novamente" : "Inspecionar ZIP"}
        </Button>
        {inspecting && (
          <Button variant="secondary" onClick={ctl.cancel}>
            Cancelar inspeção
          </Button>
        )}
        <span className="text-xs text-muted">
          A inspeção só lê o ZIP: nada é extraído nem gravado no repositório.
        </span>
      </div>

      {flow.stage === "zipInvalid" && flow.inspectError && (
        <ErrorBanner className="mt-4" title="Não foi possível usar este ZIP" error={flow.inspectError} />
      )}
      {flow.stage === "cancelled" && (
        <p role="status" className="mt-4 rounded-md border border-line bg-raised px-3 py-2 text-muted">
          Inspeção cancelada. Nada foi alterado no repositório. Escolha um ZIP para recomeçar.
          <Button variant="secondary" className="ml-3" onClick={ctl.restart}>
            Voltar à seleção
          </Button>
        </p>
      )}
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Falhas
// ---------------------------------------------------------------------------

function PartialCard({ ctl, locked }: { ctl: ZipImportController; locked: boolean }) {
  const { flow } = ctl;
  const result = flow.result;
  return (
    <Card title="Importação parcialmente concluída" tone="danger">
      <p>
        A gravação falhou no meio. O repositório está na branch <span className="font-mono">{flow.workBranch}</span> com
        parte dos arquivos aplicada. <strong>Nenhum commit foi criado e nenhuma etapa seguinte foi executada.</strong>
      </p>
      {result?.failure ? (
        <p className="mt-2">
          Falhou em <span className="font-mono">{result.failure.path}</span>: {result.failure.message}
        </p>
      ) : (
        <p className="mt-2">A operação foi interrompida antes de terminar (por exemplo, o aplicativo foi fechado).</p>
      )}
      {result && (
        <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
          <dt className="text-muted">Gravados</dt>
          <dd>{result.applied.length}</dd>
          <dt className="text-muted">Não tentados</dt>
          <dd>{result.notAttempted.length}</dd>
        </dl>
      )}
      {result && <PathList paths={result.applied.map((f) => f.path)} />}
      <p className="mt-3 text-muted">
        {result?.recovery ??
          "Desfaça a importação para voltar ao estado anterior. Arquivos que você editou depois são preservados."}
      </p>
      <div className="mt-3">
        <ConfirmButton
          label="Desfazer a importação"
          confirmLabel="Sim, desfazer"
          disabled={ctl.running || locked}
          explanation={
            <>
              Os arquivos <strong>substituídos</strong> voltam ao conteúdo original (a partir do backup) e os{" "}
              <strong>novos</strong> são removidos, desde que ainda estejam exatamente como foram importados. O que você
              tiver editado depois é preservado e listado. A branch de trabalho continua existindo.
            </>
          }
          onConfirm={() => void ctl.rollback()}
        />
      </div>
    </Card>
  );
}

function FailedCard({ ctl, locked }: { ctl: ZipImportController; locked: boolean }) {
  const failure = ctl.flow.failure;
  if (!failure) return null;
  const afterBranch = failure.at === "importing";
  return (
    <Card title={failure.manual ? "Falha que exige intervenção manual" : "Falha recuperável"} tone="danger">
      <ErrorBanner title={`Falhou em: ${STAGE_LABELS[failure.at]}`} error={failure.error} />
      <div className="mt-3 grid gap-4 sm:grid-cols-2">
        <div>
          <h4 className="font-medium">Já concluído</h4>
          <ul className="mt-1 list-disc space-y-0.5 pl-5 text-muted">
            {failure.completed.map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ul>
        </div>
        <div>
          <h4 className="font-medium">Verifique antes de continuar</h4>
          <ul className="mt-1 list-disc space-y-0.5 pl-5 text-muted">
            {failure.advice.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        </div>
      </div>
      {ctl.recoverError && <ErrorBanner className="mt-3" title="Não foi possível voltar à branch-base" error={ctl.recoverError} />}
      <div className="mt-4 flex flex-wrap gap-2">
        {afterBranch && (
          <Button
            variant="primary"
            disabled={ctl.running || locked}
            onClick={() => void ctl.recover(true)}
          >
            Voltar para “{ctl.flow.baseBranch}” e recomeçar
          </Button>
        )}
        <Button variant={afterBranch ? "secondary" : "primary"} disabled={ctl.running || locked} onClick={() => void ctl.recover(false)}>
          Recomeçar (revalida o repositório)
        </Button>
      </div>
    </Card>
  );
}

function RolledBackCard({ ctl }: { ctl: ZipImportController }) {
  const r = ctl.flow.rollback;
  return (
    <Card title="Importação desfeita" tone="success">
      <p>
        Os arquivos da importação foram revertidos. A branch <span className="font-mono">{ctl.flow.workBranch}</span>{" "}
        continua existindo, sem commits; você pode apagá-la depois pela aba Branches ou pelo Git.
      </p>
      {r && (
        <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
          <dt className="text-muted">Restaurados</dt>
          <dd>{r.restored.length}</dd>
          <dt className="text-muted">Removidos</dt>
          <dd>{r.removed.length}</dd>
          <dt className="text-muted">Preservados (editados depois)</dt>
          <dd>{r.keptModified.length}</dd>
        </dl>
      )}
      {r && r.keptModified.length > 0 && (
        <div className="mt-2">
          <p className="text-modified">Estes arquivos não foram tocados, pois mudaram depois da importação:</p>
          <PathList paths={r.keptModified} />
        </div>
      )}
      {r && r.problems.length > 0 && (
        <ul className="mt-2 list-disc pl-5 text-removed">
          {r.problems.map((p) => (
            <li key={p}>{p}</li>
          ))}
        </ul>
      )}
      <div className="mt-3">
        <Button variant="primary" onClick={ctl.restart}>
          Nova importação
        </Button>
      </div>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Commit (reaproveita seleção, revisão, diff e confirmação do fluxo de commit)
// ---------------------------------------------------------------------------

function CommitStage({
  ctl,
  locked,
  onRefresh,
}: {
  ctl: ZipImportController;
  locked: boolean;
  onRefresh: () => void;
}) {
  const { commit, flow } = ctl;
  const reviewing = commit.step === "review";
  const reviewFile = commit.review.data?.files.find((f) => f.path === commit.focusPath);
  const reviewBanner = reviewFile
    ? reviewFile.untracked
      ? "Arquivo novo: ele será adicionado ao Git por este commit."
      : reviewFile.hasStaged && reviewFile.hasUnstaged
        ? "Este arquivo tem alterações preparadas e também não preparadas. O commit registra o conteúdo atual do arquivo, com as duas, e é isso que o diff abaixo mostra."
        : undefined
    : undefined;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="space-y-3 border-b border-line px-4 py-3 text-sm">
        <p>
          <strong>{flow.importedPaths.length}</strong> arquivo(s) foram gravados em{" "}
          <span className="font-mono">{flow.workBranch}</span>. Revise o diff real, escolha quais entram no commit e
          confirme. <strong>Nada será enviado ao remoto nesta etapa.</strong>
        </p>
        {ctl.foreign.length > 0 && (
          <p className="text-muted">
            Há {ctl.foreign.length} outra(s) alteração(ões) no repositório fora desta importação; elas não entram neste
            commit nem aparecem na lista.
          </p>
        )}
        {flow.issue && (
          <ErrorBanner
            title={issueTitle(flow.issue.kind)}
            error={flow.issue.error}
            onDismiss={ctl.dismissIssue}
          />
        )}
        <div>
          <ConfirmButton
            label="Desfazer a importação"
            confirmLabel="Sim, desfazer"
            disabled={ctl.running || locked}
            explanation={
              <>
                Restaura os arquivos substituídos e remove os novos que ainda estão como foram importados. Como ainda não
                há commit, nada do histórico muda. Arquivos que você editou depois são preservados.
              </>
            }
            onConfirm={() => void ctl.rollback()}
          />
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        {reviewing ? (
          <ReviewFileList
            review={commit.review}
            focusPath={commit.focusPath}
            onFocus={commit.focus}
            onBack={commit.backToSelection}
            disabled={commit.committing}
          />
        ) : (
          <ChangesPanel
            status={ctl.restrictedStatus}
            selection={ctl.selection}
            onSelect={ctl.select}
            onRefresh={onRefresh}
            commit={{
              selected: commit.selected,
              onToggle: commit.toggle,
              onSetMany: commit.setMany,
              blocker: commit.blocker,
              locked: locked || ctl.running,
              onReview: commit.startReview,
            }}
          />
        )}
        <div className="flex min-w-0 flex-1 flex-col">
          <DiffViewer
            selection={reviewing ? commit.reviewSelection : ctl.selection}
            diff={ctl.diff}
            onRetry={ctl.retryDiff}
            banner={reviewing ? reviewBanner : undefined}
          />
          {reviewing && (
            <CommitForm
              review={commit.review}
              message={commit.message}
              messageProblem={commit.messageProblem}
              confirmed={commit.confirmed}
              committing={commit.committing}
              canSubmit={commit.canSubmit}
              notice={commit.notice}
              error={commit.commitError}
              onMessage={commit.changeMessage}
              onConfirm={commit.setConfirmed}
              onSubmit={() => void commit.submit()}
              onCancel={commit.backToSelection}
              onDismissError={commit.dismissCommitError}
            />
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Push (confirmação separada; reaproveita o envio da Fase 3)
// ---------------------------------------------------------------------------

function PublishCard({
  ctl,
  locked,
  syncStatus,
}: {
  ctl: ZipImportController;
  locked: boolean;
  syncStatus: SyncStatus | null;
}) {
  const { flow, pushPlan } = ctl;
  const pushing = flow.stage === "pushing";
  const target = pushPlan.target;
  const blocked = ctl.running || locked;

  return (
    <div className="space-y-4">
      <Card tone="success">
        <p className="flex items-center gap-2 font-medium">
          <CheckCircleIcon className="shrink-0 text-added" />
          <span>
            Commit <span className="font-mono">{flow.commit?.shortOid ?? flow.commitOid?.slice(0, 7)}</span> criado na
            branch <span className="font-mono">{flow.workBranch}</span>
          </span>
        </p>
        {flow.commit && (
          <p className="mt-1 truncate text-muted" title={flow.commit.subject}>
            {flow.commit.subject}
          </p>
        )}
        <p className="mt-1 font-mono text-xs break-all text-faint">{flow.commitOid}</p>
        {flow.commit?.summary && <p className="mt-1 text-xs text-muted">{flow.commit.summary}</p>}
      </Card>

      <Card title="Publicar no remoto (opcional)">
        {pushing ? (
          <div role="status" className="flex items-center gap-2 text-muted">
            <Spinner className="text-accent" /> Enviando ao remoto… O resultado real será exibido ao terminar.
          </div>
        ) : (
          <>
            <p className="text-muted">
              O commit já existe localmente. Enviar é uma etapa separada e usa o push da Fase 3: só a branch de trabalho,
              para o destino abaixo, sem força e sem criar ou alterar o upstream.
            </p>
            <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
              <dt className="text-muted">Branch local</dt>
              <dd className="font-mono">{flow.workBranch}</dd>
              <dt className="text-muted">Destino</dt>
              <dd className="font-mono">{target ? `${target.remote}/${target.remoteBranch}` : "— (não definido)"}</dd>
              <dt className="text-muted">Commit</dt>
              <dd className="font-mono">{flow.commitOid ? flow.commitOid.slice(0, 12) : "—"}</dd>
              {syncStatus?.ahead != null && (
                <>
                  <dt className="text-muted">Commits a enviar</dt>
                  <dd>{syncStatus.ahead} (segundo a última atualização do remoto)</dd>
                </>
              )}
            </dl>

            {pushPlan.problems.length > 0 && (
              <div role="alert" className="mt-3 rounded-md border border-modified/40 bg-modified/10 p-3">
                <p className="font-medium text-modified">Não é possível enviar agora</p>
                <ul className="mt-1 list-disc space-y-1 pl-5">
                  {pushPlan.problems.map((p) => (
                    <li key={p}>{p}</li>
                  ))}
                </ul>
                <p className="mt-2 text-xs text-muted">
                  O aplicativo não inventa destinos nem define upstream sozinho. Se precisar, configure o upstream fora do
                  aplicativo e use a aba Sincronização; o commit continua salvo localmente.
                </p>
              </div>
            )}

            {target && pushPlan.problems.length === 0 && (
              <div className="mt-3 space-y-2">
                {pushPlan.protectedDestination && (
                  <Check checked={ctl.protectedAck} onChange={ctl.setProtectedAck} disabled={blocked}>
                    <span className="text-modified">
                      O destino é <span className="font-mono">{target.remoteBranch}</span>, uma branch protegida.
                    </span>{" "}
                    Confirmo que quero enviar para ela.
                  </Check>
                )}
                <Check checked={ctl.pushAck} onChange={ctl.setPushAck} disabled={blocked}>
                  Confirmo o envio de <span className="font-mono">{flow.workBranch}</span> para{" "}
                  <span className="font-mono">
                    {target.remote}/{target.remoteBranch}
                  </span>
                  .
                </Check>
              </div>
            )}

            <div className="mt-4 flex flex-wrap gap-2">
              <Button variant="primary" disabled={blocked || !ctl.canPush} onClick={() => void ctl.push()}>
                Enviar ao remoto
              </Button>
              <Button variant="secondary" disabled={blocked} onClick={() => void ctl.finishWithoutPush()}>
                Concluir sem enviar
              </Button>
            </div>
          </>
        )}
      </Card>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Relatório final
// ---------------------------------------------------------------------------

function DoneCard({ ctl }: { ctl: ZipImportController }) {
  const { flow } = ctl;
  const rows: [string, ReactNode][] = [
    ["Pacote", <span key="z" className="font-mono text-xs break-all">{flow.zipPath}</span>],
    ["Branch de trabalho", <span key="b" className="font-mono">{flow.workBranch}</span>],
    ["Arquivos importados", flow.importedPaths.length],
    [
      "Commit",
      <span key="c" className="font-mono">
        {flow.commitOid ? flow.commitOid.slice(0, 12) : "—"}
      </span>,
    ],
    [
      "Envio ao remoto",
      flow.push
        ? `Enviado para ${flow.push.target.remote}/${flow.push.target.remoteBranch}`
        : "Não enviado (o commit permanece só no repositório local)",
    ],
  ];
  return (
    <Card title="Importação concluída" tone="success">
      <dl className="grid grid-cols-[10rem_1fr] gap-x-4 gap-y-1.5">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted">{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {isEngaged(flow.stage) ? null : (
        <div className="mt-4">
          <Button variant="primary" onClick={ctl.restart}>
            Nova importação
          </Button>
        </div>
      )}
    </Card>
  );
}
