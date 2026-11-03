/**
 * Teste de fumaça da tela de merge: renderiza `MergeView` no servidor (react-dom/server, sem DOM)
 * em cada estágio do fluxo com um controlador fabricado e falha se algo lançar exceção ou exibir
 * valores quebrados. A lógica do fluxo é testada em `lib/merge.test.ts` e o comportamento Git real
 * em `src-tauri/git-core/tests/phase5.rs`.
 */
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { MergeController } from "../../hooks/useMerge";
import { deriveStage, type MergePhase } from "../../lib/merge";
import type {
  BranchEntry,
  BranchList,
  MergeBlocker,
  MergePreview,
  MergeResult,
  RepositoryStatus,
} from "../../types/git";
import { MergeView } from "./MergeView";

const noop = () => undefined;
const asyncNoop = () => Promise.resolve();

const entry = (name: string, current = false): BranchEntry => ({
  name,
  current,
  oid: "abcdef1234567890",
  upstream: null,
  subject: "assunto do commit",
});

const list: BranchList = {
  repoPath: "/repo",
  current: "main",
  detached: false,
  unborn: false,
  headOid: "abcdef1234567890",
  branches: [entry("main", true), entry("feature/login")],
};

function preview(o: Partial<MergePreview> = {}): MergePreview {
  return {
    repoPath: "/repo",
    source: entry("feature/login"),
    target: entry("main", true),
    targetProtected: true,
    requiredConfirmation: "main",
    currentBranch: "main",
    headOid: "abcdef1234567890",
    targetIsCurrent: true,
    relation: "diverged",
    strategy: "mergeCommit",
    mergeBase: "1234567890abcdef",
    sourceOnly: 2,
    targetOnly: 1,
    sourceCommits: [
      { oid: "aaaaaaa1111", subject: "adiciona login" },
      { oid: "bbbbbbb2222", subject: "ajusta formulário" },
    ],
    forecast: { state: "clean", files: [] },
    summary: { total: 0, staged: 0, unstaged: 0, untracked: 0, conflicted: 0 },
    operation: null,
    blockers: [],
    notes: [],
    canExecute: true,
    token: "token",
    ...o,
  };
}

const status = { isClean: false, operation: "merge", summary: { conflicted: 1 } } as unknown as RepositoryStatus;

function result(o: Partial<MergeResult> = {}): MergeResult {
  return {
    repoPath: "/repo",
    source: "feature/login",
    target: "main",
    outcome: "mergeCommit",
    sourceOid: "aaaaaaa1111",
    headBefore: "0000000bbbb",
    headAfter: "9999999cccc",
    mergeCommit: true,
    integratedCommits: 2,
    conflicts: [],
    exitCode: 0,
    output: "Merge made by the 'ort' strategy.",
    status: { ...status, isClean: true, operation: null, summary: { conflicted: 0 } } as RepositoryStatus,
    ...o,
  };
}

function controller(o: Partial<MergeController> & { pv?: MergePreview | null; phase?: MergePhase } = {}): MergeController {
  const { pv = preview(), phase = "idle", ...rest } = o;
  const base = {
    source: "feature/login",
    target: "main",
    setSource: noop,
    setTarget: noop,
    swap: noop,
    preview: { key: "k", data: pv, loading: false, error: null },
    phase,
    acknowledged: false,
    setAcknowledged: noop,
    typed: "",
    setTyped: noop,
    ready: false,
    result: null,
    runError: null,
    notice: null,
    recheck: noop,
    prepare: noop,
    cancelConfirmation: noop,
    execute: asyncNoop,
    switchToTarget: asyncNoop,
    finish: noop,
    dismissRunError: noop,
    dismissNotice: noop,
  };
  const merged = { ...base, ...rest } as Omit<MergeController, "stage">;
  const stage = deriveStage({
    source: merged.source,
    target: merged.target,
    preview: merged.preview.data,
    previewLoading: merged.preview.loading,
    previewError: merged.preview.error,
    phase: merged.phase,
    result: merged.result,
    runError: merged.runError,
  });
  return { ...merged, stage } as MergeController;
}

function render(ctl: MergeController, locked = false) {
  return renderToStaticMarkup(
    <MergeView
      branches={{ data: list, loading: false, error: null }}
      ctl={ctl}
      locked={locked}
      switching={false}
      switchError={null}
      onDismissSwitchError={noop}
      onReloadBranches={noop}
      onOpenChanges={noop}
    />,
  );
}

