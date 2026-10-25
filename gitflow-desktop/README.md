# GitFlow Desktop (nome provisório) — Fase 1

Aplicativo desktop (Tauri + React + TypeScript + Rust) para **inspecionar repositórios Git** sem digitar comandos.
A Fase 1 é **somente leitura**: seleciona uma pasta, valida o repositório e mostra branch, estado, arquivos
alterados e diffs. Nada é escrito no repositório.

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
  services/gitService.ts     ÚNICA fronteira com o backend (invoke dos 4 comandos)
  services/folderPicker.ts   Seletor de pastas nativo (plugin dialog)
  lib/                       Lógica pura e testada (agrupamento, rótulos, erros)
  hooks/                     Estado: repositório aberto, status e diff (anti-resposta obsoleta)
  components/                layout · changes · diff · welcome · common
src-tauri/
  src/main.rs, commands.rs   4 comandos Tauri finos (async + spawn_blocking)
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

Comandos nativos (todos somente leitura): `check_git`, `open_repository`, `get_repository_status`, `get_file_diff`.

### Segurança

- Git é executado **sem shell**, com argumentos separados; a interface nunca envia comandos, apenas caminhos.
- Todo comando de leitura revalida que `repoPath` é **a raiz** de um repositório; caminhos de arquivo são
  rejeitados se vazios, absolutos, com `..` ou NUL, e interpretados literalmente (`--literal-pathspecs`).
- Ambiente endurecido: `core.fsmonitor=false` (a configuração de um repositório não executa programas),
  `--no-ext-diff`, `--no-textconv`, `--no-optional-locks`, `GIT_TERMINAL_PROMPT=0`, variáveis `GIT_DIR`/`GIT_INDEX_FILE`… removidas.
- Tempo limite e limite de saída em toda chamada; diff e linhas muito grandes são truncados.
- Capabilities do Tauri concedem apenas `dialog:allow-open` e os 4 comandos. CSP restritiva.
- Nenhum log de conteúdo de arquivos; mensagens de erro técnicas ficam em "Detalhes técnicos".

### Decisões relevantes

- **Crate separado `gitflow-git`**: a lógica de Git é testável sem Tauri e reaproveitada pelas próximas fases.
- **`status --porcelain=v2 -z`**: formato estável para programas; nomes com espaços/acentos não precisam de escape.
- **Estado preparado × não preparado** são mostrados em grupos separados (cada um tem diff próprio).
- **Respostas obsoletas** são descartadas por número de sequência e conferência do `repoPath` retornado.
- **Dependências**: apenas React, `@tauri-apps/api`, `@tauri-apps/plugin-dialog` e Tailwind. O diff usa uma
  tabela própria (sem biblioteca extra) por ser somente leitura e simples.

## Fora do escopo (próximas fases)

Troca/criação de branches, commits, pull/push, merge, importação de ZIP, autenticação, pull requests,
instalador/publicação. A navegação lateral (`NavId`) e `App.tsx` já têm o ponto de extensão.
