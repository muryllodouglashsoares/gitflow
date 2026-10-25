# GitFlow Desktop (nome provisório) — Fase 2

Aplicativo desktop (Tauri + React + TypeScript + Rust) para **operar repositórios Git locais** sem digitar comandos.

- **Fase 1** (somente leitura): seleciona uma pasta, valida o repositório e mostra branch, estado, arquivos alterados e diffs.
- **Fase 2** (operações locais): lista, cria e troca branches; seleciona explicitamente os arquivos de um commit,
  revisa o diff do que será registrado e cria o commit local, mostrando o resultado real do Git.
  **Não há** fetch/pull/push, ZIP, merge nem autenticação (fases 3 a 5).

## Requisitos (Windows 10/11)

| Item | Observação |
| --- | --- |
| [Git for Windows](https://git-scm.com/download/win) | Usado para todas as consultas. Precisa estar no PATH (ou instalado em `Program Files\Git`). |
| [Node.js](https://nodejs.org) 20+ | Frontend e CLI do Tauri. |
| [Rust](https://rustup.rs) (toolchain `stable-msvc`, ≥ 1.80) | Camada nativa. |
| Visual Studio Build Tools 2022 (carga "Desenvolvimento para desktop com C++") | Linker do Rust no Windows. |
| WebView2 Runtime | Já vem no Windows 11; no Windows 10 [instale aqui](https://developer.microsoft.com/microsoft-edge/webview2/). |

## Comandos

```bash
npm install            # instala dependências do frontend e a CLI do Tauri
npm run tauri:dev      # executa o aplicativo em desenvolvimento (abre a janela desktop)
npm run tauri:build    # compila o aplicativo (Fase 1 não gera instalador: bundle.active = false)

npm run typecheck      # TypeScript
npm run build          # typecheck + build do frontend (dist/)
npm test               # testes do frontend (vitest)
npm run test:rust      # testes do backend (cargo test; exige Git instalado)
```

> `npm run dev` abre só o frontend no navegador; os comandos nativos exigem `npm run tauri:dev`.

## Arquitetura

```
src/                         Frontend React
  types/git.ts               Tipos espelhando os modelos serializados pelo Rust
  services/gitService.ts     ÚNICA fronteira com o backend (invoke dos comandos)
  services/folderPicker.ts   Seletor de pastas nativo (plugin dialog)
  lib/                       Lógica pura e testada (agrupamento, rótulos, erros)
  hooks/                     Estado: repositório aberto, status e diff (anti-resposta obsoleta)
  components/                layout · changes · diff · welcome · common
src-tauri/
  src/main.rs, commands.rs   comandos Tauri finos (async + spawn_blocking)
  build.rs                   Registra os comandos permitidos (app manifest)
  capabilities/default.json  Permissões mínimas da janela
  git-core/                  Crate `gitflow-git` (sem dependência do Tauri)
    runner.rs                Execução controlada do Git
    repository.rs            Disponibilidade do Git e validação de repositório
    status.rs                `status --porcelain=v2 -z` + parser
    diff.rs                  Diff por alvo + parser unificado
    error.rs                 Erros serializáveis { code, message, details }
    tests/integration.rs     Testes com Git real em repositórios temporários
```

Comandos nativos:

| Tipo | Comandos |
| --- | --- |
| Leitura | `check_git`, `open_repository`, `get_repository_status`, `get_file_diff` (agora também com o alvo `head`), `list_branches`, `check_branch_name`, `prepare_commit_review` |
| Escrita local (Fase 2) | `create_branch` (não troca de branch), `switch_branch`, `create_commit` |

Novos módulos em `git-core/src/`: `branches.rs` (listar/validar/criar/trocar), `commit.rs` (revisão e commit seletivo),
`lock.rs` (uma operação de escrita por repositório). `runner.rs` ganhou `run_raw` (stdout/stderr/código sem interpretação,
com `stdin` opcional para listas de caminhos). No frontend: `lib/commit.ts`, `lib/branches.ts`, hooks `useCommitFlow`,
`useBranches`, `useBranchActions` e as telas `components/branches` e `components/commit`.

### Como a Fase 2 garante que só o selecionado entra no commit

- O commit usa `git commit --only` com a lista de arquivos marcados (via `--pathspec-from-file=-`, NUL-separada, e
  `--literal-pathspecs`). **Nunca** `git add .`, `git add -A` ou `commit -a`.
- Arquivos **não selecionados** não são tocados, e o que já estava preparado (staged) para eles **continua preparado**
  depois do commit (a revisão lista esses arquivos).
- Para um arquivo selecionado, entra o **conteúdo atual dele** (parte preparada + não preparada). O diff da revisão é
  `git diff HEAD -- arquivo` (alvo `head`), ou seja, exatamente o que será registrado; arquivos parcialmente preparados
  recebem um aviso explícito.
- Arquivos novos recebem `git add --intent-to-add` só para o Git reconhecê-los; se o commit falhar, isso é desfeito
  (`git reset` restrito a esses caminhos, e só se o HEAD não mudou).
- A revisão devolve uma impressão digital (branch, HEAD e conteúdo atual de cada arquivo). `create_commit` a recalcula;
  se algo mudou, recusa (`STALE_REVIEW`) e a interface refaz a revisão e pede nova confirmação.
- Bloqueios: merge/rebase/cherry-pick/revert em andamento, conflitos, HEAD desanexado, submódulos e pastas aninhadas.
- Sucesso só é exibido se o HEAD realmente avançou; hash, branch e arquivos vêm do Git depois do commit.

### Troca de branch

Alterações em arquivos rastreados (preparadas ou não) **bloqueiam** a troca, listando os arquivos e explicando o que fazer
manualmente. Arquivos apenas não rastreados não bloqueiam (o Git recusa sozinho se fossem sobrescritos). O aplicativo nunca
usa `--force`, `stash`, `reset` ou `clean` para viabilizar a troca. Criar uma branch **não** troca para ela.

### Requisitos adicionais da Fase 2

Git **2.25+** (`--pathspec-from-file`) e **2.23+** (`git switch`). Hooks do repositório (por exemplo, `pre-commit`) são
executados normalmente pelo Git durante o commit; se recusarem, o erro é exibido e nada é perdido.

### Segurança

- Git é executado **sem shell**, com argumentos separados; a interface nunca envia comandos, apenas caminhos.
- Todo comando de leitura revalida que `repoPath` é **a raiz** de um repositório; caminhos de arquivo são
  rejeitados se vazios, absolutos, com `..` ou NUL, e interpretados literalmente (`--literal-pathspecs`).
- Ambiente endurecido: `core.fsmonitor=false` (a configuração de um repositório não executa programas),
  `--no-ext-diff`, `--no-textconv`, `--no-optional-locks`, `GIT_TERMINAL_PROMPT=0`, variáveis `GIT_DIR`/`GIT_INDEX_FILE`… removidas.
- Tempo limite e limite de saída em toda chamada; diff e linhas muito grandes são truncados.
- Capabilities do Tauri concedem apenas `dialog:allow-open` e os 10 comandos listados. CSP restritiva.
- Nenhum log de conteúdo de arquivos; mensagens de erro técnicas ficam em "Detalhes técnicos".

### Decisões relevantes

- **Crate separado `gitflow-git`**: a lógica de Git é testável sem Tauri e reaproveitada pelas próximas fases.
- **`status --porcelain=v2 -z`**: formato estável para programas; nomes com espaços/acentos não precisam de escape.
- **Estado preparado × não preparado** são mostrados em grupos separados (cada um tem diff próprio).
- **Respostas obsoletas** são descartadas por número de sequência e conferência do `repoPath` retornado.
- **Dependências**: apenas React, `@tauri-apps/api`, `@tauri-apps/plugin-dialog` e Tailwind. O diff usa uma
  tabela própria (sem biblioteca extra) por ser somente leitura e simples.

## Validação manual da Fase 2

1. `npm run tauri:dev`, abra um repositório de teste (crie um com `git init` numa pasta descartável).
2. **Branches**: aba “Branches” → “Nova branch…” → nome inválido (`a b`, `-x`, `a..b`) é recusado; nome válido cria sem trocar;
   “Trocar” funciona com a árvore limpa e é bloqueado (com a lista de arquivos) se houver arquivos modificados.
3. **Seleção**: modifique 3 arquivos e prepare 1 deles com `git add`. Marque apenas um dos outros; “Revisar e commitar…”.
   Confira na revisão que o preparado não selecionado aparece em “Fora do commit, mantidos como estão”.
4. **Commit**: mensagem vazia desabilita o botão; marque a confirmação; “Criar commit”. Confira com `git log --stat` e
   `git status` que só o arquivo marcado entrou e que o arquivo preparado antes continua preparado.
5. **Mudança durante a revisão**: edite o arquivo revisado no editor e volte à janela: a revisão é refeita e a confirmação cai.
6. **Falha**: crie `.git/hooks/pre-commit` que retorne `exit 1`: o erro aparece, a seleção e a mensagem permanecem.

## Fora do escopo (próximas fases)

fetch/pull/push, merge, importação de ZIP, autenticação, pull requests, instalador/publicação.
A navegação lateral (`NavId`) e `App.tsx` já têm o ponto de extensão.
