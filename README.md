# GitFlow Desktop (nome provisório) — Fase 3

Aplicativo desktop (Tauri + React + TypeScript + Rust) para **operar repositórios Git locais** sem digitar comandos.

- **Fase 1** (somente leitura): seleciona uma pasta, valida o repositório e mostra branch, estado, arquivos alterados e diffs.
- **Fase 2** (operações locais): lista, cria e troca branches; seleciona explicitamente os arquivos de um commit,
  revisa o diff do que será registrado e cria o commit local, mostrando o resultado real do Git.
- **Fase 3** (sincronização remota): diagnóstico de remotos/upstream (sincronizada, adiantada, atrasada, divergente) e
  as operações **fetch**, **pull** e **push** pela interface, com confirmação do destino exato, bloqueios de segurança e
  o resultado real do Git. **Não há** ZIP, merge manual, pull requests nem armazenamento de credenciais (fases 4 a 5).

## Início rápido (Windows)

Dê **duplo clique em `iniciar.bat`** (na raiz do projeto). Ele abre um terminal que:

1. verifica Git (2.25+), Node.js (20+), Visual Studio Build Tools (C++), WebView2 e Rust (1.80+);
2. instala o que faltar (via `winget`; o Rust via `rustup`) — o Windows pede permissão de administrador só se necessário;
3. executa `npm install` (apenas na primeira vez ou quando `package*.json` mudar);
4. inicia o aplicativo (`npm run tauri:dev`). **Mantenha o terminal aberto** enquanto usa o app.

Na primeira execução a instalação e a compilação do Rust levam vários minutos. Se o instalador pedir reinicialização,
reinicie o Windows e abra o `iniciar.bat` de novo. Requer internet na primeira vez e o `winget` (Windows 10 1809+ / 11).

## Requisitos (Windows 10/11)

| Item (instalados automaticamente pelo `iniciar.bat`) | Observação |
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
    remote.rs                Fase 3: diagnóstico de sincronização, fetch, pull e push, classificação de falhas
    error.rs                 Erros serializáveis { code, message, details }
    tests/integration.rs     Testes com Git real em repositórios temporários
    tests/phase3.rs          Testes da Fase 3 com remotos locais (repositórios bare temporários)
