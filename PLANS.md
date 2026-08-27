# Atualização dos SDKs dos workers

## Purpose / Big Picture

Atualizar os SDKs usados pelos workers para a release
[`iii-alpha/v0.22.1-alpha.25`](https://github.com/iii-hq/iii/releases/tag/iii-alpha%2Fv0.22.1-alpha.25).
Essa release contém a correção que faz `III_WORKER_NAME` ter precedência sobre o
nome definido pelo código do worker.

Fluxo esperado:

```text
compose key: foo
worker code: batatinha
       |
       v
III_WORKER_NAME=foo
       |
       v
SDK registra: foo
       |
       v
Compose conclui readiness
```

Versões alvo:

| Ecossistema | Versão |
| --- | --- |
| Rust | `0.22.1-alpha.25` |
| Node.js | `0.22.1-alpha.25` |
| Python | `0.22.1a25` |
| Go | `v0.22.1-alpha.25` |
| Engine | tag `iii-alpha/v0.22.1-alpha.25` |

## Progress

- [x] 2026-08-27 — Confirmar a release e as versões publicadas.
- [x] 2026-08-27 — Mapear manifestos, lockfiles e pins de CI.
- [x] 2026-08-27 — Confirmar que a branch `feat/update-sdk` está limpa e alinhada com `main`.
- [x] 2026-08-27 — Atualizar os manifestos de produção.
- [x] 2026-08-27 — Atualizar as regras centrais e os pins da engine em CI.
- [x] 2026-08-27 — Regenerar os lockfiles com mudança mínima.
- [x] 2026-08-27 — Corrigir incompatibilidades reais de API e de topologia da engine.
- [x] 2026-08-27 — Executar formato, lint, build e testes por ecossistema.
- [x] 2026-08-27 — Executar o smoke test do nome gerenciado pelo Compose.
- [x] 2026-08-27 — Fazer a auditoria final do diff e dos pins.

## Scope

### Rust

- Atualizar 63 `Cargo.toml` de `iii-sdk = "=0.23.0-rc.2"` para
  `iii-sdk = "=0.22.1-alpha.25"`.
- Atualizar os 19 pins diretos de `iii-helpers` para `=0.22.1-alpha.25`.
- Atualizar os 63 `Cargo.lock` correspondentes.
- Não há pin direto de `iii-observability` no estado atual.
- Todos os pins exatos que participam do mesmo grafo devem mudar juntos.

### Node.js

- Atualizar dez manifestos de produção.
- Usar `iii-sdk@0.22.1-alpha.25`.
- Usar `iii-browser-sdk@0.22.1-alpha.25` em `console/web`.
- Atualizar os helpers transitivos para `@iii-dev/helpers@0.22.1-alpha.25`.
- Atualizar nove lockfiles:
  - `pnpm-lock.yaml` da raiz;
  - `compose-ui/pnpm-lock.yaml`;
  - `vscode/pnpm-lock.yaml`;
  - `openwiki/pnpm-lock.yaml`;
  - `cursor/pnpm-lock.yaml`;
  - `claude-code/pnpm-lock.yaml`;
  - `opengantry/pnpm-lock.yaml`;
  - `pi/pnpm-lock.yaml`;
  - `opencode/pnpm-lock.yaml`.

### Python

- Atualizar `hermes/pyproject.toml` e `scrapling/pyproject.toml`.
- Usar `iii-sdk==0.22.1a25`.
- Usar `iii-helpers==0.22.1a25`.
- Não há `uv.lock` para atualizar nesses dois workers.

### Go

- Nenhum `go.mod` usa o SDK Go nesta pasta.
- Não há alteração Go planejada.
- `provider-opencode-go` é um worker Rust apesar do nome.

### Regras centrais e CI

- Atualizar em `pnpm-workspace.yaml`:
  - `@iii-dev/helpers@0.22.1-alpha.25`;
  - `iii-browser-sdk@0.22.1-alpha.25`;
  - `iii-sdk@0.22.1-alpha.25`.
- Atualizar as asserções de versão em
  `.github/scripts/tests/test_released_worker_sdk_runtime.py`.
- Atualizar as três instalações de engine em `.github/workflows/ci.yml`.
- Para a engine alpha, usar o tag exato:

```text
III_RELEASE_TAG=iii-alpha/v0.22.1-alpha.25
```

- Remover o uso combinado de `VERSION=0.23.0-rc.2` e `--rc` nesses pontos.

## Execution Plan

### 1. Atualizar os manifestos

1. Atualizar todos os pins Rust de produção em uma única etapa.
2. Atualizar os dez manifestos Node.js de produção.
3. Atualizar os dois manifestos Python.
4. Atualizar `pnpm-workspace.yaml`, os testes de versão e os três pins da engine.
5. Verificar que nenhum pin de produção antigo ficou no repositório.

### 2. Regenerar os lockfiles

1. Atualizar cada `Cargo.lock` a partir do seu `Cargo.toml`.
2. Limitar a resolução a `iii-sdk`, `iii-helpers` e dependências que precisem
   mudar por causa desses pacotes.
3. Executar `pnpm install --lockfile-only` na raiz e nos oito projetos Node.js
   isolados, incluindo `compose-ui`.
4. Revisar o diff para remover atualização de dependência não relacionada.
5. Confirmar que instalações com `--locked` e `--frozen-lockfile` funcionam.

### 3. Tratar compatibilidade de API

A release alvo está 24 commits à frente de `iii/v0.23.0-rc.2`, apesar de usar
um número SemVer menor. A expectativa é que a maior parte da alteração seja
mecânica. Mesmo assim, cada build deve ser tratado como uma verificação de API.

```text
manifestos
    |
    v
lockfiles
    |
    v
builds e testes
    |
    v
ajustes mínimos, somente se necessários
```

Se houver erro de compilação:

1. Confirmar que o erro vem da troca de SDK.
2. Aplicar a menor mudança possível no worker afetado.
3. Preservar o comportamento existente.
4. Adicionar ou ajustar teste somente quando houver mudança de comportamento.

### 4. Validar Rust

Executar Rust diretamente no host, conforme as regras do projeto.

Para cada worker e crate afetado:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

Exceção existente para `browser`:

```bash
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Também executar:

- integração real do `llm-router`;
- contratos herméticos dos providers;
- boot smoke dos workers Rust com a engine alpha exata.

### 5. Validar Node.js

Usar `pnpm`, nunca `npm`, e executar em container por padrão.

Para os dez projetos afetados:

1. Instalar com lockfile congelado.
2. Executar build quando o projeto tiver script de build.
3. Executar Biome.
4. Executar os testes existentes.

Confirmar que o `pnpm-lock.yaml` resolve:

- `iii-sdk@0.22.1-alpha.25`;
- `iii-browser-sdk@0.22.1-alpha.25`;
- `@iii-dev/helpers@0.22.1-alpha.25`.

### 6. Validar Python

Executar em container por padrão.

Para `hermes` e `scrapling`:

1. Instalar o projeto com as dependências de desenvolvimento.
2. Executar Ruff lint.
3. Executar Ruff format check.
4. Executar Pytest.
5. Confirmar que o resolvedor instala `0.22.1a25`, sem usar cópia vendorizada.

### 7. Validar scripts e workflows

- Executar `pytest .github/scripts/tests/ -v`.
- Validar `.github/workflows/ci.yml` com Actionlint.
- Confirmar que as três instalações usam `III_RELEASE_TAG`.
- Confirmar que nenhum comando ainda seleciona o canal `--rc` para essa engine.

### 8. Smoke test do problema original

Criar ou usar um cenário temporário com:

```yaml
containers:
  foo:
    worker: path://../batatinha
```

O código do worker deve continuar usando um nome explícito diferente, por
exemplo `batatinha`. O teste deve confirmar:

1. Compose injeta `III_WORKER_NAME=foo`.
2. O SDK registra o worker como `foo`.
3. A engine lista o worker como `foo`.
4. O Compose não informa timeout de readiness.
5. O encerramento do projeto funciona sem processo órfão.

### 9. Auditoria final

1. Procurar pins de produção antigos:
   - `0.23.0-rc.2`;
   - `0.23.0rc2`.
2. Confirmar as versões alvo em manifestos e lockfiles.
3. Executar `git diff --check`.
4. Revisar o diff por ecossistema.
5. Confirmar que não existem artefatos temporários ou arquivos gerados fora do escopo.
6. Confirmar CI verde antes de marcar o trabalho como concluído.

## Exclusions

Manter inicialmente os exemplos e harnesses E2E antigos que usam `0.11.x`,
`0.20.0`, `0.21.8` ou `0.22.1`. Eles não usam `0.23.0-rc.2` e não definem a
versão dos workers publicados.

Arquivos principais dessa exclusão:

- `image-resize/example/package.json`;
- `browser/tests/e2e/workers/harness/package.json`;
- `database/tests/e2e/workers/harness/package.json`;
- `storage/tests/e2e/workers/harness/package.json`;
- `shell/tests/e2e/workers/harness/package.json`;
- `rbac-proxy/tests/e2e/workers/harness/package.json`;
- `code-runner/tests/e2e/workers/harness/package.json`;
- `harness/tests/e2e/Cargo.toml`;
- `harness/tests/integration/Cargo.toml`.

Se esses projetos também precisarem usar a nova release, tratar como uma etapa
separada. A migração de `0.11.x` para `0.22.1-alpha.25` pode exigir mudanças de
API que não pertencem ao bump dos workers publicados.

## Surprises & Discoveries

- A versão alvo tem número SemVer menor que `0.23.0-rc.2`, mas foi publicada
  depois e está 24 commits à frente na história do repositório `iii`.
- A release alvo contém a mudança `fix(sdk): prefer compose-managed worker names`.
- O repositório usa pins Rust exatos. Uma atualização parcial pode impedir a
  resolução do grafo de dependências.
- O pnpm bloqueia scripts de instalação por padrão e usa
  `minimumReleaseAgeExclude`; as exceções precisam acompanhar a nova release.
- A engine alpha deve ser instalada pelo tag completo. `VERSION` com `--rc`
  seleciona outro canal.
- Não há uso direto do SDK Go nesta pasta.
- Não há pin direto de `iii-observability` nos manifestos atuais.
- `SendOptions` passou a exigir os campos opcionais `agent` e `skills`; o
  worker `eval` preserva o comportamento anterior com ambos definidos como
  `None`.
- A engine alvo não aceita mais `iii-pubsub` e `iii-state` na lista interna de
  workers. Os testes com engine real agora iniciam o worker `state` separado e
  esperam `state::get` antes de executar os contratos.
- O `build.rs` do worker `state` exige que a UI injetada exista. Os jobs de
  integração agora configuram Node e pnpm e geram `state/ui/dist` antes do
  `cargo build`.
- O `console/web` tem duas falhas de teste e erros de lint já presentes no
  código-base: as duas falhas ainda esperam a classe de layout `left-full`.
- O `compose-ui`, adicionado a `main` durante a execução, tem um script de lint
  que chama Biome sem declarar o pacote. Typecheck, build e 21 testes passam.
- O `iii-directory` tem dez cenários BDD com expectativas antigas já
  incompatíveis com a implementação atual. Os 398 testes unitários passaram.

## Decision Log

- 2026-08-27 — Usar pins exatos em todos os ecossistemas para reproduzir a
  release solicitada.
- 2026-08-27 — Atualizar todos os 63 grafos Rust juntos para evitar conflito
  entre pins exatos.
- 2026-08-27 — Usar `III_RELEASE_TAG=iii-alpha/v0.22.1-alpha.25` nos testes de
  integração e boot smoke.
- 2026-08-27 — Excluir inicialmente fixtures históricos e exemplos que não
  participam da linha atual de release.
- 2026-08-27 — Tratar alterações de código como exceção guiada por erro de
  compilação ou teste, não como parte automática do bump.
- 2026-08-27 — Iniciar o worker `state` dos testes de integração como processo
  separado, porque a engine alvo externalizou esse serviço.
- 2026-08-27 — Pré-compilar a UI do worker `state` nos jobs que criam o binário
  separado, para que a compilação seja reproduzível em um runner limpo.

## Outcomes & Retrospective

- 154 arquivos foram adicionados ou alterados, incluindo este registro de
  execução. A maior parte do diff é a troca exata de versões em 63 grafos
  Cargo e nove lockfiles pnpm.
- Os 63 manifestos e lockfiles Rust resolvem `iii-sdk@0.22.1-alpha.25`; os 19
  pins diretos de `iii-helpers` usam a mesma versão.
- Os dez manifestos Node.js, os dois manifestos Python, as regras centrais e
  os três pins da engine usam as versões alvo.
- Rust: formato e Clippy passaram nos 63 crates. As suítes passaram, exceto os
  dez cenários BDD antigos do `iii-directory`; os 398 testes unitários desse
  crate passaram.
- Integrações Rust: 29 testes do `llm-router` e os dez contratos herméticos de
  providers passaram com a engine `0.22.1-alpha.25` e o worker `state`
  separado.
- Node.js: instalações congeladas e builds passaram nos nove projetos
  originais. O `compose-ui`, adicionado a `main` durante a execução, teve o
  manifesto e o lockfile atualizados após o rebase; typecheck, build e 21
  testes também passaram. Foram 2.550 testes verdes. As duas falhas antigas de
  `console/web`, o lint antigo desse projeto e a dependência Biome ausente no
  `compose-ui` ficaram fora do escopo.
- Python: Ruff lint, Ruff format e 131 testes passaram para `hermes` e
  `scrapling`, com as versões `0.22.1a25` instaladas.
- Scripts e workflows: 255 testes, três subtestes e Actionlint passaram.
- Os jobs de integração preparam a UI embutida antes de compilar o worker
  `state`; o mesmo fluxo passou localmente com lockfile congelado.
- O smoke do Compose registrou o worker como `foo`, concluiu readiness em
  205 ms e encerrou o worker sem processo órfão, mesmo com o código declarando
  o nome `state`.
- Exemplos e harnesses históricos nas versões listadas em `Exclusions`
  permaneceram sem alteração.

## Validation and Acceptance

O trabalho estará concluído quando:

- todos os 63 manifestos Rust de produção usarem `=0.22.1-alpha.25`;
- os 19 pins Rust de `iii-helpers` usarem `=0.22.1-alpha.25`;
- os dez manifestos Node.js usarem `0.22.1-alpha.25`;
- `console/web` usar `iii-browser-sdk@0.22.1-alpha.25`;
- `hermes` e `scrapling` usarem `0.22.1a25`;
- os 63 `Cargo.lock` e os nove lockfiles pnpm estiverem consistentes;
- os três pontos de CI instalarem a engine pelo tag alpha exato;
- os testes das regras de versão estiverem atualizados e verdes;
- formato, lint, builds e testes aplicáveis passarem;
- o smoke test registrar `foo`, e não `batatinha`;
- não houver atualização de dependência fora do escopo;
- o worktree estiver limpo após commit e push.
