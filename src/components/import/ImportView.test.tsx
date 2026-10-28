/**
 * Teste de fumaça da tela de importação: renderiza `ImportView` no servidor (react-dom/server,
 * sem DOM) em CADA etapa do fluxo, com um controlador fabricado, e falha se alguma etapa lançar
 * exceção ou exibir valores quebrados ("undefined", "NaN", "[object Object]"). Não substitui
 * testes de interação, que o projeto não tem; a lógica do fluxo é testada em `lib/importFlow.test.ts`.
 */
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { INITIAL_IMPORT_FLOW, STAGE_LABELS, type ImportFlowState, type ImportStage } from "../../lib/importFlow";
import type { ZipImportController } from "../../hooks/useZipImport";
import type { GitError, ImportPlan, ImportResult, PlanEntry, RepositoryStatus, RollbackResult } from "../../types/git";
import { ImportView } from "./ImportView";

const REPO = "/repo";
const noop = () => undefined;

function entry(path: string, action: PlanEntry["action"], reason: string | null = null): PlanEntry {
  return {
    path,
    action,
    size: 1536,
    reason,
    newSha256: "n".repeat(64),
    oldSha256: null,
    zipIndex: 0,
    zipName: path,
  };
}

const entries: PlanEntry[] = [
  entry("novo.txt", "add"),
  entry("src/app.ts", "replace"),
  entry("igual.txt", "identical"),
  entry("con.txt", "blocked", "Nome inválido: usa o nome reservado do Windows."),
];

const plan: ImportPlan = {
  planId: "p".repeat(64),
  repoPath: REPO,
  baseBranch: "main",
  baseOid: "a".repeat(40),
  zipPath: "C:\\Downloads\\pacote.zip",
  zipSize: 2048,
  zipSha256: "z".repeat(64),
  sourcePrefix: "",
  layout: { topLevelDirs: ["projeto"], hasRootFiles: false, suggestedPrefix: "projeto", directories: ["projeto"] },
  entries,
  summary: {
    totalEntries: 4,
    directories: 0,
    filesInScope: 4,
    add: 1,
    replace: 1,
    identical: 1,
    blocked: 1,
    ignored: 2,
    bytesToWrite: 3072,
    uncompressedBytes: 6144,
  },
  blockers: ["Há 1 arquivo(s) com alterações locais (a.txt)."],
  warnings: ["Todos os arquivos estão dentro de “projeto”. Escolha a pasta do pacote se ela for a raiz do projeto."],
  canApply: false,
};

const result: ImportResult = {
  opId: "0123456789abcdef",
  state: "applied",
  repoPath: REPO,
  workBranch: "importacao/x",
  applied: [
    { path: "novo.txt", action: "add", visibleInGit: true },
    { path: "src/app.ts", action: "replace", visibleInGit: true },
  ],
  failure: null,
  notAttempted: [],
  identicalSkipped: 1,
  recovery: null,
};

const partialResult: ImportResult = {
  ...result,
  state: "partial",
  failure: { path: "z.txt", message: "disco cheio" },
  notAttempted: ["w.txt"],
  recovery: "Desfaça a importação.",
};

const rollback: RollbackResult = {
  opId: "0123456789abcdef",
  state: "rolledBack",
  restored: ["src/app.ts"],
  removed: ["novo.txt"],
  keptModified: ["a1.txt"],
  problems: ["Não foi possível restaurar x"],
};

const status: RepositoryStatus = {
  repoPath: REPO,
  branch: { name: "main", detached: false, unborn: false, headOid: "a".repeat(40), upstream: null, ahead: null, behind: null },
  files: [],
  summary: { total: 0, staged: 0, unstaged: 0, untracked: 0, conflicted: 0 },
  isClean: true,
  operation: null,
};

function err(code: GitError["code"] = "COMMAND_FAILED", message = "Algo falhou", details?: string): GitError {
  return details ? { code, message, details } : { code, message };
}

