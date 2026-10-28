import { useState } from "react";
import { BranchesView } from "./components/branches/BranchesView";
import { ChangesPanel } from "./components/changes/ChangesPanel";
import { ErrorBanner } from "./components/common/ErrorBanner";
import { CommitForm } from "./components/commit/CommitForm";
import { CommitResultBanner } from "./components/commit/CommitResultBanner";
import { ReviewFileList } from "./components/commit/ReviewFileList";
import { DiffViewer } from "./components/diff/DiffViewer";
import { ImportView } from "./components/import/ImportView";
import { SyncView } from "./components/sync/SyncView";
import { RepositoryHeader } from "./components/layout/RepositoryHeader";
import { Sidebar, type NavId } from "./components/layout/Sidebar";
import { WelcomeScreen } from "./components/welcome/WelcomeScreen";
import { useBranchActions } from "./hooks/useBranchActions";
import { useBranches } from "./hooks/useBranches";
import { useCommitFlow } from "./hooks/useCommitFlow";
import { useFileDiff } from "./hooks/useFileDiff";
import { useRepositoryWorkspace } from "./hooks/useRepositoryWorkspace";
import { useSync } from "./hooks/useSync";
import { useZipImport } from "./hooks/useZipImport";

export default function App() {
  const workspace = useRepositoryWorkspace();
  const { repo, status, selection } = workspace;
  const [nav, setNav] = useState<NavId>("changes");
  const repoPath = repo?.path ?? null;

  const commit = useCommitFlow({
    repoPath,
    status,
    runExclusive: workspace.runExclusive,
    refresh: workspace.refresh,
  });
  const reviewing = commit.step === "review";

  // Um único visualizador de diff: mostra o arquivo escolhido na lista de alterações ou, durante a
  // revisão, o arquivo do commit (conteúdo atual × último commit). Ambos são relidos quando o
  // estado do repositório muda, para nunca exibir um diff antigo.
  const { diff, retry } = useFileDiff(
    repoPath,
    reviewing ? commit.reviewSelection : selection,
    reviewing ? commit.reviewVersion : status.updatedAt,
  );

  // A lista de branches também alimenta a importação de ZIP (base e nome da branch de trabalho).
  const branches = useBranches(repoPath, status.updatedAt, nav === "branches" || nav === "import");
  const branchActions = useBranchActions(repoPath, workspace.runExclusive, workspace.refresh);
  // Fase 3: diagnóstico de sincronização (sem rede) sempre disponível; fetch/pull/push sob demanda.
  const sync = useSync(repoPath, status.updatedAt, workspace.runExclusive, workspace.refresh);

  // Fase 4: importação de ZIP. O estado do fluxo vive aqui para sobreviver à troca de aba.
  const zipImport = useZipImport({
    repoPath,
    status,
    branchList: branches.branches.data,
    syncStatus: sync.diagnosis.data,
    runExclusive: workspace.runExclusive,
    refresh: workspace.refresh,
  });

  const reviewFile = commit.review.data?.files.find((f) => f.path === commit.focusPath);
  const reviewBanner = reviewFile
    ? reviewFile.untracked
      ? "Arquivo novo: ele será adicionado ao Git por este commit."
      : reviewFile.hasStaged && reviewFile.hasUnstaged
        ? "Este arquivo tem alterações preparadas e também não preparadas. O commit registra o conteúdo atual do arquivo, com as duas, e é isso que o diff abaixo mostra."
        : undefined
    : undefined;

  return (
    <div className="flex h-full">
      <Sidebar
        active={nav}
        git={workspace.git}
        busy={workspace.busy}
        openDisabled={workspace.mutating !== null}
        onOpenRepository={() => void workspace.chooseFolder()}
        onNavigate={setNav}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        {repo ? (
          <>
            {/* Falha ao abrir outra pasta: o repositório atual continua exibido. */}
            {workspace.openFailure && (
              <ErrorBanner
                className="m-3 mb-0"
                title="Não foi possível abrir a pasta selecionada"
                error={workspace.openFailure.error}
                onDismiss={workspace.dismissOpenFailure}
              />
            )}
            <RepositoryHeader
              repo={repo}
              status={status}
              onRefresh={workspace.refresh}
              onOpenSync={() => setNav("sync")}
            />

            {nav === "import" ? (
              <ImportView
                repoPath={repo.path}
                ctl={zipImport}
                branches={branches.branches.data}
                locked={workspace.mutating !== null && !zipImport.running}
                syncStatus={sync.diagnosis.data}
                onRefresh={workspace.refresh}
              />
            ) : nav === "sync" ? (
              <SyncView status={status.data} sync={sync} locked={workspace.mutating !== null} />
            ) : nav === "branches" ? (
              <BranchesView
                repoPath={repo.path}
                status={status.data}
                branches={branches.branches}
                pending={branchActions.pending}
                outcome={branchActions.outcome}
                error={branchActions.error}
                locked={workspace.mutating !== null}
                onReload={branches.reload}
                onCreate={branchActions.createBranch}
                onSwitch={branchActions.switchTo}
                onDismiss={branchActions.dismiss}
              />
            ) : (
              <>
                {commit.result && (
                  <CommitResultBanner result={commit.result} onDismiss={commit.dismissResult} />
                )}
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
                      status={status}
                      selection={selection}
                      onSelect={workspace.select}
                      onRefresh={workspace.refresh}
                      commit={{
                        selected: commit.selected,
                        onToggle: commit.toggle,
                        onSetMany: commit.setMany,
                        blocker: commit.blocker,
                        locked: workspace.mutating !== null,
                        onReview: commit.startReview,
                      }}
                    />
                  )}
                  <div className="flex min-w-0 flex-1 flex-col">
                    <DiffViewer
                      selection={reviewing ? commit.reviewSelection : selection}
                      diff={diff}
                      onRetry={retry}
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
              </>
            )}
          </>
        ) : (
          <WelcomeScreen
            git={workspace.git}
            busy={workspace.busy}
            failure={workspace.openFailure}
            onOpenRepository={() => void workspace.chooseFolder()}
            onRecheckGit={() => void workspace.checkGit()}
            onDismissFailure={workspace.dismissOpenFailure}
          />
        )}
      </div>
    </div>
  );
}
