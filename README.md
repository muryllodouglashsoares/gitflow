# GitFlow Desktop (nome provisório) — Fase 4

Aplicativo desktop (Tauri + React + TypeScript + Rust) para **operar repositórios Git locais** sem digitar comandos.

- **Fase 1** (somente leitura): seleciona uma pasta, valida o repositório e mostra branch, estado, arquivos alterados e diffs.
- **Fase 2** (operações locais): lista, cria e troca branches; seleciona explicitamente os arquivos de um commit,
  revisa o diff do que será registrado e cria o commit local, mostrando o resultado real do Git.
- **Fase 3** (sincronização remota): diagnóstico de remotos/upstream (sincronizada, adiantada, atrasada, divergente) e
  as operações **fetch**, **pull** e **push** pela interface, com confirmação do destino exato, bloqueios de segurança e
  o resultado real do Git.
- **Fase 4** (importação de ZIP): importa uma atualização de código a partir de um `.zip` por um fluxo revisável — inspeção
  sem extrair, prévia do plano, branch de trabalho dedicada, gravação controlada, diff real, commit só dos arquivos da
  importação e push em etapa separada (reaproveitando o push da Fase 3). **Não há** merge manual, pull requests nem
  armazenamento de credenciais (fase 5 em diante).

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

## Fase 4 — Importação de ZIP

Aba **Importar ZIP**: transforma um ZIP **não confiável** em um plano de alterações validado, revisável e restrito ao
repositório aberto. Nenhuma etapa avança sozinha: não existe um botão que faça tudo de ponta a ponta.

### Fluxo

1. **Seleção** — escolha o `.zip` (seletor nativo), confira o repositório de destino (o repositório aberto) e a
   **branch-base** (por padrão, a branch ativa).
2. **Inspeção** (somente leitura) — o ZIP é lido **sem extrair nada**. A tela mostra novos, substituições, idênticos,
   bloqueados e ignorados, com tamanho e SHA-256 do pacote. Se o ZIP tiver uma pasta-raiz, o aplicativo **sugere** a pasta
   do pacote, mas não remove nenhum nível sozinho: você escolhe em “Pasta do pacote” (a inspeção é refeita na hora).
   Arquivos ocultos legítimos (`.gitignore`, `.github/…`) são preservados.
3. **Plano** — cada arquivo é comparado (SHA-256) com o conteúdo atual: *Novo*, *Substitui*, *Idêntico* (nunca é
   regravado) ou *Bloqueado* (com o motivo). Você pode **desmarcar** arquivos para não importá-los. Substituições exigem uma
   confirmação explícita, com a lista dos arquivos afetados.
4. **Branch de trabalho** — informe (ou aceite a sugestão editável `importacao/<zip>-AAAAMMDD`) o nome da nova branch.
   Ela é criada a partir da base e ativada **antes** de qualquer gravação. Uma única confirmação cobre o plano e a branch,
   e o resumo mostra repositório, base → nova branch e quantos arquivos serão gravados.
5. **Importação** — o backend recalcula o plano e só grava se ele ainda for **idêntico** ao confirmado. Antes de tocar o
   repositório, os arquivos são extraídos para uma área privada, conferidos contra o hash do plano e, no caso de
   substituições, copiados para um backup. Depois são aplicados um a um.
6. **Diff e commit** — a tela reaproveita a seleção, a revisão de diff e a confirmação da Fase 2, mas alimentadas **só**
   com os arquivos gravados pela importação. O backend também recusa qualquer arquivo que não pertença à operação.
   Arquivos fora da importação aparecem apenas como um aviso e não entram no commit.
7. **Push** — etapa separada, com confirmação própria que mostra a branch local, `remoto/branch`, o commit e quantos
   commits serão enviados. Usa o `push_branch` da Fase 3 (sem `--force`, sem criar upstream, só a branch de trabalho).
   Destino `main`/`master` exige uma caixa extra. Você também pode **concluir sem enviar**.
8. **Relatório** — pacote, branch, arquivos importados, commit e se houve envio. O “Registro da operação” lista cada
   etapa (início, resultado e detalhe).

A importação **nunca exclui** arquivos: o que não está no ZIP permanece no repositório.

### Segurança do ZIP

O arquivo é entrada não confiável; o backend (Rust) valida tudo, e a interface não é uma fronteira de confiança.

