import { useState } from "react";
import { ErrorBanner } from "./components/common/ErrorBanner";
import { ChangesPanel } from "./components/changes/ChangesPanel";
import { DiffViewer } from "./components/diff/DiffViewer";
import { RepositoryHeader } from "./components/layout/RepositoryHeader";
import { Sidebar, type NavId } from "./components/layout/Sidebar";
import { WelcomeScreen } from "./components/welcome/WelcomeScreen";
import { useFileDiff } from "./hooks/useFileDiff";
import { useRepositoryWorkspace } from "./hooks/useRepositoryWorkspace";

export default function App() {
  const workspace = useRepositoryWorkspace();
  const { repo, status, selection } = workspace;
  const [nav, setNav] = useState<NavId>("changes");

  const { diff, retry } = useFileDiff(repo?.path ?? null, selection, status.updatedAt);

  return (
    <div className="flex h-full">
      <Sidebar
        active={nav}
        git={workspace.git}
        busy={workspace.busy}
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
            <RepositoryHeader repo={repo} status={status} onRefresh={workspace.refresh} />
            {/* Fases futuras: renderizar outras telas conforme `nav`. */}
            <div className="flex min-h-0 flex-1">
              <ChangesPanel
                status={status}
                selection={selection}
                onSelect={workspace.select}
                onRefresh={workspace.refresh}
              />
              <DiffViewer selection={selection} diff={diff} onRetry={retry} />
            </div>
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