```

Comandos nativos:

| Tipo | Comandos |
| --- | --- |
| Leitura | `check_git`, `open_repository`, `get_repository_status`, `get_file_diff` (agora também com o alvo `head`), `list_branches`, `check_branch_name`, `prepare_commit_review` |
| Escrita local (Fase 2) | `create_branch` (não troca de branch), `switch_branch`, `create_commit` |
| Sincronização (Fase 3) | `get_sync_status` (somente leitura, sem rede), `fetch_remote`, `pull_branch`, `push_branch` |

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
- Capabilities do Tauri concedem apenas `dialog:allow-open` e os 14 comandos listados. CSP restritiva.
- Nenhum log de conteúdo de arquivos; mensagens de erro técnicas ficam em "Detalhes técnicos".

### Decisões relevantes

- **Crate separado `gitflow-git`**: a lógica de Git é testável sem Tauri e reaproveitada pelas próximas fases.
- **`status --porcelain=v2 -z`**: formato estável para programas; nomes com espaços/acentos não precisam de escape.
- **Estado preparado × não preparado** são mostrados em grupos separados (cada um tem diff próprio).
- **Respostas obsoletas** são descartadas por número de sequência e conferência do `repoPath` retornado.
- **Dependências**: apenas React, `@tauri-apps/api`, `@tauri-apps/plugin-dialog` e Tailwind. O diff usa uma
  tabela própria (sem biblioteca extra) por ser somente leitura e simples.

## Fase 3 — Sincronização remota

Tela **Sincronização** na barra lateral (o texto de acompanhamento do cabeçalho também abre essa tela).

### Diagnóstico (somente leitura, não usa a rede)

Remotos (URLs **sem usuário/senha**), branch atual, upstream e o estado, calculados a partir das referências **locais**:

| Estado | Significado |
| --- | --- |
| Sem remoto / Sem upstream / Upstream inválido / HEAD desanexado / Sem commits | Não há destino utilizável; pull e push ficam bloqueados com a explicação |
| Remoto ainda não verificado | Upstream configurado, mas a referência remota ainda não existe localmente: falta `fetch` |
| Sincronizada · Adiantada · Atrasada · Divergente | Comparação `HEAD...upstream` (`git rev-list --left-right --count`) |
| Possivelmente desatualizada | Sem fetch há mais de 30 min (ou nunca): o aplicativo **não** afirma "sincronizada" |
| Remoto não verificável | A última tentativa de falar com o remoto falhou (rede, credenciais, tempo) |

A data do último fetch vem do `FETCH_HEAD` do Git (e do último fetch/pull concluído na sessão). Limitação: um fetch que
falha pode regravar o `FETCH_HEAD`; por isso, após uma falha de rede/credenciais, a interface passa a mostrar "Remoto não
verificável" até o próximo sucesso.

### Operações

- **Fetch** — `git fetch --no-recurse-submodules <remoto>`. O remoto tem de ser um dos listados por `git remote` (nunca
  texto livre). Não altera arquivos nem branches locais e **não integra nada**. Sem confirmação (é só consulta).
- **Pull** — `git pull --no-edit --no-recurse-submodules <remoto> <branch>` com remoto e branch do **upstream já confirmado**.
  Bloqueado com alterações locais em arquivos rastreados (preparadas ou não), conflitos, merge/rebase/etc. em andamento,
  HEAD desanexado, sem upstream, ou divergência sem estratégia configurada. Arquivos não rastreados **não** bloqueiam (o Git
  recusa sozinho se fossem sobrescritos). A estratégia é a que o **seu** Git já tem (`pull.rebase`, `pull.ff`); se não há
  nenhuma, o pull só aceita avanço rápido (`--ff-only`, sem merge nem rebase). Conflitos **não** são resolvidos: o
  repositório fica no estado do Git e a interface explica como seguir.
- **Push** — `git push --porcelain --no-recurse-submodules <remoto> refs/heads/<local>:refs/heads/<remota>` (refspec
  explícito do upstream). Nunca usa `--force`/`--force-with-lease`, `-u`, `--all`, `--mirror` nem `--tags`, nunca cria ou
  altera upstream e não pula hooks (`--no-verify`). Bloqueado sem commits a enviar, com o remoto à frente/divergente, sem
  upstream ou com a referência remota ainda não buscada. Para `main`/`master` exige uma segunda confirmação (caixa de
  seleção na interface **e** campo `confirmProtectedBranch` validado no backend).
- **Destino confirmado**: pull e push recebem o destino que o usuário viu (branch, remoto, branch remota). O backend relê o
  estado dentro da trava do repositório e recusa (`STALE_REVIEW`) se algo mudou.
- **Sucesso só com prova**: código de saída 0 e, no push, nenhuma linha rejeitada na saída `--porcelain`; no pull, o HEAD antes
  e depois. Depois de cada operação o diagnóstico e o status são lidos de novo.

### Erros e autenticação

Códigos novos: `AUTH_FAILED`, `PERMISSION_DENIED`, `NETWORK_ERROR`, `TLS_ERROR`, `REMOTE_NOT_FOUND`,
`REMOTE_BRANCH_MISSING`, `PUSH_REJECTED`, `MERGE_CONFLICT`. A classificação usa o código de saída e evidências do Git
(linhas `--porcelain` do push, estado real do repositório para conflitos, padrões do transporte); sem classificação segura,
a mensagem é genérica com "Detalhes técnicos" higienizados (URLs sem credenciais, cabeçalhos `Authorization` ocultos).

- O aplicativo **não** tem fluxo de login: usa o que o Git já tem (Git Credential Manager para HTTPS, chave SSH + agente).
  `GIT_TERMINAL_PROMPT=0` continua ativo e o processo não tem terminal; se o remoto pedir senha, o erro orienta a configurar o
  mecanismo fora do aplicativo (por exemplo, fazer um `git fetch` uma vez no terminal). **Nunca** peça nem cole tokens na
  interface; o aplicativo não grava credenciais.
- SSH em um servidor novo: confirme a chave do host uma vez no terminal (`ssh -T git@<servidor>`).
- O Git executa o que estiver na configuração do repositório (hooks, `credential.helper`, `core.sshCommand`), como no terminal.
  Só sincronize repositórios em que você confia.
- `gh` (GitHub CLI) **não** é usado: fetch/pull/push/diagnóstico dependem apenas do Git. Ele só será considerado para
  recursos exclusivos do GitHub (como pull requests), em fase futura.

### Tempo limite, progresso e concorrência

- Operações de rede têm tempo limite de **5 minutos** (as consultas locais continuam com 15–60 s). Ao esgotar, o processo é
  encerrado e o resultado é "**não confirmado**" (o servidor pode ter concluído): atualize o diagnóstico antes de repetir.
  Processos auxiliares do Git (como o transporte) podem demorar a encerrar.
- Não há progresso percentual (o runner captura a saída só ao final): a interface mostra indicador indeterminado e a etapa
  ("Atualizando referências remotas…", "Enviando commits…"). **Não há cancelamento** nesta fase.
- Fetch, pull e push usam a trava por repositório (`lock.rs`) e o `runExclusive` do frontend; cliques repetidos e respostas de
  outro repositório/tentativa são descartados (`lib/syncOperation.ts`).

### Testes da Fase 3

- Rust: `cargo test --manifest-path src-tauri/Cargo.toml --workspace` (`git-core/tests/phase3.rs` usa remotos bare locais;
  sem internet nem credenciais; a configuração global do Git é isolada). O teste de conexão recusada usa `127.0.0.1:1`.
  Falhas de autenticação/TLS reais não são reproduzíveis sem servidor: a classificação é coberta por testes unitários
  em `remote.rs` com saídas reais do Git.
- Frontend: `npm test` (`lib/sync.test.ts`, `lib/syncOperation.test.ts`). O projeto não tem testes de componentes React; a
  lógica que importa (bloqueios, diagnóstico, descarte de respostas obsoletas, cliques repetidos) está em funções puras.

### Validação manual da Fase 3 (Windows)

1. Crie uma pasta descartável e um remoto local: `git init --bare C:\teste\remoto.git`; `git init C:\teste\meu` (identidade
   configurada, um commit, `git remote add origin C:\teste\remoto.git`).
2. **Sem upstream**: abra `meu` → *Sincronização*: "Sem upstream"; pull/push desabilitados com o motivo. No terminal:
   `git push -u origin main`; atualize o diagnóstico: "Possivelmente desatualizada" ou "Sincronizada" conforme a idade.
3. **Push**: crie uma branch (`feature`), publique (`git push -u origin feature`), faça um commit pelo aplicativo → "Adiantada"
   → *Enviar commits* → confira o destino na confirmação → confirme. Verifique com `git -C C:\teste\remoto.git log feature`.
   Em `main`, a confirmação exige marcar a caixa extra.
4. **Fetch/Pull**: clone o remoto em outra pasta, faça commit e `push` por lá. No aplicativo, *Atualizar referências (fetch)*
   → "Atrasada" (nada foi integrado) → *Baixar e integrar (pull)* → HEAD avança. Teste também com um arquivo modificado: o pull
   é bloqueado e lista o arquivo.
5. **Push rejeitado**: faça commits nos dois clones sem fetch e envie do outro clone primeiro; no aplicativo o push é recusado
   com "Nada foi enviado". Depois do fetch, o estado vira "Divergente" e o push fica bloqueado.
6. **Falha de autenticação/rede**: `git remote set-url origin https://127.0.0.1:1/x.git` → fetch mostra erro de rede e o
   estado vira "Remoto não verificável". Para autenticação, use uma URL HTTPS privada sem credenciais salvas: o erro orienta a
   configurar o Git Credential Manager ou SSH fora do aplicativo e nenhuma senha é pedida na interface.

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

merge manual entre branches, importação de ZIP, resolução de conflitos, pull requests, sistema próprio de autenticação,
instalador/publicação.
A navegação lateral (`NavId`) e `App.tsx` já têm o ponto de extensão.