function flowAt(stage: ImportStage, patch: Partial<ImportFlowState> = {}): ImportFlowState {
  return {
    ...INITIAL_IMPORT_FLOW,
    repoPath: REPO,
    stage,
    zipPath: "C:\\Downloads\\pacote.zip",
    baseBranch: "main",
    workBranch: "importacao/pacote-20261027",
    log: [{ step: "Inspeção do ZIP", outcome: "ok", at: 1_800_000_000_000, detail: "1 novo" }],
    ...patch,
  };
}

function ctlFor(flow: ImportFlowState, patch: Record<string, unknown> = {}): ZipImportController {
  const commit = {
    selected: new Set<string>(),
    step: "select",
    review: { data: null, loading: false, error: null },
    reviewVersion: 0,
    reviewSelection: null,
    focusPath: null,
    focus: noop,
    message: "",
    changeMessage: noop,
    messageProblem: null,
    confirmed: false,
    setConfirmed: noop,
    committing: flow.stage === "committing",
    canSubmit: false,
    result: null,
    commitError: null,
    notice: null,
    blocker: null,
    toggle: noop,
    setMany: noop,
    clearSelection: noop,
    startReview: noop,
    backToSelection: noop,
    submit: noop,
    dismissResult: noop,
    dismissCommitError: noop,
  };
  const ctl = {
    flow,
    running: ["inspecting", "creatingBranch", "importing", "rollingBack", "committing", "pushing"].includes(flow.stage),
    picking: false,
    pickError: null,
    chooseZip: noop,
    inspect: noop,
    cancel: noop,
    changeBase: noop,
    changePrefix: noop,
    changeWorkBranch: noop,
    toggleExclude: noop,
    problems: ["Confirme o plano e a criação da branch de trabalho."],
    overwrites: entries.filter((e) => e.action === "replace"),
    writes: entries.filter((e) => e.action === "add" || e.action === "replace"),
    overwriteAck: false,
    setOverwriteAck: noop,
    planAck: false,
    setPlanAck: noop,
    startImport: noop,
    rollback: noop,
    recover: noop,
    recoverError: null,
    restart: noop,
    dismissIssue: noop,
    commit,
    restrictedStatus: { data: status, loading: false, error: null, updatedAt: 1 },
    selection: null,
    select: noop,
    diff: { data: null, loading: false, error: null },
    retryDiff: noop,
    foreign: ["alheio.txt"],
    pushPlan: {
      target: { branch: "importacao/x", remote: "origin", remoteBranch: "importacao/x" },
      problems: [],
      protectedDestination: false,
    },
    pushAck: false,
    setPushAck: noop,
    protectedAck: false,
    setProtectedAck: noop,
    canPush: false,
    push: noop,
    finishWithoutPush: noop,
    currentBranch: "main",
    ...patch,
  };
  return ctl as unknown as ZipImportController;
}

function render(flow: ImportFlowState, patch: Record<string, unknown> = {}, locked = false): string {
  return renderToStaticMarkup(
    <ImportView
      repoPath={REPO}
      ctl={ctlFor(flow, patch)}
      branches={{
        repoPath: REPO,
        current: "main",
        detached: false,
        unborn: false,
        headOid: "a".repeat(40),
        branches: [{ name: "main", current: true, oid: "a".repeat(40), upstream: null, subject: "inicial" }],
      }}
      locked={locked}
      syncStatus={null}
      onRefresh={noop}
    />,
  );
}

function expectSane(html: string) {
  expect(html).not.toMatch(/undefined|NaN|\[object Object\]/);
}