const BROKEN = /undefined|NaN|\[object Object\]/;
const blocker = (kind: MergeBlocker["kind"], files: string[] = []): MergeBlocker => ({
  kind,
  message: "mensagem do bloqueio",
  files,
  filesTotal: files.length + 3,
});

describe("MergeView", () => {
  it("aguarda a seleção sem mostrar prévia", () => {
    const html = render(controller({ source: null, pv: null }));
    expect(html).toContain("Aguardando seleção");
    expect(html).toContain("Nada é alterado enquanto você apenas seleciona");
    expect(html).not.toMatch(BROKEN);
  });

  it("mostra a direção e o botão que apenas abre a confirmação", () => {
    const html = render(controller());
    expect(html).toContain("Pronto para integrar");
    expect(html).toContain("Direção da integração");
    expect(html).toContain("Preparar integração");
    expect(html).toContain("principal");
    expect(html).toContain("adiciona login");
    expect(html).not.toContain("Confirmar merge");
    expect(html).not.toMatch(BROKEN);
  });

  it("exige digitar main e o aceite na confirmação, com o botão desabilitado até lá", () => {
    const html = render(controller({ phase: "confirming" }));
    expect(html).toContain("Aguardando confirmação");
    expect(html).toContain("Confirmar merge");
    expect(html).toContain("modificará a branch principal");
    expect(html).toContain("Integrar feature/login em main");
    expect(html).toMatch(/<button[^>]*disabled[^>]*>Integrar feature\/login em main/);
    expect(html).not.toMatch(BROKEN);
  });

  it("bloqueios listam os arquivos e oferecem a troca explícita de branch", () => {
    const html = render(
      controller({
        pv: preview({
          canExecute: false,
          blockers: [blocker("stagedChanges", ["a.txt", "b/c.txt"]), blocker("targetNotCurrent")],
        }),
      }),
    );
    expect(html).toContain("Operação bloqueada");
    expect(html).toContain("a.txt");
    expect(html).toContain("… e mais 3");
    expect(html).not.toContain("Trocar para"); // há outro bloqueio além do destino não atual
    expect(html).toMatch(/<button[^>]*disabled[^>]*>Preparar integração/);

    const onlyTarget = render(
      controller({ pv: preview({ canExecute: false, targetIsCurrent: false, blockers: [blocker("targetNotCurrent")] }) }),
    );
    expect(onlyTarget).toContain("Trocar para");
    expect(onlyTarget).not.toMatch(BROKEN);
  });

  it("indica que não há nada a integrar", () => {
    const html = render(controller({ pv: preview({ canExecute: false, relation: "alreadyIntegrated", strategy: null }) }));
    expect(html).toContain("Nada a integrar");
    expect(html).toContain("Já integrada");
  });

  it("mostra a execução em andamento", () => {
    const html = render(controller({ phase: "running" }), true);
    expect(html).toContain("Executando merge");
    expect(html).not.toMatch(BROKEN);
  });

  it("mostra o resultado verdadeiro de um merge concluído", () => {
    const html = render(controller({ phase: "done", result: result() }));
    expect(html).toContain("Merge concluído");
    expect(html).toContain("9999999");
    expect(html).toContain("Commit de merge criado");
    expect(html).not.toContain("NÃO concluído");
    expect(html).not.toMatch(BROKEN);
  });

  it("com conflitos, nunca declara sucesso e lista os caminhos", () => {
    const html = render(
      controller({
        phase: "done",
        result: result({
          outcome: "conflicts",
          mergeCommit: false,
          headAfter: "0000000bbbb",
          conflicts: [{ path: "src/shared.txt", code: "UU" }],
          exitCode: 1,
          status,
        }),
      }),
    );
    expect(html).toContain("Conflitos encontrados");
    expect(html).toContain("NÃO concluído");
    expect(html).toContain("src/shared.txt");
    expect(html).toContain("Ver arquivos em conflito");
    expect(html).not.toContain("Merge concluído");
    expect(html).not.toMatch(BROKEN);
  });

  it("mostra a falha da execução", () => {
    const html = render(controller({ runError: { code: "STALE_REVIEW", message: "A revisão está desatualizada." } }));
    expect(html).toContain("Falha na operação");
    expect(html).toContain("A revisão está desatualizada.");
  });
});