- **Caminhos** recusados: absolutos, UNC, com unidade (`C:`), com `..`/`.` ou componentes vazios, com barra invertida
  (`\`, ambígua entre sistemas), caracteres de controle ou de direção de texto Unicode, nomes inválidos/reservados do
  Windows (`CON`, `NUL`, `a:b`, `x?`, terminados em ponto/espaço…), a pasta `.git` e caminhos longos demais (limite de
  240 bytes no pacote e `MAX_PATH` do Windows no destino).
- **Colisões**: entradas **duplicadas** (lidas direto do diretório central, pois o crate `zip` as colapsa em silêncio),
  nomes que colidem por maiúsculas/minúsculas ou por Unicode equivalente (NFC) e arquivo × pasta com o mesmo caminho.
- **Tipos**: links simbólicos, dispositivos e outras entradas especiais são **bloqueados** (nunca materializados); entradas
  criptografadas e métodos de compressão não suportados também.
- **Limites** (`ImportLimits`, em `git-core/src/import/mod.rs`): ZIP de até 1 GiB, 5.000 entradas, 50 MiB por arquivo,
  500 MiB descomprimidos no total e taxa de compressão máxima de 200× (para arquivos acima de 1 MiB). Os tamanhos são
  conferidos contra o conteúdo **realmente descomprimido**, não só contra o cabeçalho. **ZIP64 não é suportado** (os limites
  o tornam desnecessário e um ZIP64 forjado poderia esconder entradas).
- **Gravação**: cada caminho é revalidado e montado por componentes (nunca por prefixo de texto); nenhum componente
  existente pode ser link simbólico; arquivos ignorados pelo `.gitignore` são bloqueados (o Git não os veria); nada é
  sobrescrito fora do plano; antes de cada substituição o hash do arquivo atual é conferido de novo.
- Nenhum conteúdo do ZIP é executado.

Qualquer entrada bloqueada impede a importação do pacote inteiro, com o caminho e o motivo.

### Estado do repositório e falhas

O plano só pode ser aplicado com **árvore limpa** (sem alterações, sem arquivos não rastreados), sem merge/rebase em
andamento, sem conflitos, sem HEAD desanexado e com a branch ativa igual à branch-base — alterações preexistentes nunca
são misturadas, escondidas nem descartadas. Se a base escolhida não é a branch ativa, o plano é bloqueado e a tela pede
que você troque para ela antes (aba Branches).

Cada etapa é uma transição de uma **máquina de estados** (`src/lib/importFlow.ts`): eventos fora de ordem, cliques repetidos
e respostas de outro repositório ou de uma tentativa antiga são ignorados. Em falha, o fluxo para e mostra a etapa, o que
já foi concluído e o que verificar:

| Situação | Resultado |
| --- | --- |
| ZIP inválido, corrompido, vazio ou bloqueado | Nada é criado; mensagem com o motivo |
| Falha ao criar/ativar a branch | Nenhum arquivo gravado; orienta conferir a aba Branches |
| Plano/ZIP/destino mudou depois da prévia | Recusado (“plano desatualizado”), nada gravado; volte à base e inspecione de novo |
| Falha **no meio** da gravação | Estado **parcial** com o que foi gravado; commit e push ficam bloqueados; ofereça-se **Desfazer** |
| Commit falha | Seleção e mensagem preservadas; nada é commitado |
| Push rejeitado/sem upstream/timeout | O commit local é preservado; a operação continua em “commit concluído”; nada é repetido sozinho; timeout = “não confirmado” |

**Desfazer** (antes do commit) restaura as substituições a partir do backup e remove os arquivos novos **somente se ainda
estiverem exatamente como foram importados**; o que você editou depois é preservado e listado. Se a branch de trabalho já
recebeu commits (por exemplo, você commitou pela aba Alterações), o desfazer é recusado: aí o caminho é `git revert`.

**Retomada**: a operação fica registrada em `.git/gitflow-import/<id>/journal.json` (sem segredos nem conteúdo de
arquivos; backups e temporários ficam ao lado). Ao abrir o repositório, uma operação pendente é retomada na etapa correta
(parcial → desfazer; aplicada → revisão/commit; commitada → push) **sem repetir etapas** e sem criar um segundo commit.
“Concluir”/“Desfazer” bem-sucedidos limpam os temporários e backups.

### Limitações conhecidas

- **Push exige upstream já configurado.** A branch de trabalho é nova e não tem upstream; o aplicativo não o inventa nem
  o define. Para publicar pelo app, configure antes `git push -u origin <branch-de-trabalho>` no terminal (por exemplo,
  logo depois de criada) ou envie depois do commit pelo terminal. Sem upstream, o app explica o motivo e preserva o commit.
- A comparação “idêntico × substitui” é **byte a byte**: diferenças de fim de linha (CRLF × LF) contam como diferença.
- Nomes de arquivos com `\` (alguns ZIPs gerados no Windows PowerShell 5.1) são bloqueados; gere o pacote novamente
  (por exemplo, com `tar -a -c -f` ou o PowerShell 7).
- Não há cancelamento durante gravação, commit ou push (só da inspeção); não há porcentagem de progresso (o indicador é
  indeterminado). Não há checagem prévia de espaço em disco: um erro de escrita durante a extração ocorre **antes** de tocar
  o repositório.
- Permissões de arquivo não são definidas (substituições preservam as do arquivo existente em sistemas Unix).
- Se o aplicativo for encerrado à força no instante de uma gravação, pode sobrar um `.gitflow-import-*.tmp` ao lado do
  destino; a operação é retomada como parcial e o **Desfazer** cuida do restante.
- Ao desfazer, pastas que ficaram vazias no caminho dos arquivos removidos também são removidas.
- O frontend não tem testes de interação com componentes React (o Vitest do projeto roda em Node, sem DOM): a lógica do
  fluxo, os bloqueios e o descarte de respostas obsoletas estão em funções puras testadas, e há um teste de renderização
  de todas as etapas da tela; cliques, foco e o seletor nativo de arquivos só são validados pelo roteiro manual abaixo.

### Testes da Fase 4

- Rust: `git-core/tests/phase4.rs` (42 testes, repositórios e remotos bare temporários, ZIPs montados em memória e
  adulterados por patch de bytes) e testes unitários em `git-core/src/import/`. Cobrem: ZIP válido/corrompido/vazio/não-ZIP,
  duplicatas, ZIP64 forjado, traversal e caminhos absolutos, separadores alternativos, nomes reservados, colisões, links
  simbólicos, limites de tamanho/quantidade/compressão, pasta-raiz, `.gitignore`, destino com link simbólico, alterações
  locais, merge em andamento, branch inválida/existente/igual à base, plano e ZIP desatualizados, sobrescrita sem confirmação,
  exclusões, ausência de exclusão de arquivos, falha no meio da gravação, desfazer (inclusive com edições do usuário e após
  commit externo), commit isolado e sem duplicação, push confirmado/rejeitado/sem upstream/destino errado, trava de
  concorrência, identificadores de operação malformados e o **contrato JSON** com os tipos TypeScript.
- Frontend: `src/lib/importFlow.test.ts` (transições permitidas, ordem das etapas, respostas obsoletas, cliques repetidos,
  cancelamento, falhas, retomada, confirmações, exclusões, destino do push, nome da branch) e
  `src/components/import/ImportView.test.tsx` (renderização no servidor de cada etapa, sem exceções nem valores quebrados).
- `npm test`, `npm run test:rust`, `npm run typecheck` e `npm run build` fazem parte da verificação.

### Validação manual da Fase 4 (Windows, repositório descartável)

1. Crie `C:\teste\meu` com `git init`, identidade configurada e um commit com `src\app.txt` e `keep.txt`. Para o push, crie
   também `git init --bare C:\teste\remoto.git`, `git remote add origin C:\teste\remoto.git` e `git push -u origin main`.
2. Monte `C:\teste\pacote.zip` com `novo.txt`, `src\app.txt` (conteúdo diferente) e um arquivo idêntico a um existente.
   Abra `meu` → **Importar ZIP** → *Escolher ZIP…* → *Inspecionar ZIP*. Confira: 1 novo, 1 substitui, 1 idêntico, nada gravado.
3. Marque a confirmação de sobrescrita e a do plano; troque o nome da branch para um inválido (`a b`) e veja o bloqueio;
   volte ao nome válido → *Criar branch e importar*. Confira no terminal: `git branch` (nova branch ativa),
   `git status` (só os arquivos do plano) e que `keep.txt` continua lá.
4. **Commit**: marque os arquivos, *Revisar e commitar…*, confira o diff real, escreva a mensagem, confirme.
   Edite `keep.txt` no editor e veja que ele não aparece como arquivo da importação nem entra no commit.
5. **Push**: sem upstream, a tela explica e não envia. No terminal: `git push -u origin <branch>`; volte ao app (o diagnóstico
   atualiza) → *Enviar ao remoto* exige a confirmação; confira `git -C C:\teste\remoto.git log <branch>`.
6. **Bloqueios**: um ZIP com `..\fora.txt`, `con.txt` ou dois arquivos que só diferem em maiúsculas é bloqueado, com o
   caminho e o motivo, e nada é gravado. Com um arquivo modificado no repositório, o plano é bloqueado listando o motivo.
7. **Desfazer**: repita a importação, pare antes do commit e use *Desfazer a importação*; confira `git status` limpo e
   `src\app.txt` restaurado. Edite um arquivo importado antes de desfazer: ele é preservado e listado.
8. **Retomada**: importe, feche o aplicativo antes de commitar e reabra o repositório: a operação volta na etapa de revisão.

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

merge manual entre branches e proteção da `main` (Fase 5), resolução de conflitos, pull requests, sistema próprio de
autenticação, exclusão de arquivos por importação (se vier a existir, será um fluxo separado, com prévia e confirmação),
instalador/publicação.
A navegação lateral (`NavId`) e `App.tsx` já têm o ponto de extensão.
