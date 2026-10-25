import { useEffect, useMemo, useState } from "react";
import type { BranchesState } from "../../hooks/useBranches";
import type { BranchOutcome, BranchPending } from "../../hooks/useBranchActions";
import { quickBranchNameProblem, sortBranches, suggestBase, switchBlock, untrackedCount } from "../../lib/branches";
import { gitService, errorFrom } from "../../services/gitService";
import type { BranchEntry, GitError, RepositoryStatus } from "../../types/git";
import { Button } from "../common/Button";
import { EmptyState } from "../common/EmptyState";
import { ErrorBanner } from "../common/ErrorBanner";
import { BranchIcon, CheckCircleIcon, CloseIcon } from "../common/icons";
import { Spinner } from "../common/Spinner";

interface BranchesViewProps {
  repoPath: string;
  status: RepositoryStatus | null;
  branches: BranchesState;
  pending: BranchPending;
  outcome: BranchOutcome | null;
  error: GitError | null;
  /** Há alguma operação de escrita em andamento (qualquer tipo). */
  locked: boolean;
  onReload: () => void;
  onCreate: (name: string, base: string) => Promise<boolean>;
  onSwitch: (name: string) => Promise<boolean>;
  onDismiss: () => void;
}

export function BranchesView(props: BranchesViewProps) {
  const { branches, status, pending, outcome, error, locked } = props;
  const [creating, setCreating] = useState(false);
  const [confirmSwitch, setConfirmSwitch] = useState<string | null>(null);

  const list = branches.data;
  const sorted = useMemo(() => (list ? sortBranches(list.branches) : []), [list]);
  const block = switchBlock(status);
  const untracked = untrackedCount(status);

  return (
    <main className="min-w-0 flex-1 overflow-y-auto bg-canvas" aria-label="Branches locais">
      <div className="mx-auto max-w-3xl p-5">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-sm font-semibold">Branches locais</h2>
          <div className="flex gap-2">
            <Button onClick={props.onReload} disabled={branches.loading || locked}>
              {branches.loading ? <Spinner /> : null}Atualizar lista
            </Button>
            <Button variant="primary" onClick={() => setCreating(true)} disabled={creating || locked || !list || list.unborn}>
              Nova branch…
            </Button>
          </div>
        </div>

        {outcome && (
          <div role="status" className="mb-4 flex items-start gap-2.5 rounded-md border border-added/40 bg-added/10 p-3 text-sm">
            <CheckCircleIcon className="mt-0.5 shrink-0 text-added" />
            <div className="min-w-0 flex-1">
              {outcome.kind === "created" ? (
                <>
                  <p className="font-medium">
                    Branch <span className="font-mono">{outcome.result.name}</span> criada a partir de{" "}
                    <span className="font-mono">{outcome.result.base}</span>.
                  </p>
                  <p className="mt-0.5 text-muted">
                    Você continua na branch atual; a nova branch ainda não foi ativada. Use “Trocar” para ir até ela.
                  </p>
                </>
              ) : (
                <p className="font-medium">
                  Agora você está na branch <span className="font-mono">{outcome.result.current}</span>
                  {outcome.result.previous ? (
                    <>
                      {" "}
                      (antes: <span className="font-mono">{outcome.result.previous}</span>)
                    </>
                  ) : null}
                  .
                </p>
              )}
            </div>
            <button type="button" onClick={props.onDismiss} aria-label="Fechar mensagem" className="rounded p-1 text-muted hover:bg-raised hover:text-fg">
              <CloseIcon />
            </button>
          </div>
        )}

        {error && <ErrorBanner className="mb-4" title="A operação não foi concluída" error={error} onDismiss={props.onDismiss} />}

        {creating && list && (
          <CreateBranchForm
            repoPath={props.repoPath}
            branches={sorted}
            defaultBase={suggestBase(list)}
            busy={pending?.kind === "create"}
            locked={locked}
            onCancel={() => setCreating(false)}
            onSubmit={async (name, base) => {
              const ok = await props.onCreate(name, base);
              if (ok) setCreating(false);
            }}
          />
        )}

        {block && list && list.branches.length > 1 && (
          <div role="note" className="mb-4 rounded-md border border-modified/40 bg-modified/10 p-3 text-sm text-modified">
            <p className="font-medium">Troca de branch bloqueada por segurança</p>
            <p className="mt-0.5">{block.reason}</p>
            {block.files.length > 0 && (
              <ul className="mt-1.5 max-h-28 overflow-auto font-mono text-xs break-all">
                {block.files.slice(0, 12).map((path) => (
                  <li key={path}>{path}</li>
                ))}
                {block.files.length > 12 && <li>… e mais {block.files.length - 12}</li>}
              </ul>
            )}
          </div>
        )}

        {branches.error ? (
          <ErrorBanner
            title="Não foi possível ler as branches"
            error={branches.error}
            actions={<Button onClick={props.onReload}>Tentar novamente</Button>}
          />
        ) : !list ? (
          <EmptyState icon={<Spinner className="h-5 w-5" />} title="Carregando branches…" />
        ) : (
          <>
            {list.detached && (
              <p className="mb-3 rounded-md border border-line bg-raised/50 p-3 text-sm text-muted">
                O HEAD está desanexado (fora de qualquer branch).
              </p>
            )}
            {list.unborn && (
              <p className="mb-3 rounded-md border border-line bg-raised/50 p-3 text-sm text-muted">
                A branch <span className="font-mono text-fg">{list.current}</span> ainda não tem commits. Crie o
                primeiro commit antes de criar outras branches.
              </p>
            )}
            {sorted.length === 0 ? (
              <EmptyState title="Nenhuma branch local com commits" description="As branches aparecem aqui depois do primeiro commit." />
            ) : (
              <ul className="divide-y divide-line rounded-md border border-line bg-surface">
                {sorted.map((branch) => (
                  <BranchRow
                    key={branch.name}
                    branch={branch}
                    blocked={block !== null}
                    locked={locked}
                    switching={pending?.kind === "switch" && pending.name === branch.name}
                    confirming={confirmSwitch === branch.name}
                    untracked={untracked}
                    onAsk={() => setConfirmSwitch(branch.name)}
                    onCancel={() => setConfirmSwitch(null)}
                    onConfirm={async () => {
                      await props.onSwitch(branch.name);
                      setConfirmSwitch(null);
                    }}
                  />
                ))}
              </ul>
            )}
            <p className="mt-3 text-xs text-faint">
              Somente branches locais são listadas; referências remotas não aparecem aqui.
            </p>
          </>
        )}
      </div>
    </main>
  );
}