describe("ImportView renderiza todas as etapas sem quebrar", () => {
  it("seleção sem ZIP e com ZIP", () => {
    const empty = render(flowAt("selecting", { zipPath: null, baseBranch: null }));
    expectSane(empty);
    expect(empty).toContain("Escolher ZIP");
    expect(empty).toContain("Nenhum arquivo selecionado");
    expect(empty).toContain(STAGE_LABELS.selecting);

    const withZip = render(flowAt("selecting"));
    expectSane(withZip);
    expect(withZip).toContain("Trocar ZIP");
    expect(withZip).toContain("Inspecionar ZIP");
    expect(withZip).toContain("nada é extraído");
  });

  it("inspeção em andamento (indeterminada, com cancelamento) e ZIP inválido", () => {
    const inspecting = render(flowAt("inspecting"));
    expectSane(inspecting);
    expect(inspecting).toContain("Cancelar inspeção");
    expect(inspecting).not.toMatch(/\d+\s?%/); // nenhuma porcentagem inventada

    const invalid = render(flowAt("zipInvalid", { inspectError: err("INVALID_ARCHIVE", "O arquivo não é um ZIP válido.", "detalhe") }));
    expectSane(invalid);
    expect(invalid).toContain("Não foi possível usar este ZIP");
    expect(invalid).toContain("O arquivo não é um ZIP válido.");
  });

  it("revisão do plano: contagens, bloqueios, sobrescrita e confirmação", () => {
    const html = render(flowAt("planReview", { plan, sourcePrefix: "" }));
    expectSane(html);
    expect(html).toContain("Plano de importação");
    expect(html).toContain("Nada é excluído");
    expect(html).toContain("Há 1 arquivo(s) com alterações locais");
    expect(html).toContain("1 entrada(s) bloqueada(s) por segurança");
    expect(html).toContain("con.txt");
    expect(html).toContain("será(ão) substituído(s)");
    expect(html).toContain("Confirmo este plano");
    expect(html).toContain("Pasta do pacote");
    expect(html).toContain("1.5 KiB");
    // Sem confirmação o botão fica desabilitado e os motivos aparecem.
    expect(html).toMatch(/<button[^>]*disabled[^>]*>Criar branch e importar 2 arquivo\(s\)/);
    expect(html).toContain("Confirme o plano e a criação da branch de trabalho.");
  });

  it("criando a branch, importando e desfazendo (progresso indeterminado)", () => {
    const creating = render(flowAt("creatingBranch"));
    expect(creating).toContain("Criando a branch de trabalho");
    expect(creating).toContain("Nenhum arquivo foi gravado ainda");
    const importing = render(flowAt("importing"));
    expect(importing).toContain("Importando arquivos");
    const rolling = render(flowAt("rollingBack"));
    expect(rolling).toContain("Desfazendo a importação");
    for (const html of [creating, importing, rolling]) {
      expectSane(html);
      expect(html).not.toMatch(/\d+\s?%/);
    }
  });

  it("gravação parcial: informa o que ocorreu e oferece desfazer", () => {
    const html = render(flowAt("partial", { result: partialResult, opId: partialResult.opId, importedPaths: ["novo.txt"] }));
    expectSane(html);
    expect(html).toContain("Importação parcialmente concluída");
    expect(html).toContain("Nenhum commit foi criado");
    expect(html).toContain("z.txt");
    expect(html).toContain("disco cheio");
    expect(html).toContain("Desfazer a importação");
  });

  it("falhas: recuperável e que exige intervenção manual", () => {
    const base = {
      at: "creatingBranch" as ImportStage,
      error: err("OPERATION_BLOCKED", "A branch foi criada, mas…"),
      completed: ["ZIP inspecionado"],
      advice: ["Nenhum arquivo do ZIP foi gravado."],
    };
    const recoverable = render(flowAt("failed", { failure: { ...base, manual: false } }));
    expectSane(recoverable);
    expect(recoverable).toContain("Falha recuperável");
    expect(recoverable).toContain("Já concluído");
    expect(recoverable).toContain("Verifique antes de continuar");
    expect(recoverable).toContain("Recomeçar");

    const manual = render(flowAt("failed", { failure: { ...base, at: "importing", manual: true } }));
    expectSane(manual);
    expect(manual).toContain("Falha que exige intervenção manual");
    expect(manual).toContain("Voltar para “main” e recomeçar");
  });

  it("importação desfeita lista restaurados, removidos, preservados e problemas", () => {
    const html = render(flowAt("rolledBack", { rollback }));
    expectSane(html);
    expect(html).toContain("Importação desfeita");
    expect(html).toContain("a1.txt");
    expect(html).toContain("Não foi possível restaurar x");
    expect(html).toContain("Nova importação");
  });

  it("revisão e criação do commit reaproveitam o painel de alterações", () => {
    const review = render(flowAt("commitReview", { result, opId: result.opId, importedPaths: ["novo.txt", "src/app.ts"] }));
    expectSane(review);
    expect(review).toContain("2</strong> arquivo(s) foram gravados");
    expect(review).toContain("Nada será enviado ao remoto nesta etapa");
    expect(review).toContain("1 outra(s) alteração(ões)");
    expect(review).toContain("Desfazer a importação");

    const committing = render(flowAt("committing", { result, opId: result.opId, importedPaths: ["novo.txt"] }));
    expectSane(committing);
    expect(committing).toContain(STAGE_LABELS.committing);

    const reviewing = render(flowAt("commitReview", { result, opId: result.opId, importedPaths: ["novo.txt"] }), {
      commit: { ...ctlFor(flowAt("commitReview")).commit, step: "review", review: { data: null, loading: true, error: null } },
    });
    expectSane(reviewing);
  });

  it("commit concluído: push com destino exato, confirmação e bloqueios", () => {
    const committed = flowAt("committed", { result, opId: result.opId, commitOid: "c".repeat(40) });
    const ok = render(committed);
    expectSane(ok);
    expect(ok).toContain("Publicar no remoto");
    expect(ok).toContain("origin/importacao/x");
    expect(ok).toContain("Concluir sem enviar");
    expect(ok).toMatch(/<button[^>]*disabled[^>]*>Enviar ao remoto/); // exige confirmação
    expect(ok).toContain("Confirmo o envio");

    const blocked = render(committed, {
      pushPlan: { target: null, problems: ["A branch atual não tem upstream."], protectedDestination: false },
    });
    expectSane(blocked);
    expect(blocked).toContain("Não é possível enviar agora");
    expect(blocked).toContain("A branch atual não tem upstream.");
    expect(blocked).not.toContain("Confirmo o envio");

    const protectedDest = render(committed, {
      pushPlan: {
        target: { branch: "importacao/x", remote: "origin", remoteBranch: "main" },
        problems: [],
        protectedDestination: true,
      },
    });
    expect(protectedDest).toContain("branch protegida");

    const failedPush = render(
      flowAt("committed", {
        result,
        opId: result.opId,
        commitOid: "c".repeat(40),
        issue: { kind: "push", error: err("TIMEOUT", "demorou"), unconfirmed: true },
      }),
    );
    expectSane(failedPush);
    expect(failedPush).toContain("O envio ao remoto não foi concluído");
    expect(failedPush).toContain("Resultado não confirmado");

    const pushing = render(flowAt("pushing", { result, opId: result.opId, commitOid: "c".repeat(40) }));
    expectSane(pushing);
    expect(pushing).toContain("Enviando ao remoto");
  });

  it("concluído (com e sem push) e cancelado", () => {
    const sent = render(
      flowAt("done", {
        importedPaths: ["a", "b"],
        commitOid: "c".repeat(40),
        push: {
          repoPath: REPO,
          target: { branch: "importacao/x", remote: "origin", remoteBranch: "importacao/x" },
          pushedCommits: 1,
          localOid: "c".repeat(40),
          output: "",
          sync: null,
        } as never,
      }),
    );
    expectSane(sent);
    expect(sent).toContain("Importação concluída");
    expect(sent).toContain("Enviado para origin/importacao/x");

    const local = render(flowAt("done", { importedPaths: ["a"], commitOid: "c".repeat(40) }));
    expect(local).toContain("Não enviado");

    const cancelled = render(flowAt("cancelled"));
    expectSane(cancelled);
    expect(cancelled).toContain("Nada foi alterado");
  });

  it("outra operação em andamento é sinalizada e o registro aparece", () => {
    const html = render(flowAt("selecting"), {}, true);
    expect(html).toContain("Outra operação do repositório está em andamento");
    expect(html).toContain("Registro da operação (1)");
  });
});
