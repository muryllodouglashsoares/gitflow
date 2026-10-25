import type { GitState, OpenFailure } from "../../hooks/useRepositoryWorkspace";
import { isGitInstallationError } from "../../lib/errors";
import { Button } from "../common/Button";
import { ErrorBanner } from "../common/ErrorBanner";
import { FolderOpenIcon, LogoMark } from "../common/icons";
import { Spinner } from "../common/Spinner";

interface WelcomeScreenProps {
  git: GitState;
  busy: boolean;
  failure: OpenFailure | null;
  onOpenRepository: () => void;
  onRecheckGit: () => void;
  onDismissFailure: () => void;
}

export function WelcomeScreen({
  git,
  busy,
  failure,
  onOpenRepository,
  onRecheckGit,
  onDismissFailure,
}: WelcomeScreenProps) {
  const gitMissing = !git.checking && git.info !== null && !git.info.available;

  return (
    <div className="flex h-full items-center justify-center p-8">
      <div className="w-full max-w-md">
        <div className="mb-5 flex items-center gap-3">
          <LogoMark width={36} height={36} />
          <div>
            <h1 className="text-lg font-semibold">Abra um repositório</h1>
            <p className="text-sm text-muted">Veja o estado e as diferenças sem modificar nada.</p>
          </div>
        </div>

        {gitMissing && git.info?.error && (
          <ErrorBanner
            className="mb-4"
            title="Git não disponível"
            error={git.info.error}
            actions={<Button onClick={onRecheckGit}>Verificar novamente</Button>}
          />
        )}

        {failure && !(gitMissing && isGitInstallationError(failure.error)) && (
          <ErrorBanner
            className="mb-4"
            title={failure.path ? "Não foi possível abrir esta pasta" : "Não foi possível abrir o seletor de pastas"}
            error={failure.error}
            onDismiss={onDismissFailure}
          />
        )}
        {failure?.path && (
          <p className="mb-4 -mt-2 truncate text-xs text-faint" title={failure.path}>
            Pasta: {failure.path}
          </p>
        )}

        <Button variant="primary" className="w-full py-2" onClick={onOpenRepository} disabled={busy || git.checking}>
          {busy || git.checking ? <Spinner /> : <FolderOpenIcon />}
          {busy ? "Abrindo…" : git.checking ? "Verificando Git…" : "Selecionar pasta do repositório"}
        </Button>

        <p className="mt-4 text-xs text-muted">
          O GitFlow Desktop usa o Git instalado no seu computador. Nesta versão ele apenas lê informações:
          nenhum arquivo ou histórico é alterado.
        </p>
      </div>
    </div>
  );
}