function BranchRow(props: {
  branch: BranchEntry;
  blocked: boolean;
  locked: boolean;
  switching: boolean;
  confirming: boolean;
  untracked: number;
  onAsk: () => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { branch } = props;
  return (
    <li className="px-4 py-2.5">
      <div className="flex items-center gap-3">
        <BranchIcon className={branch.current ? "text-accent" : "text-faint"} />
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-2 text-sm">
            <span className="truncate font-mono">{branch.name}</span>
            {branch.current && (
              <span className="shrink-0 rounded-full border border-accent/40 bg-accent/10 px-2 py-0.5 text-[11px] text-accent">
                atual
              </span>
            )}
          </p>
          <p className="truncate text-xs text-muted">
            <span className="font-mono">{branch.oid.slice(0, 7)}</span>
            {branch.subject ? ` · ${branch.subject}` : ""}
            {branch.upstream ? ` · ${branch.upstream}` : ""}
          </p>
        </div>
        {!branch.current && !props.confirming && (
          <Button
            onClick={props.onAsk}
            disabled={props.blocked || props.locked}
            title={props.blocked ? "Troca bloqueada: veja o aviso acima." : undefined}
          >
            Trocar
          </Button>
        )}
      </div>
      {props.confirming && (
        <div className="mt-2 rounded-md border border-line bg-raised/50 p-2.5 text-sm">
          <p>
            Trocar para <span className="font-mono">{branch.name}</span>? Seus arquivos não serão apagados nem
            descartados.
          </p>
          {props.untracked > 0 && (
            <p className="mt-1 text-xs text-muted">
              Há {props.untracked} {props.untracked === 1 ? "arquivo não rastreado" : "arquivos não rastreados"}; o Git
              recusa a troca se ela fosse sobrescrevê-los.
            </p>
          )}
          <div className="mt-2 flex gap-2">
            <Button variant="primary" onClick={props.onConfirm} disabled={props.switching || props.locked}>
              {props.switching ? <Spinner /> : null}
              {props.switching ? "Trocando…" : "Confirmar troca"}
            </Button>
            <Button onClick={props.onCancel} disabled={props.switching}>
              Cancelar
            </Button>
          </div>
        </div>
      )}
    </li>
  );
}

function CreateBranchForm(props: {
  repoPath: string;
  branches: BranchEntry[];
  defaultBase: string | null;
  busy: boolean;
  locked: boolean;
  onCancel: () => void;
  onSubmit: (name: string, base: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [base, setBase] = useState(props.defaultBase ?? "");
  const [check, setCheck] = useState<{ name: string; reason: string | null; error: GitError | null } | null>(null);
  const quick = name === "" ? null : quickBranchNameProblem(name);

  // Validação pelo próprio Git (com pequena espera ao digitar). Respostas antigas são descartadas.
  useEffect(() => {
    if (name === "" || quick !== null) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      gitService
        .checkBranchName(props.repoPath, name)
        .then((res) => {
          if (!cancelled) setCheck({ name, reason: res.valid ? null : (res.reason ?? "Nome inválido."), error: null });
        })
        .catch((e: unknown) => {
          if (!cancelled) setCheck({ name, reason: null, error: errorFrom(e) });
        });
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [name, quick, props.repoPath]);

  const checked = check !== null && check.name === name;
  const gitReason = checked ? check.reason : null;
  const validating = name !== "" && quick === null && !checked;
  const problem = quick ?? gitReason;
  const canCreate =
    name !== "" && quick === null && checked && gitReason === null && check.error === null && base !== "" && !props.busy && !props.locked;

  return (
    <form
      className="mb-4 rounded-md border border-line bg-surface p-4"
      aria-label="Criar branch"
      onSubmit={(event) => {
        event.preventDefault();
        if (canCreate) void props.onSubmit(name, base);
      }}
    >
      <p className="mb-3 text-sm font-medium">Criar uma nova branch local</p>
      <p className="mb-3 text-xs text-muted">
        Esta ação apenas cria a branch. Você continua na branch atual até escolher “Trocar”.
      </p>

      <label htmlFor="new-branch-name" className="text-xs text-muted">
        Nome da nova branch
      </label>
      <input
        id="new-branch-name"
        value={name}
        onChange={(event) => setName(event.target.value)}
        disabled={props.busy}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={problem !== null}
        placeholder="ex.: feature/login"
        className="mt-1 w-full rounded-md border border-line bg-canvas px-3 py-1.5 font-mono text-sm focus:border-accent disabled:opacity-60"
      />
      <p className={`mt-1 min-h-4 text-xs ${problem ? "text-removed" : "text-muted"}`} aria-live="polite">
        {problem ??
          (check?.error && checked ? check.error.message : validating ? "Verificando com o Git…" : name !== "" ? "Nome disponível." : "")}
      </p>

      <label htmlFor="new-branch-base" className="mt-2 block text-xs text-muted">
        Criar a partir de (branch-base)
      </label>
      <select
        id="new-branch-base"
        value={base}
        onChange={(event) => setBase(event.target.value)}
        disabled={props.busy}
        className="mt-1 w-full rounded-md border border-line bg-canvas px-3 py-1.5 font-mono text-sm focus:border-accent disabled:opacity-60"
      >
        {props.branches.map((b) => (
          <option key={b.name} value={b.name}>
            {b.name}
            {b.current ? " (atual)" : ""}
          </option>
        ))}
      </select>

      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={props.onCancel} disabled={props.busy}>
          Cancelar
        </Button>
        <Button type="submit" variant="primary" disabled={!canCreate}>
          {props.busy ? <Spinner /> : null}
          {props.busy ? "Criando…" : "Criar branch"}
        </Button>
      </div>
    </form>
  );
}
