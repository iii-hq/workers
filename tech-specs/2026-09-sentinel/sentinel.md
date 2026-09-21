# sentinel

Worker prefix: `sentinel::*`

## Definição

`sentinel` é o monitor de erros do iii: um worker que observa a telemetria do
engine, agrupa as falhas por causa provável, guarda a evidência antes que ela
evapore e entrega ao harness tudo que uma investigação precisa. É o Sentry da
stack — mas construído sobre o que a stack já emite (spans OTel, logs OTel,
triggers do engine), sem SDK de captura em nenhum worker.

Ele responde três perguntas que hoje exigem alguém garimpando a página de
traces:

1. **O que está quebrando, e quanto?** — grupos com contagem, sessões
   afetadas, primeira/última ocorrência, versão do worker.
2. **Isso é novo ou voltou?** — ciclo de vida com regressão explícita.
3. **Por quê?** — uma investigação do harness aberta ao lado da página, que o
   usuário acompanha e direciona, com a evidência congelada e o código-fonte
   na mão, e na qual o agente **grava** a causa provável chamando uma função
   do próprio Sentinel.

Regra de escopo, herdada do harness: **se uma preocupação cresce lógica
própria, vira worker irmão**. O sentinel agrupa, persiste e despacha. Ele não
corrige código, não abre issue, não notifica canal — esses são irmãos que
assinam `sentinel::group-changed` (ver [Fora de escopo](#fora-de-escopo-do-v1)).

## O que ele liga

```mermaid
flowchart LR
  otlp["todo worker · SDK OTel<br/>(inclusive o Sentinel)"]
  engine["engine · observability<br/>trace store (10 000 spans) · log store<br/>triggers trace / log · RPCs"]
  registry["engine::workers::list<br/>engine::functions::list<br/>(namespace · versão · dono da função)"]
  sentinel["sentinel"]
  queue["queue<br/>sentinel-ingest · breaker"]
  db["database<br/>(SQLite primary)"]
  storage["storage<br/>(cópia JSON, opcional)"]
  router["llm-router<br/>complete (triagem) · decide (decisões)"]
  harness["harness<br/>sessão sentinel-inv-* · kind: automation"]
  ide["ide (coder::*)<br/>somente leitura"]
  repos["repositórios mapeados<br/>path local"]
  console["console (ade)<br/>página + sessão ao lado"]
  siblings["irmãos<br/>(alertas, Linear, PR)"]

  otlp -->|"spans · logs"| engine
  engine -->|"trigger trace {status: error} → {trace_ids}<br/>trigger log {level: error} → StoredLog"| sentinel
  sentinel -->|"traces::list · spans · tree<br/>logs::list"| engine
  sentinel -->|"dono do function_id · versão"| registry
  sentinel -->|"enfileira · consome"| queue
  sentinel -->|"grupos · ocorrências · sessões · buckets<br/>investigações · diagnósticos · decisões · arestas"| db
  sentinel -.->|"cópia JSON"| storage
  sentinel -.->|"router::complete · router::decide<br/>(tiers opcionais)"| router
  sentinel -->|"harness::send: 1ª passada (texto, somente leitura)"| harness
  harness -->|"sentinel::diagnosis::record { group_id, DiagnosisV1 }<br/>(ƒ chamada pelo agente) · harness::turn-completed"| sentinel
  harness -->|"router::chat"| router
  harness -->|"coder::read-file …"| ide
  ide -->|"fs_scope"| repos
  console <-->|"sentinel::* · group-changed"| sentinel
  console <-->|"transcript · composer (steering)"| harness
  sentinel -.->|"seus próprios spans e logs<br/>(cortados no ingest: raiz = sentinel)"| otlp
  sentinel -.->|"sentinel::group-changed"| siblings
```

O sentinel é consumidor dos mesmos RPCs que a página de traces do console
usa (`engine::traces::list`, `engine::traces::tree`, `engine::logs::list`) e
das assinaturas push que o engine já oferece. Não há mudança no engine para o
v1.

## A restrição que define a arquitetura

O store de telemetria do engine é **em memória e com teto**: `memory_max_spans`
(padrão 10 000), `logs_max_count` (padrão 1 000) e `logs_retention_seconds`
(padrão 3 600). Sob carga real — quatro turnos de harness em paralelo — 10 000
spans equivalem a minutos de histórico.

Consequência: **a evidência tem de ser capturada no ingest, não na ação**.
Quando alguém abrir a UI e mandar investigar, o trace de origem provavelmente
já saiu do ring buffer. O sentinel congela a árvore de spans e os logs
correlacionados no momento em que vê o erro; a UI e o harness leem a cópia do
sentinel, e só recorrem ao engine quando o trace ainda existe (deep link para a
página de traces).

## Vocabulário

| Termo | Significado |
|---|---|
| **ErrorEvent** | Uma falha observada, normalizada e agnóstica de fonte: quem (`service_name`, `function_id`), o quê (`exception.type`, mensagem), onde (`trace_id`, `span_id`), quando, em que versão, em qual sessão. É a unidade que os adapters produzem. |
| **Fonte / adapter** | O que transforma um sinal externo em `ErrorEvent`. v1: `trace` (spans com status `error`) e `log` (registros OTel `ERROR`). |
| **Fingerprint** | Hash determinístico que agrupa eventos pela causa provável (ver [Fingerprint](#fingerprint)). |
| **Grupo** | O "issue" do Sentry: um fingerprint, com estado, contadores e evidência. É o que a UI lista e o que se resolve, ignora ou investiga. |
| **Ocorrência** | Uma instância de um grupo — um `ErrorEvent` persistido, com ou sem bundle de evidência. |
| **Evidência** | Snapshot congelado: a árvore de spans do trace, os logs do trace, tags de sessão/turno, versão do worker. |
| **Investigação** | Uma sessão do harness (`kind: automation`) aberta ao lado da página do grupo: começa com a evidência e o código, o usuário acompanha e direciona, e o agente grava um `DiagnosisV1` quando tem uma causa — ou quando o usuário pede. |
| **Primeira passada** | O turno inicial da investigação: roda com a evidência e a política somente-leitura, em **texto livre** — sem contrato de saída, para que o usuário possa intervir e ser respondido em prosa. É o único trecho que corre sem o usuário digitar — mas nunca sem ele ver. |
| **Registro de diagnóstico** | A função `sentinel::diagnosis::record { group_id, diagnosis }`, que o agente chama de dentro da sessão como qualquer outra função iii. É a única escrita que a investigação faz e o único lugar onde um `DiagnosisV1` nasce. |
| **Diagnóstico** | O `DiagnosisV1` que o agente gravou. Uma investigação pode gravar vários — a cada chamada, uma versão; o mais recente vale, todos ficam. O harness nunca "resolve"; resolver é decisão humana. |
| **Triagem** | Passada barata e opcional em todo grupo novo, sem acesso ao código: título, resumo, uma hipótese rotulada como não testada e onde olhar. Nunca um diagnóstico. |
| **Tier de decisões** | Camada opcional que troca heurísticas por decisões calibradas, uma a uma, cada uma com a heurística atrás como fallback obrigatório (ver [Tier de decisões](#tier-de-decisões-opcional)). Desligado por padrão; desligado, o Sentinel é exatamente o que as outras seções descrevem. |
| **Ponto de decisão** | Um lugar da spec onde hoje decide uma constante afinada na mão (janela de junção, span folha, quais bundles reter). Com o tier ligado, cada um vira uma decisão com valor tipado e probabilidade calibrada. |
| **Regressão** | Uma ocorrência nova em um grupo `resolved`. É o estado que justifica o worker existir. |

## Fontes de erro

### O que o engine oferece (confirmado em `iii/engine/src/workers/observability`)

| Mecanismo | Contrato | Nota |
|---|---|---|
| Trigger `trace` | `config: { service_name?, status? }`; entrega `{ trace_ids: string[] }` por janela de ~300 ms, só com os traces cujos spans casaram o filtro | `should_trigger_for_span` (`mod.rs:584`), `fire_trace_triggers` (`mod.rs:2338`). Spans internos (`iii.function.kind=internal`, `engine::*`) nunca disparam. |
| Trigger `log` | `config: { level }` com **match exato** (`error` = só ERROR); entrega o `StoredLog` completo: `trace_id`, `span_id`, `service_name`, `resource`, `body`, `attributes`, `severity_*`. ⚠ O handler o recebe **tipado** (`StoredLogEventV1`, todo campo `#[serde(default)]`) e não como valor livre: a superfície publicada é o que a captura de interface grava, e um request sem tipo ali não diz nada a quem lê | `should_trigger_for_level` (`mod.rs:577`), `invoke_triggers_for_log` (`mod.rs:2119`). |
| `engine::traces::list` | Aceita `trace_ids` **combinado** com `status: 'error'` — o ramo `trace_ids` busca os spans e o filtro de sumário aplica o status agregado | `mod.rs:2442` → `mod.rs:836`. Devolve `TraceSummary` com `error_count`, `function_id`, `service_name`, `trace_tags` (`iii.session.id`, `iii.message.id`, `iii.tag.*`). |
| `engine::traces::tree` | `{ trace_id }` → `{ roots: SpanTreeNode[] }` com `status`, `status_description`, `events` (`exception.type/message/stacktrace`), `attributes`, `resource` | A árvore completa é o que vira evidência. |
| `engine::logs::list` | `{ trace_id, severity_min?, limit }` → `{ logs }` | Enriquecimento por trace. |
| `engine::workers::list` | `WorkerSummary.version` auto-reportada no register | **Fonte da versão.** `service.version` nos spans vem de `SERVICE_VERSION` ou `"unknown"` em todos os SDKs — não serve. |
| Laço de entrega | O subscriber exclui spans de qualquer função registrada num trigger `trace`, renovando o conjunto a cada janela | `mod.rs:2219-2280`. O sentinel não precisa se proteger da própria entrega; precisa se proteger da própria *semântica* (abaixo). |

Ambas as entregas são `engine.call` fire-and-forget com resultado ignorado:
os handlers do sentinel só enfileiram e respondem.

### v1: `trace` + `log`

**`trace`** é a fonte principal. Cobre toda chamada de função que falha em
qualquer worker com SDK, e built-ins do engine com contexto de caller. Dá o
contexto de execução completo.

**Qual span vira evento.** Um trace com erro costuma ter a falha propagada em
cascata: `execute shell::run` (erro) → `harness::turn step` (erro) → raiz
(erro). Cada nível viraria um grupo. Regra: **só spans de erro folha** — spans
com status `error` cujos descendentes não contêm nenhum span de erro. Os
ancestrais com erro entram na evidência como `propagated_through`, não geram
grupo. Um pai que falha por razão própria depois de o filho ter sucesso é folha
e gera evento normalmente. Com o tier de decisões ligado, o ponto
`origin_span` resolve os casos que a regra da folha erra — erros em paralelo,
um pai que falha por razão própria depois de o filho também ter falhado.

**`log`** cobre o que o trace não vê: `tracing::error!` que nunca derruba uma
chamada, loops de fundo, falhas de reconcile. Todos os SDKs (Rust, Node,
Python) exportam logs ao engine.

**Regra de junção.** Log ERROR e span de erro do mesmo trace chegam **fora de
ordem, sempre**: o log é emitido dentro do handler e o span fecha depois, então
o tick de log costuma chegar antes. Por isso um log ERROR nunca vira grupo na
hora — entra como ocorrência `pending_join` por `join_window_ms`. Se nesse
tempo chega a ocorrência de span do mesmo `trace_id`, o log é dobrado nela
como enriquecimento e a pendente é removida; se o span já tinha chegado, o log
o encontra e dobra na hora. Expirou sem span → o log é **promovido** a evento
próprio, com fingerprint de log. A promoção é o único momento em que um grupo
de fonte log é criado ou incrementado. Na janela, a pendente é uma linha de
`sentinel_occurrences` **sem `group_id`** — o schema só admite isso com
`pending_join = 1`; a transação da promoção cria ou encontra o grupo e
preenche a coluna, a da dobra apaga a linha e leva o log para a evidência do
span. A janela é o fallback: com o tier ligado, `log_join` decide se o log
descreve a mesma falha sem depender de tempo.

### Adapters futuros (o `ErrorEvent` já nasce para eles)

- `harness-turn` — `harness::turn-completed { status: 'failed' }`: erro
  semântico do agente (recusa, orçamento, tool). Fingerprint por
  `reason` + `result_error` normalizados.
- `report` — função pública `sentinel::report { event: ErrorEvent }` para
  qualquer worker ou app empurrar erro. É o que torna o sentinel "adaptável
  a qualquer coisa" sem SDK.

Ambos são adapters atrás do mesmo trait; nenhum toca o núcleo.

### `ErrorEvent`

```typescript
type ErrorSource = "trace" | "log" | "harness-turn" | "report";

type ErrorEvent = {
  source: ErrorSource;
  dedupe_key: string;            // único por evento físico (ver Ingest)
  at_ms: number;
  namespace: string;             // "default" quando o engine só tem um; "?" quando namespace_ambiguous (ver Ingest)
  service_name: string;          // worker DONO do function_id (engine::functions::list) — não o service_name do span
  function_id?: string;          // atributo function_id do span, ou derivado de "execute <fn>"
  span_name?: string;
  exception_type?: string;       // exception.type do evento OTel, quando há
  message: string;               // exception.message ?? status_description ?? body — já redigido (Ingest, passo 4)
  stacktrace?: string;
  trace_id?: string;
  span_id?: string;
  session_id?: string;           // trace_tags["iii.session.id"]
  turn_id?: string;              // trace_tags["iii.message.id"]
  worker_version?: string;       // de engine::workers::list no momento do ingest
  attributes: Record<string, string>; // atributos do span/log, já filtrados (ver Evidência)
};
```

## Ingest

```mermaid
sequenceDiagram
  participant E as engine
  participant S as sentinel
  participant Q as queue
  participant D as database

  E->>S: sentinel::on-trace-activity { trace_ids }
  Note over S: descarta os ids dos próprios traces (anel)
  S->>Q: enqueue sentinel::ingest { trace_ids }
  S-->>E: { queued: n }
  Q->>S: sentinel::ingest (FIFO por trace_id · breaker)
  S->>E: traces::list { trace_ids, status: "error" }
  loop cada trace com error_count > 0
    S->>E: traces::tree { trace_id } · logs::list { trace_id }
    Note over S: raiz = sentinel ou sessão sentinel-inv-* → descarta o trace
    Note over S: spans de erro folha → ErrorEvent · dono via functions::list
    Note over S: redação de valores — o dado cru nunca chega ao banco
    Note over S: fingerprint (ns·dono·fn·tipo·norm) · dedupe
    S->>D: upsert grupo · ocorrência · evidência · sessão · bucket
    S-->>E: sentinel::group-changed (coalescido 1 s)
  end
  Note over S: agenda settle(trace_id) em +settle_delay_ms
  E->>S: sentinel::on-log { StoredLog }
  Note over S: pending_join por join_window_ms → dobra no span ou promove
```

**Passos por evento:**

1. **Exclusões — pela raiz do trace, não pelo span.** O engine cobre o laço
   da *entrega* do trigger. O laço que ele não cobre é o do trabalho do
   próprio Sentinel: uma gravação em `database::execute` que falha produz um
   span de erro no worker `database` — não no `sentinel` — que ticka, que
   vira ingest, que grava, que falha, sem fim. Por isso a regra descarta o
   **trace inteiro** quando a **raiz** é do Sentinel: root `service_name ==
   "sentinel"` ou root `function_id` `sentinel::*` (o `fn_queue
   sentinel-ingest` entra aqui). Excluir só spans com `service_name ==
   "sentinel"` não fecha o laço. Além disso:
   - traces cujo `iii.session.id` tem prefixo `sentinel-inv-` (sessões de
     investigação, nomeadas deterministicamente — ver [Investigação](#investigação));
   - spans `iii.function.kind = internal` ou `function_id` `engine::*`
     (`traces::list` já os oculta por padrão; a verificação é cinto);
   - `service_name` na lista `ignore_services` da config.
   Logs não carregam tag de sessão; como se aplica a eles está em
   [Logs sem tag de sessão](#logs-sem-tag-de-sessão).
2. **Ticks fantasmas.** `sentinel::ingest` roda como trace próprio (`fn_queue`
   → `execute sentinel::ingest` → `call database::*`), e o engine só exclui do
   trigger as funções **registradas num trigger `trace`** — o handler
   `on-trace-activity`, não o consumidor da fila. Sem cuidado, cada ingest
   produz um tick, que vira um job vazio: a fila dobra exatamente na
   tempestade. O ingest registra o `trace_id` corrente (do contexto OTel) num
   anel em memória de curta duração, e `on-trace-activity` filtra os ids do
   tick contra ele antes de enfileirar. Verificar na implementação se
   `internal: true` no registro de `sentinel::ingest` estampa
   `iii.function.kind=internal` nos spans — se sim, o engine exclui sozinho e
   o anel vira cinto.
3. **Atribuição pelo dono da função, não pelo span.** Quando o span `execute`
   do callee ainda não chegou (SDK atrasado, span perdido), a folha de erro é
   o `call <fn>` do engine, com `service_name` `iii` — o grupo nasceria como
   `iii · state::compare-and-set`, no repositório errado, e o settle corrige
   a árvore mas **não** o fingerprint. Por isso o `service_name` do evento é o
   **worker dono do `function_id`** (`engine::functions::list`, cacheado
   junto com os workers), e o span só o fornece quando não há `function_id`.
4. **Redação de valores — antes de persistir, antes de qualquer modelo.**
   Descartar atributos por nome (`authorization`, `token`, …) não protege o
   que vive dentro de `exception.message`, `status_description`, do corpo do
   log, de `db.statement` ou de `tool.arguments`: uma connection string num
   `status_description` atravessaria intacta, e o system prompt só age depois
   que o valor já cruzou a fronteira do provedor. Por isso o ingest passa
   **todo valor de texto** do evento — mensagem, stack, corpo, cada atributo
   que sobrou do filtro por chave — por um redator de padrões em memória,
   entre a resposta de `traces::tree`/`logs::list` e o primeiro `INSERT`:
   `Bearer …` e JWTs (`eyJ…`), chaves com prefixo conhecido (`sk-`, `ghp_`,
   `gho_`, `xox[abp]-`, `AKIA…`), blocos `-----BEGIN … PRIVATE KEY-----`,
   credenciais em URL (`scheme://user:pass@host`), pares `chave=valor` e
   `"chave": "valor"` cujo nome casa com a lista sensível, e e-mails. Cada
   acerto vira `[redacted:<kind>]` e conta em `status.ingest.redactions`. O
   dado cru **nunca é gravado**: `message_sample`, a evidência, o digest da
   mensagem de investigação e as entradas de triagem e decisão nascem do
   valor já redigido — e como o fingerprint corre depois, um token diferente
   por requisição deixa de separar grupos. Os padrões embutidos rodam
   sempre; `redaction.patterns` acrescenta os do projeto. Um redator por
   padrões é uma denylist: um segredo num formato desconhecido passa. O
   contador e a lista extensível são a resposta honesta a isso, não uma
   garantia (risco 23).
5. **Dedupe** por `dedupe_key`: `trace:<trace_id>:<span_id>` para spans,
   `log:<trace_id|->:<span_id|->:<timestamp_unix_nano>:<sha256(body)[:16]>`
   para logs. Índice único; colisão é no-op. Ticks disparam em abertura e
   fechamento de span, o mesmo span chega mais de uma vez.
6. **Fingerprint** → upsert do grupo (criar ou `last_seen`/`count++`).
7. **Transição de estado** (ver [Ciclo de vida](#ciclo-de-vida-do-grupo)).
8. **Ocorrência** com evidência completa se a retenção permitir; senão só
   a linha. Sessão distinta → linha em `sentinel_group_sessions`.
9. **Bucket horário** `(group_id, hour_ms) += 1` — alimenta o sparkline sem
   varrer ocorrências.
10. **Evento** `sentinel::group-changed`.

Regra extra, barata, que vale para todos os handlers: o Sentinel **nunca
emite `tracing::error!`** — um ERROR do próprio Sentinel voltaria pelo
trigger `log`. Falha interna é WARN mais contador no `status`.

**Breaker do ingest.** Retries da fila (3, com backoff) cobrem um soluço; não
cobrem o banco fora do ar. Com `ingest.breaker_failures` (5) jobs falhando em
sequência, o consumidor **pausa** por `ingest.breaker_cooldown_ms` (30 000) e
`status.ingest.paused_until` fica preenchido. Mensagens que esgotam os retries
são descartadas com o `trace_id` em WARN e contadas em `ingest.dropped_failed`
— nunca reprocessadas em laço. É o amplificador do laço do item 1 sendo
desligado na fonte.

**Settle.** O status `error` de um span é escrito no fechamento, mas o trace
pode continuar aberto (pai pendente, `end_time_unix_nano = 0`). Depois de
`settle_delay_ms` (padrão 5 000) o worker refaz `tree` + `logs` uma única vez
e substitui o bundle da ocorrência mais recente do trace, se o trace ainda
existir e tiver mudado. Uma só passada: o settle não é um loop de polling.

⚠ **A atribuição atravessa a reconstrução.** O settle reconstrói o bundle a
partir do span, e um span nomeia o processo que o *emitiu* — não o worker dono
da função que falhou. Reconstruir sem cuidado re-arquiva a evidência sob o
emissor (visto ao vivo: `iii:c:my-project` no lugar de `compose`). O `worker`
do bundle anterior é carregado para o novo; quem decide a atribuição é o passo
3, uma vez, na captura.

**Workers, versão e namespace.** Dois mapas em memória, renovados ao
encontrar uma chave desconhecida e por TTL (`workers_ttl_ms`, padrão 60 000):
`(namespace, worker) → version` de `engine::workers::list`, e
`(namespace, function_id) → worker` de `engine::functions::list` — este
segundo é o que dá a atribuição do passo 3. O engine permite o **mesmo nome
de worker em namespaces diferentes** (`WorkerSummary.namespace`) e, por
consequência, o **mesmo `function_id` registrado uma vez por namespace**
(`FunctionSummary.namespace` é "a chave `(namespace, function_id)` sob a qual
a função vive no registro", `engine_fn/mod.rs:241-247`). Um mapa chaveado só
por `function_id` guardaria um dono por função: dois registros com donos
diferentes se sobrescreveriam e o grupo nasceria com worker, versão e
fingerprint do namespace errado — marcar ambiguidade depois não recupera a
entrada perdida. Por isso o mapa guarda **todas** as entradas, mais um índice
`function_id → {namespaces}`, e o namespace entra na identidade do grupo. A
resolução de um span, nesta ordem:

1. o span traz namespace (`resource` / atributo estampado pelo SDK) → chave
   exata;
2. sem namespace, e a função existe num único namespace → esse;
3. sem namespace, e a função existe em vários: se o `service_name` do span
   coincide com o dono em exatamente um deles, esse; senão a ocorrência é
   marcada `namespace_ambiguous`, o evento leva `namespace = "?"` (um valor
   próprio, para que o grupo ambíguo nunca se misture com os resolvidos), o
   `service_name` do próprio span e `worker_version` nulo. A ambiguidade
   fica **preservada e visível** — nunca decidida por ordem de chegada.
   Quando ela some (um dos registros sai), as ocorrências novas resolvem
   normalmente; as antigas ficam no grupo `?` até a fusão proposta pelo
   tier, ou a mão do usuário, as juntar.

`engine::functions::info` sem namespace faz o análogo — responde erro
nomeando os candidatos em vez de escolher um — só que lá um registro em
`default` vence antes disso. Aqui não: um span sem namespace não diz de qual
namespace veio, e deixar `default` vencer atribuiria sistematicamente ao
worker errado assim que o nome existisse em dois. Assunção a validar na
implementação: o `service_name` dos spans é o nome com que o worker se
registrou, e qual atributo carrega o namespace.

**Medido, não assumido (SDK Rust).** `service.name` é
`OtelConfig::service_name` → `OTEL_SERVICE_NAME` → **o nome do binário**, e
nenhum atributo de namespace é exportado no `resource`
(`iii-helpers/src/observability/telemetry/mod.rs:218-235`). Dois efeitos: o
Sentinel declara `service_name: "sentinel"` explicitamente no seu próprio
`OtelConfig` — senão não se reconheceria na exclusão pela raiz — e a regra 1
acima (namespace vindo do span) nunca dispara para workers Rust, deixando o
peso nas regras 2 e 3. `service_aliases` na config é a saída quando um SDK
reporta um nome que não é o registrado.

### Logs sem tag de sessão

Registros de log trazem `trace_id` e `span_id`, mas **não** as tags de trace
(`iii.session.id`). A exclusão por raiz e por sessão de investigação se aplica
a logs **resolvendo o trace**: durante a janela `pending_join` o ingest lê
`traces::list { trace_ids: [id] }` uma vez e usa a raiz e as `trace_tags` do
trace para decidir. Como o log chega quase junto do span, o trace ainda existe
na esmagadora maioria dos casos — inclusive nos traces do próprio ingest, que
é o que fecha o laço do `database` pelo lado dos logs. Quando o trace já não
existe, o log é promovido **marcado** `session_unknown` e contado em
`status.ingest.logs_unattributed`: um vazamento limitado e visível, nunca
silencioso.

**Store desligado.** `engine::traces::list` respondendo
`memory exporter not enabled` deixa o sentinel em modo degradado:
`sentinel::status.engine.trace_store = "disabled"`, a UI mostra o mesmo estado
vazio da página de traces. Logs continuam se `logs_enabled`.

## Fingerprint

Determinístico, custo zero por evento, estável entre reinícios. O modelo de
IA nunca decide grupo.

```
trace:  key = [namespace, owner_worker, function_id ?? span_name, exception_type ?? "", normalize(message)]
log:    key = [namespace, service_name, attributes["code.function"] ?? attributes["target"] ?? instrumentation_scope_name ?? "", "", normalize(message)]
fingerprint = hex(sha256(source ⊕ 0x1f ⊕ key.join(0x1f)))[0..32]
```

`owner_worker` é o worker que registrou o `function_id` (ver [Ingest](#ingest),
passo 3) — nunca o `service_name` do span, que pode ser o `iii` de um `call`
do engine quando o span do callee ainda não chegou. `namespace` é `default`
quando o engine só tem um, e `?` numa ocorrência `namespace_ambiguous` (ver
[Ingest](#ingest)) — um valor próprio, para que o ambíguo nunca se misture
com o resolvido.

`normalize(msg)`, nesta ordem:

1. UUID / ULID → `<id>`
2. sequências hex ≥ 8 → `<hex>`
3. caminhos absolutos (`/…`, `[A-Z]:\…`) → `<path>`
4. strings entre aspas simples ou duplas → `<str>`
5. números (`\d+(\.\d+)?`) → `<n>`, **exceto** quando precedidos por um
   marcador de identidade — `HTTP`, `status`, `code`, `errno`, `exit`,
   `signal`, `port` — que ficam como estão: `HTTP 429` e `HTTP 500` são erros
   diferentes e não podem cair no mesmo grupo. A lista é
   `fingerprint.identity_numbers` na config, com esse padrão.
6. espaços colapsados, trim, truncado em 200 caracteres

Ordem importa: mascarar números antes de hex quebraria hashes. Os casos de
teste do normalizador são fixtures versionadas; mudar o normalizador muda
fingerprints de grupos existentes, então **mudanças exigem migração de
fingerprint** (recalcular a partir do `message` bruto guardado no grupo) — é
por isso que o grupo guarda `message_sample` cru.

Mascarar strings entre aspas é uma escolha com custo: `page "kanban" is not
registered` e `page "security-scan" is not registered` viram um grupo. É a
escolha certa quando o valor entre aspas é dado (nome de página, chave, id) e
errada quando é a identidade do erro. O v1 mascara sempre — grupos a mais são
mais baratos de fundir do que grupos a menos de separar, e o ponto
`group_membership` do tier de decisões existe para apanhar o caso errado. O
valor cru fica em `message_sample` e em cada ocorrência; nada se perde.

**Título** do grupo, sem LLM: `<exception_type>: <normalize(message)>`
truncado em 120; sem tipo, `<function_id>: <normalize(message)>`.

### Triagem (opcional, desligada por padrão)

Uma chamada `router::complete` por grupo novo, com o modelo de triagem, a
mensagem crua, o stack trace e os logs do trace — **sem acesso ao código**.
Produz:

```typescript
type TriageV1 = {
  title?: string;                // reescrita do título, mesma regra de tamanho
  summary?: string;              // 1–2 frases
  hypothesis: string;            // UMA linha, sempre exibida como "hipótese não testada"
  where_to_look: string[];       // caminhos/funções que o stack trace aponta
  category_guess?: "bug" | "configuration" | "dependency" | "transient" | "unknown";
};
```

Regras: nunca funde nem separa grupos; nunca preenche `root_cause` nem
`proposed_fix` — sem ler código, um "root cause" é palpite vestido de
diagnóstico, e o schema o faria parecer autoritativo. A UI mostra a triagem
como uma dica discreta na página do grupo, rotulada, e o card de diagnóstico
só existe depois de uma investigação com código.

Com o [tier de decisões](#tier-de-decisões-opcional) ligado, esta passada
racha: `category_guess`, `likely_expected` e quais frames importam viram
decisões calibradas, e `title` / `summary` / `hypothesis` continuam sendo o
único trabalho que exige um modelo generativo — desligável por conta própria.

## Ciclo de vida do grupo

```mermaid
stateDiagram-v2
  [*] --> new: primeira ocorrência
  new --> investigating: sentinel::investigate
  investigating --> diagnosed: sentinel::diagnosis::record
  new --> diagnosed: record de uma sessão ainda aberta
  investigating --> new: turn failed / cancelled
  new --> resolved: sentinel::groups::resolve
  diagnosed --> resolved: sentinel::groups::resolve
  diagnosed --> investigating: investigate de novo
  resolved --> regressed: nova ocorrência (regra de versão)
  regressed --> investigating: investigate
  regressed --> resolved: resolve
  new --> ignored: sentinel::groups::ignore
  diagnosed --> ignored: ignore
  regressed --> ignored: ignore
  ignored --> new: unignore / regra de ignore expirou
```

**Estados:** `new`, `investigating`, `diagnosed`, `resolved`, `regressed`,
`ignored`.

`investigating` cobre só a **primeira passada** da sessão. A conversa que
continua depois e cada novo `sentinel::diagnosis::record` não mudam o estado
— o grupo já está `diagnosed`, e cada registro novo só substitui o vigente.
*Investigate again* abre uma sessão nova e volta a `investigating`.

**Um `record` nunca desfaz uma decisão humana.** Ele move o grupo para
`diagnosed` apenas a partir de `investigating` ou `new` (o segundo cobre a
sessão que ficou aberta depois de um *Stop* ou de uma primeira passada sem
registro). Em `resolved`, `ignored` e `regressed` o estado **fica** e o
diagnóstico é anexado — a corrida real é o usuário clicar *Resolve* com um
pedido de diagnóstico ainda pendente na sessão; a resposta do agente chega
depois e não pode reabrir o grupo. A transição é decidida na transação da
gravação, lendo o estado daquele instante.

**Resolver** é humano. `resolve` aceita uma regra opcional:

```typescript
type ResolveRequest = {
  group_id: string;
  until_version_change?: boolean; // default false
};
```

- `false`: qualquer ocorrência nova → `regressed`.
- `true`: ocorrências da **mesma** `worker_version` de `resolved_version`
  contam (`count++`, `last_seen`) mas não regridem — a correção ainda não foi
  deployada. A primeira ocorrência com versão **diferente** → `regressed`. É o
  "resolved in next release" do Sentry, para uma stack onde a versão do worker
  é o release.

**Ignorar** tem escopo:

```typescript
type IgnoreRule =
  | { kind: "forever" }
  | { kind: "occurrences"; count: number }   // reabre após `count` novas ocorrências
  | { kind: "version_change" };              // reabre quando worker_version ≠ baseline
```

Um grupo ignorado continua ingerindo (contadores, `last_seen`, bucket) mas só
guarda a evidência da última ocorrência. A regra é avaliada a cada ocorrência;
ao expirar, o grupo volta a `new` e emite `group-changed { reason:
"ignore_expired" }`.

**Regressão** reposiciona o grupo no topo da lista (`regressed_at_ms`) e é
o único estado que a lista padrão destaca visualmente.

## Evidência

### `EvidenceBundleV1`

```typescript
type EvidenceBundleV1 = {
  version: 1;
  captured_at_ms: number;
  settled: boolean;                  // true depois do settle
  trace_id: string;
  origin_span_id: string;            // o span folha que gerou a ocorrência
  propagated_through: string[];      // span_ids dos ancestrais com erro
  trace_tags: Record<string, string>;
  spans: EvidenceSpan[];             // a árvore, achatada, em ordem de início
  logs: EvidenceLog[];               // logs do trace, severidade ≥ WARN (13), máx. 200
  worker: { service_name: string; version?: string };
  truncated: { spans: number; logs: number; attributes: number };
};

type EvidenceSpan = {
  span_id: string;
  parent_span_id?: string;
  name: string;
  service_name: string;
  function_id?: string;
  start_time_unix_nano: number;
  end_time_unix_nano: number;        // 0 = pendente na captura
  status: "ok" | "error" | "unset";
  status_description?: string;
  attributes: Record<string, string>; // filtrados
  events: Array<{ name: string; timestamp_unix_nano: number; attributes: Record<string, string> }>;
  depth: number;
};

type EvidenceLog = {
  timestamp_unix_nano: number;
  severity_text: string;
  body: string;
  span_id?: string;
  attributes: Record<string, string>;
};
```

**Filtro de atributos.** Ficam: `function_id`, `iii.*`, `exception.*`,
`code.*`, `error.*`, `rpc.*`, `http.*` (sem `http.request.header.*`),
`db.statement`, `tool.name`, e qualquer chave em `attribute_allowlist` da
config. Valores truncados em 4 KiB; `tool.arguments` e `tool.result` truncados
em 1 KiB. Chaves com `authorization`, `token`, `secret`, `api_key`, `password`
são descartadas. O bundle é truncado em `evidence_max_bytes` (padrão 1 MiB)
cortando spans mais distantes da origem primeiro — ou os menos relevantes,
com o ponto `evidence_pruning` ligado (desligado por padrão: é o único ponto
que roda um map sobre a árvore inteira).

**Redação de valores.** O filtro por chave é a primeira barreira, não a
única: todo valor que sobra — mensagem, stack, corpo de log, `db.statement`,
`tool.arguments`, cada atributo — já chega aqui redigido pelo passo 4 do
[Ingest](#ingest). O bundle nunca contém um valor cru, e é do bundle que
nascem o digest da investigação e as entradas dos modelos. O agente também
não tem outro caminho para o dado cru: na lista de allow, `engine::traces::*`
e `engine::logs::list` deram lugar a `sentinel::trace::get` e
`sentinel::logs::list`, proxies somente-leitura que aplicam o mesmo redator
à resposta do engine (ver [Plano do turno](#plano-do-turno)).

### Retenção

| O quê | Regra padrão |
|---|---|
| Evidência completa | primeira ocorrência + as `evidence_per_group` (5) mais recentes; as outras têm `evidence = NULL` (a linha fica) |
| Grupo ignorado | só a última ocorrência com evidência |
| Linhas de ocorrência | máximo `occurrences_per_group` (1 000); além disso apaga as mais antigas exceto a primeira |
| Buckets horários | `buckets_days` (30) |
| Grupos `resolved` sem ocorrência há `resolved_ttl_days` (90) | arquivados (linha fica, evidência removida) |

A poda roda no próprio ingest (barata, por grupo) e em um cron diário para
buckets e arquivamento.

"As mais recentes" é o fallback. Com o ponto `evidence_retention` ligado, o
grupo guarda as `evidence_per_group` **mais informativas** — a ocorrência que
difere (outra versão, outro caminho de chamada, outro atributo) vale mais que
a de dez segundos atrás. É a decisão de maior efeito prático do tier: muda o
que o harness vai encontrar meses depois, que é o ativo mais valioso do
worker.

### Arquivo (opcional)

`archive.bucket` configurado → cada bundle e cada diagnóstico também vão para
o `storage` como `sentinel/<group_id>/<occurrence_id>.json`. O `database`
permanece a fonte da verdade; o arquivo é cópia durável, no mesmo padrão do
`security-scan`. Sem bucket, nada muda — `storage` não é dependência
obrigatória porque exige configuração do operador para ter um bucket.

## Tier de decisões (opcional)

Tudo acima descreve o Sentinel inteiro e funcionando. Este tier **não
acrescenta funcionalidade**: ele troca heurísticas por decisões calibradas,
uma a uma, e cada uma continua tendo a heurística atrás como fallback
obrigatório.

**A invariante, e a razão de o tier existir nesta forma:** desligado, o
comportamento é exatamente o das seções anteriores. Ligar nunca muda contrato
de wire, nunca cria estado que o modo determinístico não saiba ler, e nunca
vira pré-requisito de nada. Desligar depois de ter usado não invalida o que
já foi decidido — decisões são **valores persistidos, nunca recalculados**.

### Por que um modelo de decisão e não um LLM

Toda constante afinada na mão nesta spec é uma decisão que eu resolvi com um
chute. É isso que o tier ataca — não as chamadas de LLM que já existem:

| Onde a spec decide por constante | A pergunta que ela responde mal |
|---|---|
| `join_window_ms: 2000` | este log ERROR descreve a mesma falha que este span? |
| "só spans de erro folha" | este span é a origem ou é propagação? |
| "poda os spans mais distantes da origem" | este span é relevante para a falha? |
| "primeira + as 5 mais recentes" | esta ocorrência acrescenta informação às que já guardei? |
| igualdade de fingerprint | esta ocorrência é mesmo a mesma causa do exemplar do grupo? |
| `category_guess` da triagem | qual a categoria provável? |

Um modelo **System One** — saída sempre dentro de um schema, sem geração de
texto livre, com probabilidade calibrada e latência de dezenas a centenas de
milissegundos — é a forma certa dessas perguntas. O [Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
é a implementação de referência (custo anunciado US$ 0,042/MTok de entrada,
saída livre, 70–500 ms); o tier é escrito contra a **forma**, não contra o
fornecedor.

**A aritmética é o que muda o desenho.** A spec proíbe qualquer chamada de
modelo por ocorrência porque o custo escala com o incidente. Uma decisão
recebe o digest — mensagem, atributos, exemplar do grupo — e não o bundle:
~2 k tokens, ~US$ 0,00008 por chamada, ~US$ 0,08 por mil ocorrências. **Por
ocorrência passa a caber.** Mandar o bundle inteiro (1 MiB ≈ 250 k tokens)
custaria ~130× mais por chamada e está proibido em todos os pontos.

### Contrato

```typescript
type DecisionPoint =
  | "origin_span" | "log_join" | "group_membership" | "group_merge"
  | "evidence_retention" | "evidence_pruning"
  | "triage" | "investigation_preflight" | "diagnosis_verification";

type DecisionV1<T> = {
  point: DecisionPoint;
  value: T | null;          // null = abaixo do limiar; a heurística decidiu
  confidence: number;       // 0..1, calibrada
  fell_back: boolean;       // true quando a heurística decidiu (limiar, timeout, tier off)
  model?: string;
  at_ms: number;
};
```

**Banda de confiança, uma regra para todos os pontos — com o limiar de
cada um.** `confidence ≥ decisions.points.<ponto>.min_confidence` → o valor
vale. O limiar é **por ponto** porque o custo do erro não é o mesmo: guardar
uma amostra um pouco menos informativa (`evidence_retention`) é barato;
declarar que uma ocorrência não é do grupo (`group_membership`) ou que um log
é a mesma falha (`log_join`) não é. Abaixo → a heurística decide e a
decisão é gravada mesmo assim, com `fell_back: true`. A faixa do meio é
**"não sei"**, nunca um arredondamento: é para isso que a calibração serve, e
é o que separa este tier de um classificador comum.

**Três modos de agir**, declarados por ponto:

| Modo | Significado | Pontos |
|---|---|---|
| `auto` | a decisão age direto; a heurística faria uma escolha equivalente e o custo do erro é uma amostra pior | `origin_span`, `log_join`, `evidence_retention`, `evidence_pruning`, `investigation_preflight` |
| `proposal` | a decisão vira sugestão revisável na UI; nada muda sozinho | `group_membership`, `group_merge`, `triage` (auto-ignore) |
| `display` | a decisão só é exibida ao lado do que ela julga | `diagnosis_verification` |

Nada que altere estado de grupo (resolver, ignorar, fundir) roda em `auto`.
Continua valendo a fronteira: só o humano resolve.

### Os pontos

| Ponto | Heurística (fallback, e o comportamento com o tier desligado) | O que a decisão acrescenta | Escala |
|---|---|---|---|
| `origin_span` | span de erro sem descendente com erro | classifica origem vs. propagação quando a árvore é ambígua (erro em paralelo, pai que falha por razão própria) | por span de erro |
| `log_join` | mesmo `trace_id` dentro de `join_window_ms` | decide se o log descreve **a mesma falha**, sem depender de janela de tempo | por log ERROR |
| `group_membership` | igualdade de fingerprint | detecta colisão: mesma mensagem normalizada, causa diferente. `proposal`: marca a ocorrência como suspeita, não move de grupo | por ocorrência |
| `group_merge` | nada (fora de escopo do v1) | na criação de um grupo, aponta grupos existentes com a mesma causa. **Aresta persistida, nunca recálculo de identidade** | por grupo novo |
| `evidence_retention` | primeira + as N mais recentes | pontua novidade: guarda as N **mais informativas** (outra versão, outro caminho, outro atributo) em vez das mais recentes | por ocorrência |
| `evidence_pruning` | corta spans mais distantes da origem | corta os menos relevantes quando o bundle estoura `evidence_max_bytes` | map sobre a árvore |
| `triage` | título determinístico; sem categoria | `category`, `likely_expected` e quais frames do stack importam — as partes **estruturadas** do `TriageV1` | por grupo novo |
| `investigation_preflight` | mapa `repositories` da config | "vale investigar?", repositório provável e por quais arquivos começar — o modelo forte entra com um plano em vez de se localizar | por investigação |
| `diagnosis_verification` | validação de schema | por item de evidência, se ele sustenta a afirmação; e se a confiança **auto-declarada** pelo modelo forte bate com o que foi citado | por diagnóstico |

`diagnosis_verification` é o ponto que mais muda a confiança do produto. O
`DiagnosisV1` traz `confidence: "high"` escrito pelo próprio modelo que fez o
diagnóstico — isso é retórica, não calibração. A verificação independente
produz um número calibrado ao lado; onde os dois discordam, a UI mostra os
dois. A aba Diagnosis ganha essa marca; o card nunca é escondido nem
reescrito por causa dela.

### Identidade continua determinística

`group_merge` e `group_membership` **não podem** mudar o fingerprint. A
identidade segue sendo `sha256` sobre a mensagem normalizada, reproduzível a
partir do `message_sample` guardado. O que a decisão produz é uma **aresta**
(`sentinel_group_edges`) que a UI e as consultas respeitam, revisável e
revogável. Agrupamento não determinístico faria o histórico oscilar, quebraria
a detecção de regressão e faria os contadores mentirem — o preço é alto
demais para o ganho.

### O que o tier não faz

- **Texto livre.** `title`, `summary` e `hypothesis` do `TriageV1` são prosa;
  um modelo System One abre mão de gerar string. Com o tier ligado e o
  modelo de triagem desligado, o grupo mostra título determinístico +
  categoria calibrada — e **essa é a configuração que eu recomendo**: some a
  hipótese escrita por modelo, que era a parte que mais lia como autoridade
  sem ter uma.
- **Diagnóstico.** Causa-raiz e correção proposta exigem ler código e
  escrever prosa; são do harness, sempre.
- **Transições de estado.** Resolver e ignorar são humanos por desenho.

### Integração

Jev é um **provider do `llm-router`**, no padrão dos outros
(`provider-anthropic`, `provider-openai`, …), e o router ganha uma superfície
de decisão — `router::decide { model, point, input, schema }` →
`{ value, confidence }`. O router já carrega a peça que falta para isso:
`supports_structured_output` é capacidade de modelo no catálogo
(`llm-router/src/types/model.rs`) e `router/structured_output_unsupported` já
é código de erro; um `supports_decisions` irmão fecha o contrato. O Sentinel
nunca fala com o fornecedor direto — credencial e roteamento são do router,
como em todo o resto da stack.

Enquanto `router::decide` não existir, o tier fica `enabled: false` e a spec
inteira vale como está. Essa é a questão aberta #10.

### Falha e degradação

O ingest **não pode** depender do tier: colocar uma API externa no caminho
quente significa depender dela exatamente durante um incidente, quando ela
também pode estar degradada.

- `decisions.timeout_ms` (padrão 400) por chamada. Estourou → heurística,
  `fell_back: true`, segue o fluxo. Nunca re-tenta no caminho do ingest.
- Erro, indisponibilidade ou tier desligado → idêntico: heurística.
- Um circuit breaker simples: `consecutive_failures ≥ 5` desarma o tier por
  `cooldown_ms` (padrão 60 000) e o `sentinel::status` reporta.
- As taxas de fallback por ponto entram no `status`. Fallback alto e
  silencioso é o modo de falha real aqui — um tier ligado que nunca decide
  parece estar funcionando.

**Cuidado que não deve ser perdido de vista:** "não alucina" é garantia de
**tipo**, não de acerto. A saída sempre casa com o schema; ela pode estar
errada assim mesmo, e uma decisão bem tipada e errada parece confiável. Por
isso a probabilidade é persistida junto com o valor, a banda do meio existe,
e nenhum ponto que mude estado de grupo roda em `auto`.

## Investigação

Uma investigação é uma **sessão do harness que o usuário assiste e
direciona**, não um job que devolve um resultado. O que a torna confiável são
duas coisas distintas, e o desenho preserva as duas:

- **O processo** é visível: a sessão abre na coluna ao lado da página do
  grupo, e o usuário vê o agente ler arquivos, levantar hipóteses e chamar
  funções — e pode intervir a qualquer momento.
- **O registro** é estruturado: quando o agente tem uma causa provável, ele
  a **grava** chamando `sentinel::diagnosis::record { group_id, diagnosis }`
  — uma função iii do próprio Sentinel, exposta a ele como qualquer outra,
  com o `DiagnosisV1` (causa-raiz com evidência `file:line`, correção
  proposta, confiança, o que faltou) como schema da requisição. É isso que dá
  o estado `diagnosed`, sobrevive à regressão e pode ser comparado entre
  versões. Não há contrato de saída em nenhum turno: a conversa é livre, e o
  registro é uma ação do agente dentro dela.

Três propriedades do harness sustentam isso sem gambiarra:

1. **funções são a interface do agente**: com `expose: "agent_trigger"` o
   modelo chama qualquer função da lista de allow. ⚠ **Medido no harness
   (`harness/src/policy.rs:186-215`, `harness/src/trigger.rs:569-608`): o
   despacho checa a política de globs e que o schema compila, e não valida o
   payload contra ele.** A validação real é o `serde` do request tipado do
   SDK — por isso `DiagnosisRecordRequestV1` e todo struct aninhado nele
   usam `deny_unknown_fields` e enums fechados: é isso, e só isso, que
   recusa um `DiagnosisV1` malformado, com uma mensagem que o agente lê e
   corrige;
2. **steering**: um `send` numa sessão com turno em andamento funde a
   mensagem no turno que roda (`merged: true`) — o usuário interrompe o
   agente no meio da investigação e ele incorpora;
3. a sessão é uma **conversa real do console**, aberta ao lado da página com
   `host.chat.selectConversation(session_id)` — o `security-scan` já faz
   exatamente isso com a sessão de análise.

### Como funciona

1. **Investigate** no grupo → `sentinel::investigate { group_id }` cria a
   sessão (`sentinel-inv-<id>`, `kind: automation`) e manda a **primeira
   passada**: um turno de **texto livre** — mensagem com a evidência, política
   somente-leitura, **sem contrato de saída**. Devolve o `session_id`; a UI
   abre a sessão na coluna ao lado.
2. O usuário assiste e, se digitar, a mensagem entra por steering no turno em
   curso — e o agente responde em prosa, porque nada no turno força JSON.
3. Quando tem uma causa provável — ou quando o system prompt manda encerrar
   — o agente chama **`sentinel::diagnosis::record { group_id, diagnosis }`**
   de dentro da sessão. O Sentinel confere que a sessão é uma investigação
   daquele grupo, valida o schema, grava a linha em `sentinel_diagnoses` e
   move o grupo para `diagnosed`. Os cards aparecem na aba Diagnosis na
   hora; a chamada fica visível no transcript como qualquer `triggered ƒ`.
4. Dali em diante a conversa segue **com a política de funções do console**.
   É o caminho natural de diagnóstico para correção: o usuário libera
   `shell::*` para rodar o teste, ou pede o patch, na mesma sessão, com o
   controle que o console lhe dá. O Sentinel não tem um modo "fix"; o
   console já é esse modo.
5. **Ask for a diagnosis** na aba Diagnosis não é uma função do Sentinel: é
   o console compondo uma mensagem na sessão (`host.chat.compose({ text,
   submit: true })` — "grave o que você tem até agora"). O agente responde
   chamando `record` de novo; cada chamada é uma versão nova, o mais recente
   vira o vigente, os anteriores ficam. Depois da primeira passada a chamada
   segue a política do console; `record` continua sendo a única escrita do
   Sentinel que o agente conhece.
6. Entrada inversa: **Open in chat** (`mode: "chat"`) cria a sessão sem
   primeira passada: ⚠ `session::ensure { session_id, title, kind: automation,
   metadata }` (`session::create` não aceita id do chamador) e
   `session::append` da evidência como entrada `user` marcada
   ⚠ `origin.sentinel_evidence: true` (entradas não têm `metadata`; `origin` é
   o campo opaco que o append carrega) — só transcript, nenhum turno roda até o
   usuário falar; quando falar, o turno do console monta o contexto com a
   evidência dentro. O console renderiza essa entrada com o renderer de
   transcript do Sentinel, rotulada como dele, não como do usuário. O registro
   é o mesmo: o agente chama `record` quando tiver algo a gravar.

```mermaid
sequenceDiagram
  participant P as console · página
  participant S as sentinel
  participant H as harness
  participant C as console · sessão ao lado

  P->>S: sentinel::investigate { group_id }
  S->>H: session::ensure (kind: automation) · filesystem::grant
  S->>H: harness::send — 1ª passada (texto, somente leitura)
  S-->>P: { session_id }
  P-->>C: selectConversation
  C->>H: usuário digita → steering (merged: true)
  H-->>C: responde em prosa
  H->>S: sentinel::diagnosis::record { group_id, diagnosis } (ƒ chamada pelo agente)
  Note over S: sessão ∈ investigação do grupo? · schema · verificação (opcional) · grava v1 · grupo → diagnosed
  S-->>P: group-changed → card
  H->>S: turn-completed { first_pass_turn_id }
  Note over S: sem registro? um nudge de texto, uma vez
  C->>H: a conversa continua · regras do console
  H->>S: sentinel::diagnosis::record de novo → v2
  P-->>C: Ask for a diagnosis → compose("grave o que tem")
```

**Por que não há contrato de saída em turno nenhum.** Verificado no harness:
com `structured_output` nativo ele passa `response_format: json` ao provedor
e **toda** mensagem do assistente sai JSON — o usuário digitaria no meio da
passada e receberia JSON ou silêncio. A conversa é a prioridade. O registro
estruturado é uma **função**, e função é o que o agente já sabe chamar: o
schema vale (o request tipado do worker o impõe — ver o ⚠ acima: o harness
despacha, o `serde` valida), a versão vale (cada chamada é uma linha), e nada
constrange a prosa. Também
some a necessidade de a sessão estar ociosa, a fila de pedidos e o
reconhecimento de turnos "do Sentinel" — o `group_id` na chamada diz tudo.

Uma restrição honesta: a mensagem fundida por steering **não muda modelo,
system prompt nem política** do turno em curso (harness § Concurrency &
steering) — isso vale a partir do turno seguinte. Na prática não atrapalha:
o usuário interrompe com informação; permissão vem no próximo turno dele.

### Sem tetos

O Sentinel **não impõe** `max_turns`, tokens, custo, concorrência nem cota
por hora. O usuário está olhando, e **Stop** (`harness::stop`) é o limite.
Os defaults do próprio harness continuam valendo (`max_turns` 16 por turno)
e são visíveis na sessão como em qualquer conversa. A única regra de
concorrência é semântica, não de orçamento: uma sessão em primeira passada
por grupo — um segundo *Investigate* abre a existente em vez de criar outra.

### Plano do turno

```typescript
// harness::send
{
  session_id: `sentinel-inv-${investigation_id}`,
  message: string,                    // ver "Mensagem"
  model, provider,
  idempotency_key: `${investigation_id}:analysis`,
  session: {
    title: `Sentinel: ${group.title}`,
    kind: "automation",
    metadata: { sentinel: true, sentinel_group_id, sentinel_investigation_id },
  },
  options: {
    system_prompt, system_prompt_strategy: "override",
    // sem `output`: texto livre; o DiagnosisV1 é gravado pelo agente com sentinel::diagnosis::record
    functions: {
      allow: [
        "coder::info", "coder::read-file", "coder::search", "coder::list-folder", "coder::tree",
        "engine::functions::info", "engine::functions::list",
        "sentinel::trace::get", "sentinel::logs::list",   // o engine vivo, pelos proxies redigidos — nunca engine::traces::* direto
        "sentinel::evidence::get",
        "sentinel::diagnosis::record",   // a única escrita: o próprio diagnóstico, só para este grupo
      ],
      deny: [
        "shell::*", "state::*", "queue::*", "worktree::*", "harness::*", "github::*",
        "configuration::*", "storage::*", "database::*", "engine::traces::*", "engine::logs::*",
        "sentinel::groups::*", "sentinel::investigate", "sentinel::investigations::*",
        "sentinel::on-*", "sentinel::ingest", "sentinel::investigate-run", "sentinel::report",
      ],
      expose: "agent_trigger",
    },
    metadata: { fs_scope: { root: repository.path } },
  },
}
```

A política do harness é fail-closed com **deny vencendo**: uma chamada passa
só se casar um glob de `allow` e nenhum de `deny`
(`harness/src/policy.rs:112`). Por isso o deny não pode ser `sentinel::*` —
mataria `sentinel::evidence::get`. A lista nega os ids mutáveis um a um; toda
função nova do sentinel entra nela por padrão (teste de manifesto: o conjunto
⚠ `registrados − {evidence::get, trace::get, logs::list, diagnosis::record} ⊆ deny`).
A isenção de `status` e `occurrences::list` **caiu**: o agente trabalha com a
evidência que recebeu e o código que pode ler; a superfície de saúde do worker
e a lista de todas as outras ocorrências são do console, não da investigação.
Ambas estão no deny, e o teste de manifesto exige que toda função registrada
esteja numa das duas listas.

Antes do `send`: `harness::filesystem::grant { session_id, root }`.

**Não há gate nesta arquitetura: permissão é sempre `full`.** Nenhuma
chamada do agente é segurada para aprovação humana, e o Sentinel não faz
nenhuma chamada `approval::*`. A fronteira de segurança é a política
**fail-closed do próprio harness** — `functions.allow` estreito, `deny`
explícito, `fs_scope` na raiz mapeada. A consequência prática é que **a lista
de allow é o modelo de permissão inteiro**: o que está nela roda, o que não
está é recusado antes de sair. Por isso ela nunca contém uma escrita fora do
Sentinel — o agente lê código e grava o próprio diagnóstico, e nada mais.

**`sentinel::diagnosis::record` é a única escrita da lista.** A identidade
da sessão vem **só** do contexto da invocação, nunca do payload: o
`harness::turn step` estampa `iii.session.id` na baggage OTel do turno
(`harness/src/functions/turn.rs:51-53`), o engine a carrega na invocação
(`InvokeFunction.baggage`, `engine/src/protocol.rs:176`) e o handler a lê
com `get_baggage_entry` do SDK. O agente não escreve baggage.

⚠ **Onde essa leitura acontece é load-bearing, e errar é silencioso.** O SDK
avalia `handler(request)` e **só então** embrulha o futuro devolvido com o
contexto OTel da invocação (`iii-sdk-0.23.0/src/iii.rs:2327`
— `handler(data, metadata).with_context(otel_cx).await`). Ler a baggage no
corpo do closure roda **fora** desse contexto e devolve `None`: compila,
passa em todo teste unitário, e só falha quando há um modelo do outro lado.
A leitura tem de estar dentro do bloco `async` — pela mesma razão que um
`tokio::spawn` nu perde o contexto. Medido ao vivo: seis chamadas seguidas de
`record` recusadas com `sentinel/no_investigation` enquanto a regra estava
certa e a leitura no lugar errado. Um teste de fonte fixa a posição. O Sentinel
resolve `session_id → investigação` e aceita a chamada só quando essa
investigação é do `group_id` informado. Sem `iii.session.id`, ou com um id
que não é de investigação → `sentinel/no_investigation`; investigação de
outro grupo → `sentinel/not_this_group`. O payload **não** tem
`investigation_id`: um id que aparece nas respostas de
`investigations::list` não prova nada, e um fallback por ele deixaria
qualquer chamada sem contexto escolher uma investigação e gravar em nome
dela. Fora dessa função, a política é a mesma leitura jailed de antes.

No `mode: "chat"` não há primeira passada nem jail: a sessão nasce por
⚠ `session::ensure`, a evidência entra por `session::append` como entrada
`user` com ⚠ `origin.sentinel_evidence: true` (conteúdo `custom` **nunca**
chega ao modelo, por contrato do harness — não serve para isto), e todo turno
é do usuário, com a política do console.

`sentinel::trace::get` e `sentinel::logs::list` na lista de allow são o "se o
trace ainda existir": o agente pode comparar o snapshot com o estado atual ou
olhar traces vizinhos — pelo proxy do Sentinel, que redige a resposta do
engine com o redator do ingest, nunca por `engine::traces::*` direto (que
está no deny, por cinto). O diagnóstico declara quando trabalhou só com o
snapshot.

### Mensagem

Markdown determinístico, gerado pelo worker:

1. Cabeçalho: título, fingerprint, `service_name`/`function_id`, estado,
   `count`, `first_seen`/`last_seen`, sessões distintas, versão do worker na
   ocorrência (`worker_version`) e no checkout (`git rev-parse` do
   `repository.path`, quando for um repositório git).
2. **Digest da evidência**, inline: o span de origem (nome, `function_id`,
   `status_description`, evento `exception` completo, atributos), o caminho
   de ancestrais (nomes e status), spans irmãos com erro, e os logs do trace
   (≥ WARN, máx. 50). Limite de `message_max_bytes` (64 KiB); o que exceder
   fica só em `sentinel::evidence::get`.
3. As três ocorrências anteriores mais recentes: `at_ms`, versão, mensagem
   crua — para o agente ver variação.
4. O mapa: "o código de `<service_name>` provavelmente vive em
   `<repository.path>/<service_name>/`; o repositório inteiro está
   disponível em leitura". Com o ponto `investigation_preflight` ligado,
   entra aqui a lista curta de arquivos por onde começar, e o turno abre com
   um plano em vez de gastar chamadas de `coder::search` se localizando.
5. Instrução de registro: quando tiver uma causa provável, **grave** com
   `sentinel::diagnosis::record { group_id: <este>, diagnosis }`; grave de
   novo sempre que a causa mudar; ao encerrar sem causa, grave mesmo assim
   com `confidence: "low"` e `missing_evidence` — é resposta válida e
   preferível a chute.

O system prompt fixa o papel (investigador somente-leitura), trata todo
texto de repositório, trace e log como dado não confiável, proíbe executar
código, mutar arquivos ou reproduzir segredos, exige caminhos relativos ao
repositório com linha quando houver, avisa que **o usuário pode intervir a
qualquer momento** e que essas mensagens têm precedência sobre o plano do
agente, e nomeia `sentinel::diagnosis::record` como a única escrita
permitida — e a forma de entregar o resultado.

### `DiagnosisV1`

```typescript
type DiagnosisV1 = {
  summary: string;                   // 1–3 frases, voltado ao humano
  category: "bug" | "configuration" | "dependency" | "transient" | "expected" | "unknown";
  confidence: "high" | "medium" | "low";
  root_cause: {
    description: string;
    evidence: Array<{
      kind: "code" | "trace" | "log";
      path?: string; line?: number;  // kind: code (relativo ao repositório)
      span_id?: string;              // kind: trace
      excerpt: string;               // ≤ 400 chars
      why: string;
    }>;
  };
  proposed_fix?: {
    description: string;
    files: string[];
    risk: "low" | "medium" | "high";
    steps: string[];
  };
  reproduction?: string;
  missing_evidence?: string[];       // o que faltou para subir a confiança
  version_note?: string;             // deriva entre checkout e versão que falhou
  related_groups?: string[];         // group_ids que o agente acredita partilharem a causa
};
```

`related_groups` é sugestão exibida na UI; o sentinel não funde grupos por
conta disso no v1.

### Conclusão e diagnósticos

- **O diagnóstico chega pela função, não pelo fim do turno.** Cada chamada
  válida de `sentinel::diagnosis::record` é uma linha em `sentinel_diagnoses`
  (`source: first_pass` enquanto a primeira passada roda, `conversation`
  depois) e vira o vigente do grupo — na hora, sem esperar `turn-completed`.
  O estado só muda a partir de `investigating` ou `new` (→ `diagnosed`), e a
  transição lê o estado **no instante da gravação**, na mesma transação. Se
  alguém resolveu ou ignorou enquanto o agente trabalhava — o pedido "grave
  o que você tem" ainda pendente e o usuário já clicou *Resolve* — o estado
  fica e o diagnóstico é anexado mesmo assim, com `group_status` na resposta
  dizendo qual. Um `regressed` também fica: regressão é fato sobre
  ocorrências, e só um `resolve` humano ou um *Investigate again* a limpa.
- Payload inválido contra o schema → o harness nem despacha (o `agent_trigger`
  valida) e o agente vê o erro; chamada sem `iii.session.id` na baggage ou de
  uma sessão que não é investigação → `sentinel/no_investigation`;
  investigação de outro grupo → `sentinel/not_this_group`. Todos visíveis ao
  agente; nenhum cria linha.
- Doorbell: trigger `harness::turn-completed` (config `{}`), handler
  `sentinel::on-turn-completed` — ignora sessões sem o prefixo
  `sentinel-inv-`. Só um turno interessa ao Sentinel: o da primeira passada
  (`first_pass_turn_id`). Turnos do usuário são do usuário.
- Primeira passada `completed` → se terminou **sem nenhum registro**, o
  Sentinel manda **um** nudge de texto
  ("grave o que você tem com `sentinel::diagnosis::record`") — uma vez, nunca
  em laço; se ainda assim não vier, o grupo fica `investigating → new` e a
  UI diz "no diagnosis recorded — ask for one in the session".
- Reconcile a cada 30 s via `harness::status { session_id, verbose: true }`
  para primeiras passadas `running` (doorbell perdido, restart). Igual ao
  `security-scan`.
- Primeira passada `failed` / `cancelled` → investigação `failed` /
  `cancelled` com `error`; o grupo mantém o que já foi
  gravado (um `record` anterior continua vigente) ou volta ao estado anterior
  (`previous_status`) se nada foi gravado. A sessão continua aberta.
- Turnos, duração e custo observados vêm de `harness::metrics { session_id }`
  quando disponível — informação, nunca teto.
- Com o ponto `diagnosis_verification` ligado, cada registro passa pela
  verificação antes de virar o vigente: o resultado é gravado em
  `sentinel_diagnoses.verification` e exibido ao lado da confiança
  auto-declarada. A verificação **nunca** rejeita nem reescreve o diagnóstico
  — discordância é informação para o humano, não um veto.

### A sessão no console

A sessão é `kind: automation` **nos dois modos**, `assisted` e `chat`, e
assim permanece: o kind é gravado na criação, `session::set-meta` não o muda,
e os turnos que o usuário manda depois pela console não o alteram — uma
sessão de investigação não vira conversa do usuário por ele ter entrado nela.
Fica fora do sidebar padrão de conversas, no filtro de automações. A página do grupo é o lugar de encontrá-la: **Investigate**
abre a sessão na coluna ao lado (`host.chat.selectConversation`), o estado
`investigating` mostra "ao vivo" enquanto a primeira passada roda, e a aba
Diagnosis tem *Continue in chat* depois. A sessão traz o link de volta ao
grupo (`session.metadata.sentinel_group_id`).

## Modelo de dados

`database` (`primary`, SQLite por padrão — zero-config). Tabelas prefixadas
`sentinel_`, criadas com `CREATE TABLE IF NOT EXISTS` no boot; migrações por
`sentinel_meta.schema_version`.

```sql
CREATE TABLE sentinel_groups (
  id                TEXT PRIMARY KEY,          -- grp_<ulid>
  fingerprint       TEXT NOT NULL UNIQUE,
  source            TEXT NOT NULL,             -- trace | log | harness-turn | report
  namespace         TEXT NOT NULL DEFAULT 'default',
  service_name      TEXT NOT NULL,             -- worker dono do function_id (não o service_name do span)
  function_id       TEXT,
  exception_type    TEXT,
  title             TEXT NOT NULL,
  summary           TEXT,                      -- triagem, opcional
  triage            TEXT,                      -- JSON TriageV1, opcional
  message_sample    TEXT NOT NULL,             -- mensagem crua da primeira ocorrência
  status            TEXT NOT NULL,             -- new | investigating | diagnosed | resolved | regressed | ignored
  previous_status   TEXT,
  ignore_rule       TEXT,                      -- JSON IgnoreRule
  ignore_baseline   TEXT,                      -- JSON { count, version }
  first_seen_ms     INTEGER NOT NULL,
  last_seen_ms      INTEGER NOT NULL,
  occurrence_count  INTEGER NOT NULL DEFAULT 0,
  first_version     TEXT,
  last_version      TEXT,
  resolved_at_ms    INTEGER,
  resolved_version  TEXT,
  resolve_until_version_change INTEGER NOT NULL DEFAULT 0,
  regressed_at_ms   INTEGER,
  diagnosis_id      TEXT,                      -- o DiagnosisV1 vigente (sentinel_diagnoses)
  archived          INTEGER NOT NULL DEFAULT 0,
  updated_ms        INTEGER NOT NULL
);
CREATE INDEX sentinel_groups_list ON sentinel_groups (archived, status, last_seen_ms DESC);
CREATE INDEX sentinel_groups_service ON sentinel_groups (service_name, last_seen_ms DESC);

CREATE TABLE sentinel_occurrences (
  id              TEXT PRIMARY KEY,            -- occ_<ulid>
  group_id        TEXT REFERENCES sentinel_groups(id),  -- NULL só enquanto pending_join = 1 (ver CHECK)
  dedupe_key      TEXT NOT NULL UNIQUE,
  source          TEXT NOT NULL,
  at_ms           INTEGER NOT NULL,
  trace_id        TEXT,
  span_id         TEXT,
  session_id      TEXT,
  turn_id         TEXT,
  worker_version  TEXT,
  message         TEXT NOT NULL,
  evidence        TEXT,                        -- JSON EvidenceBundleV1 ou NULL (podado)
  evidence_bytes  INTEGER NOT NULL DEFAULT 0,
  settled         INTEGER NOT NULL DEFAULT 0,
  pending_join    INTEGER NOT NULL DEFAULT 0,  -- log ERROR à espera do span do mesmo trace
  join_deadline_ms INTEGER,                    -- fim da janela; NULL quando não é log pendente
  session_unknown INTEGER NOT NULL DEFAULT 0,  -- log promovido sem conseguir resolver o trace
  namespace_ambiguous INTEGER NOT NULL DEFAULT 0,
  novelty         REAL,                        -- ponto evidence_retention; NULL = tier desligado (ordena por recência)
  membership_doubt REAL,                       -- ponto group_membership; NULL = sem dúvida levantada
  -- Um log pendente é uma linha SEM grupo: o grupo de fonte log só nasce na
  -- promoção, e é nessa transação que a coluna é preenchida (ou a linha
  -- apagada, se o log dobrou num span). Fora da janela, group_id é obrigatório.
  CHECK ((pending_join = 1 AND group_id IS NULL) OR (pending_join = 0 AND group_id IS NOT NULL))
);
CREATE INDEX sentinel_occurrences_pending ON sentinel_occurrences (pending_join, join_deadline_ms);

-- Sessões distintas por grupo: uma linha por (grupo, sessão), nunca podada
-- com as ocorrências — é o que mantém "sessions affected" correto depois
-- que as linhas antigas caem.
CREATE TABLE sentinel_group_sessions (
  group_id     TEXT NOT NULL REFERENCES sentinel_groups(id),
  session_id   TEXT NOT NULL,
  first_ms     INTEGER NOT NULL,
  last_ms      INTEGER NOT NULL,
  PRIMARY KEY (group_id, session_id)
);
CREATE INDEX sentinel_occurrences_group ON sentinel_occurrences (group_id, at_ms DESC);
CREATE INDEX sentinel_occurrences_trace ON sentinel_occurrences (trace_id);

CREATE TABLE sentinel_buckets (
  group_id  TEXT NOT NULL,
  hour_ms   INTEGER NOT NULL,
  count     INTEGER NOT NULL,
  PRIMARY KEY (group_id, hour_ms)
);

CREATE TABLE sentinel_investigations (
  id               TEXT PRIMARY KEY,           -- inv_<ulid>
  group_id         TEXT NOT NULL REFERENCES sentinel_groups(id),
  occurrence_id    TEXT NOT NULL,              -- evidência primária
  session_id       TEXT NOT NULL,
  mode             TEXT NOT NULL,              -- assisted | chat
  first_pass_turn_id TEXT,                     -- NULL em mode = chat
  nudged           INTEGER NOT NULL DEFAULT 0, -- a primeira passada acabou sem registro e o nudge já foi
  model            TEXT NOT NULL,
  provider         TEXT,
  repository_id    TEXT,
  checkout_ref     TEXT,                       -- git rev-parse HEAD do path, se houver
  investigated_version TEXT,                   -- worker_version da ocorrência
  status           TEXT NOT NULL,              -- running (primeira passada) | completed | failed | cancelled | open (mode = chat)
  error            TEXT,
  turns            INTEGER,                    -- observados via harness::metrics; informação, não teto
  duration_ms      INTEGER,
  cost_usd         REAL,
  requested_by     TEXT,                       -- opaco; quem chamou investigate
  created_ms       INTEGER NOT NULL,
  finished_ms      INTEGER
);
CREATE UNIQUE INDEX sentinel_investigations_active
  ON sentinel_investigations (group_id) WHERE status = 'running';

CREATE TABLE sentinel_diagnoses (
  id               TEXT PRIMARY KEY,           -- dgn_<ulid>
  investigation_id TEXT NOT NULL REFERENCES sentinel_investigations(id),
  group_id         TEXT NOT NULL REFERENCES sentinel_groups(id),
  turn_id          TEXT,                       -- turno em que o agente chamou record (do contexto), quando disponível
  source           TEXT NOT NULL,              -- first_pass | conversation
  recorded_by      TEXT NOT NULL DEFAULT 'agent',
  diagnosis        TEXT,                       -- JSON DiagnosisV1 (NULL quando inválido)
  raw_result       TEXT,                       -- texto cru quando o schema falhou
  valid            INTEGER NOT NULL DEFAULT 1,
  verification     TEXT,                       -- JSON do ponto diagnosis_verification; NULL = não verificado
  created_ms       INTEGER NOT NULL
);
CREATE INDEX sentinel_diagnoses_group ON sentinel_diagnoses (group_id, created_ms DESC);

CREATE TABLE sentinel_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

-- Tier de decisões (opcional). Sem ele, estas tabelas ficam vazias e nada
-- muda: toda leitura tem o caminho determinístico como padrão.
CREATE TABLE sentinel_decisions (
  id            TEXT PRIMARY KEY,              -- dec_<ulid>
  point         TEXT NOT NULL,                 -- DecisionPoint
  subject_kind  TEXT NOT NULL,                 -- occurrence | group | span | log | diagnosis | investigation
  subject_id    TEXT NOT NULL,
  value         TEXT,                          -- JSON; NULL quando abaixo do limiar
  confidence    REAL NOT NULL,
  fell_back     INTEGER NOT NULL DEFAULT 0,
  model         TEXT,
  latency_ms    INTEGER,
  created_ms    INTEGER NOT NULL
);
CREATE INDEX sentinel_decisions_subject ON sentinel_decisions (point, subject_kind, subject_id);

-- Aresta de fusão: NUNCA muda o fingerprint, que segue sendo a identidade.
CREATE TABLE sentinel_group_edges (
  group_id       TEXT NOT NULL REFERENCES sentinel_groups(id),
  other_group_id TEXT NOT NULL REFERENCES sentinel_groups(id),
  kind           TEXT NOT NULL,                -- merge_candidate
  confidence     REAL NOT NULL,
  status         TEXT NOT NULL,                -- proposed | accepted | rejected
  decided_by     TEXT,                         -- opaco; quem aceitou ou recusou
  created_ms     INTEGER NOT NULL,
  PRIMARY KEY (group_id, other_group_id, kind)
);
```

"Sessões afetadas" é `COUNT(*)` em `sentinel_group_sessions` — uma linha por
sessão distinta, escrita no ingest e nunca podada junto com as ocorrências.
`COUNT(DISTINCT session_id)` sobre `sentinel_occurrences` mentiria assim que
a retenção de 1 000 linhas começasse a cortar.

### ⚠ `sentinel_transitions` (schema v2)

O grupo carrega só o presente. Um grupo que foi ignorado, designorado,
diagnosticado e então resolvido não mostra nada disso, e "quem decidiu isto, e
quando" é exatamente a pergunta de meses depois. Uma linha por movimento, na
mesma transação que o movimento — uma história que pode discordar do estado
que descreve é pior que nenhuma.

```sql
CREATE TABLE sentinel_transitions (
  id          TEXT PRIMARY KEY,
  group_id    TEXT NOT NULL REFERENCES sentinel_groups(id),
  from_status TEXT,                      -- NULL na linha que registra o nascimento do grupo
  to_status   TEXT NOT NULL,
  reason      TEXT,                      -- GroupChangeReason, quando houve uma
  actor       TEXT NOT NULL,             -- papel: ingest | agent | investigation | console
  at_ms       INTEGER NOT NULL
);
CREATE INDEX sentinel_transitions_group ON sentinel_transitions (group_id, at_ms DESC);
```

A poda diária mantém as 200 mais recentes por grupo, mais o nascimento: as
linhas são minúsculas, mas "minúsculo vezes para sempre" ainda é para sempre.

## Funções registradas

| Função | Papel | Trace |
|---|---|---|
| `sentinel::on-trace-activity` | handler do trigger `trace` | `trace_hidden` |
| `sentinel::on-log` | handler do trigger `log` | `trace_hidden` |
| `sentinel::on-turn-completed` | handler do trigger `harness::turn-completed` | `trace_hidden` |
| `sentinel::ingest` | consumidor da fila `sentinel-ingest` | `trace_hidden`, `internal` |
| `sentinel::diagnosis::record` | **chamada pelo agente** de dentro da investigação: grava um `DiagnosisV1` para o grupo | visível |
| `sentinel::groups::merge` · `unmerge` | aceita ou recusa uma proposta de fusão (só existe com o tier de decisões) | visível |
| `sentinel::groups::list` · `get` · `resolve` · `ignore` · `unignore` · `reopen` | superfície da UI | visível |
| `sentinel::occurrences::list` | superfície da UI | visível |
| `sentinel::evidence::get` | UI e harness | visível |
| `sentinel::trace::get` · `sentinel::logs::list` | proxies somente-leitura de `engine::traces::tree`/`spans` e `engine::logs::list` com o redator do ingest aplicado — o único caminho do agente para o engine vivo | visível |
| `sentinel::investigate` · `investigations::get` · `investigations::list` · `investigations::cancel` | superfície da UI | visível |
| `sentinel::status` | saúde e contadores | visível |
| `sentinel::ui-content` | ativos da UI injetada (crate `iii-console-ui`) | — |

Chamadas de saída para `database::*` e `queue::*` correm sob
`iii.tag.hidden = "sentinel store"` para não poluir a página de traces.

### Referência

#### `sentinel::groups::list`

```typescript
type GroupsListRequest = {
  status?: GroupStatus[];        // default: ["new","investigating","diagnosed","regressed"]
  service_name?: string;
  since_ms?: number;             // last_seen_ms ≥
  search?: string;               // substring em title / message_sample / function_id
  sort?: "last_seen" | "first_seen" | "count" | "priority"; // default "priority"
  offset?: number; limit?: number; // default 0 / 50, máx. 200
};
type GroupSummary = {
  id: string; fingerprint: string; source: ErrorSource;
  title: string; summary?: string;
  service_name: string; function_id?: string; exception_type?: string;
  status: GroupStatus; ignore_rule?: IgnoreRule;
  first_seen_ms: number; last_seen_ms: number; occurrence_count: number;
  sessions_affected: number;
  first_version?: string; last_version?: string; resolved_version?: string;
  regressed_at_ms?: number;
  sparkline: number[];           // 24 buckets horários, mais antigo primeiro
  has_diagnosis: boolean; active_investigation_id?: string;
};
type GroupsListResponse = { groups: GroupSummary[]; total: number };
```

`priority` ordena: `regressed` por `regressed_at_ms` desc, depois os demais
por `last_seen_ms` desc.

#### `sentinel::groups::get`

```typescript
type GroupGetRequest = { group_id: string };
type GroupGetResponse = {
  group: GroupSummary & { message_sample: string; previous_status?: string };
  latest_occurrence?: OccurrenceSummary;
  first_occurrence?: OccurrenceSummary;
  triage?: TriageV1;             // dica rotulada; nunca um diagnóstico
  diagnosis?: DiagnosisRecord;   // o vigente
  active_investigation?: InvestigationSummary; // primeira passada em curso
  latest_investigation?: InvestigationSummary; // para "Continue in chat"
  trace_available: boolean;      // engine ainda tem o trace da última ocorrência
};
type DiagnosisRecord = {
  id: string; investigation_id: string; session_id: string; turn_id: string;
  source: "first_pass" | "conversation"; model: string; created_ms: number;
  valid: boolean; diagnosis?: DiagnosisV1; raw_result?: string;
};
```

#### `sentinel::occurrences::list`

```typescript
type OccurrencesListRequest = { group_id: string; offset?: number; limit?: number };
type OccurrenceSummary = {
  id: string; at_ms: number; source: ErrorSource;
  trace_id?: string; span_id?: string; session_id?: string; turn_id?: string;
  worker_version?: string; message: string;
  has_evidence: boolean; settled: boolean;
};
type OccurrencesListResponse = { occurrences: OccurrenceSummary[]; total: number };
```

#### `sentinel::evidence::get`

```typescript
type EvidenceGetRequest = { occurrence_id: string };
type EvidenceGetResponse = { occurrence: OccurrenceSummary; evidence: EvidenceBundleV1 | null };
```

#### `sentinel::groups::resolve` · `ignore` · `unignore` · `reopen`

```typescript
type ResolveRequest  = { group_id: string; until_version_change?: boolean };
type IgnoreRequest   = { group_id: string; rule: IgnoreRule };
type UnignoreRequest = { group_id: string };
type ReopenRequest   = { group_id: string };   // resolved | ignored → new
type GroupMutationResponse = { group: GroupSummary };
```

Transições inválidas (resolver um `investigating`, ignorar um `resolved`)
respondem `sentinel/invalid_transition { from, to }`.

#### `sentinel::investigate`

```typescript
type InvestigateRequest = {
  group_id: string;
  mode?: "assisted" | "chat";    // default "assisted": primeira passada automática; "chat": só a sessão com a evidência
  model?: string;                // "provider::model" do catálogo, ou id cru com `provider`
  provider?: string;
  occurrence_id?: string;        // default: a mais recente com evidência
};
type InvestigateResponse = {
  investigation_id: string;
  session_id: string;            // a UI abre isto na coluna ao lado
  first_pass_turn_id?: string;
  existing: boolean;             // já havia uma primeira passada em curso: devolveu essa
};
```

Erros: `sentinel/no_model` (nem request nem config), `sentinel/no_evidence`,
`sentinel/harness_unavailable`.

#### `sentinel::groups::merge` · `unmerge`

Aceita ou recusa uma proposta de fusão do ponto `group_merge`. Só existe com o
tier de decisões ligado; sem ele nunca há aresta para agir.

```typescript
type GroupMergeRequest   = { group_id: string; other_group_id: string };
type GroupUnmergeRequest = { group_id: string; other_group_id: string };
type GroupMergeResponse  = { edge: { group_id: string; other_group_id: string; status: "accepted" | "rejected" } };
```

Aceitar **não** apaga nem reescreve grupo nenhum: a lista passa a mostrar os
dois como um só (contadores somados, a evidência de ambos disponível) e um
`unmerge` desfaz. Fingerprints, ocorrências e histórico ficam intactos nos
dois lados.

#### `sentinel::trace::get` · `sentinel::logs::list`

```typescript
type TraceGetRequest  = { trace_id: string; include_spans?: boolean };   // → engine::traces::tree (+ spans)
type TraceGetResponse = { trace: TraceTree | null; redactions: number };  // null = o trace já saiu do store
type LogsListRequest  = { trace_id?: string; service_name?: string; level?: string; since_ms?: number; limit?: number };
type LogsListResponse = { logs: StoredLog[]; redactions: number };
```

O único caminho do agente para o engine vivo: a resposta passa pelo redator
do passo 4 do ingest antes de sair, e a chamada corre sob `iii.tag.hidden`
como as demais chamadas de saída. `engine::traces::*` cru não está na lista
de allow — e está na de deny, por cinto.

#### `sentinel::diagnosis::record`

A função que o agente chama de dentro da sessão de investigação — e a única
escrita que a política da sessão permite. É registrada com o `DiagnosisV1`
como schema da requisição. ⚠ O `agent_trigger` **não** valida o payload contra
esse schema antes de despachar (ver acima); quem o valida é o `serde` do
request tipado, que recusa campo desconhecido e enum fora do conjunto.

```typescript
type DiagnosisRecordRequest = {
  group_id: string;
  diagnosis: DiagnosisV1;
  // sem investigation_id nem session_id: a identidade da sessão vem da baggage
  // OTel da invocação (iii.session.id), que o agente não controla
};
type DiagnosisRecordResponse = {
  diagnosis_id: string;
  version: number;               // 1 na primeira gravação da investigação, e daí em diante
  group_status: GroupStatus;     // diagnosed a partir de investigating/new; resolved, ignored e regressed ficam como estavam
};
```

Erros: `sentinel/no_investigation` (sem `iii.session.id` na baggage da
invocação, ou a sessão não é uma investigação), `sentinel/not_this_group` (a
investigação da sessão é de outro grupo). Ambos voltam ao agente como
resultado da função, então ele corrige e chama de novo — mas nenhum campo do
payload o deixa apontar para outra investigação.

O que **não** existe: uma função para "pedir" diagnóstico. *Ask for a
diagnosis* na UI é o console compondo uma mensagem na sessão
(`host.chat.compose`), e o agente responde gravando. Sentinel só escuta.

#### `sentinel::investigations::get` · `list` · `cancel`

```typescript
type InvestigationSummary = {
  id: string; group_id: string; occurrence_id: string;
  session_id: string; mode: "assisted" | "chat"; first_pass_turn_id?: string;
  model: string; provider?: string;
  repository_id?: string; checkout_ref?: string; investigated_version?: string;
  status: "running" | "completed" | "failed" | "cancelled" | "open";
  error?: string;
  turns?: number; duration_ms?: number; cost_usd?: number; // observados, informativos
  created_ms: number; finished_ms?: number;
};
type InvestigationGetResponse  = { investigation: InvestigationSummary; diagnoses: DiagnosisRecord[] }; // mais recente primeiro
type InvestigationsListRequest = { group_id?: string; status?: string[]; offset?: number; limit?: number };
type InvestigationCancelRequest = { investigation_id: string }; // → harness::stop na primeira passada
```

#### `sentinel::status`

```typescript
type StatusResponse = {
  enabled: boolean;
  engine: { trace_store: "memory" | "disabled" | "unknown"; logs: boolean };
  sources: { trace: boolean; log: boolean };
  ingest: {
    queued: number; processed_total: number; deduped: number;
    dropped_own_trace: number;      // raiz do trace é do Sentinel
    dropped_investigation: number;  // sessão sentinel-inv-*
    dropped_ignored_service: number;
    phantom_ticks_dropped: number;  // ticks dos próprios traces do ingest, filtrados antes da fila
    logs_pending_join: number; logs_unattributed: number;
    redactions: number;             // valores redigidos na captura, todas as fontes
    dropped_failed: number;         // esgotaram os retries
    paused_until?: number;          // breaker aberto
  };
  groups: { open: number; regressed: number; ignored: number; resolved: number };
  investigations: { running: number; open_sessions: number };
  decisions: {
    enabled: boolean;
    model?: string;
    armed: boolean;                 // false enquanto o circuit breaker está aberto
    points: Record<string, { decided: number; fell_back: number; p50_ms: number }>;
  };
  repositories: Array<{ id: string; path: string; exists: boolean; workers: string[] }>;
};
```

## Triggers

### Assinados pelo sentinel

| Tipo | Config | Handler |
|---|---|---|
| `trace` | `{ status: "error" }` | `sentinel::on-trace-activity` |
| `log` | `{ level: "error" }` | `sentinel::on-log` |
| `harness::turn-completed` | `{}` | `sentinel::on-turn-completed` |
| `cron` | diário | poda de buckets e arquivamento |

Os três primeiros são registrados no boot e re-registrados em reconexão
(padrão `recover_bindings` do `security-scan`).

### Emitidos

| Tipo | Quando | Payload |
|---|---|---|
| `sentinel::group-changed` | grupo criado, contagem/estado mudou, regressão, ignore expirou | `{ op: "created" \| "occurrence" \| "status" ; group_id; status; previous_status?; occurrence_count; reason?: "regression" \| "ignore_expired" \| "resolved" \| "ignored" \| "diagnosed" \| "reopened" }` |
| `sentinel::investigation-changed` | investigação criada, começou, terminou | `{ investigation_id; group_id; status }` |

`group-changed` com `op: "occurrence"` é coalescido por grupo em janelas de
1 s — um incidente com mil ocorrências não vira mil eventos. É a superfície
para irmãos: alerta no Slack, abertura de issue, notificação — nenhum deles
mora aqui.

## Configuração

Registrada no worker `configuration` sob o id `sentinel`, com JSON Schema;
hot-reload em toda chave exceto `database`.

```yaml
enabled: true
sources:
  trace: { enabled: true }
  log:   { enabled: true, join_window_ms: 2000 }   # o log fica pending_join até o span chegar ou a janela vencer
ingest:
  breaker_failures: 5               # jobs falhando em sequência → pausa o consumidor
  breaker_cooldown_ms: 30000
fingerprint:
  identity_numbers: [HTTP, status, code, errno, exit, signal, port]   # números após estes marcadores não são mascarados
ignore_services: []                 # service_names nunca ingeridos (além do próprio sentinel)
attribute_allowlist: []             # chaves extras preservadas na evidência
service_aliases: {}                 # service.name do span → nome registrado do worker (ver Ingest)
redaction:
  patterns: []                      # regexes extras do projeto; os embutidos (bearer/JWT, sk-/ghp_/AKIA, PEM, user:pass@host, chave=valor sensível, e-mail) rodam sempre
evidence:
  max_bytes: 1048576
  settle_delay_ms: 5000
  message_max_bytes: 65536          # digest inline na mensagem da investigação
retention:
  evidence_per_group: 5
  occurrences_per_group: 1000
  buckets_days: 30
  resolved_ttl_days: 90
  cron: "0 0 3 * * *"               # seis campos, UTC — a poda diária
# As chaves `decisions` e `triage` abaixo pertencem aos tiers opcionais e
# **não estão no schema do v1**: entram junto com o tier, num plano próprio.
decisions:                          # tier opcional (ver "Tier de decisões"); desligado = o comportamento descrito no resto desta spec
  enabled: false
  model: ""                         # "provider::model" com supports_decisions; via router::decide
  timeout_ms: 400                   # estourou → heurística, sempre; nunca re-tenta no ingest
  cooldown_ms: 60000                # 5 falhas seguidas desarmam o tier por este tempo
  points:                           # cada ponto liga por conta própria, com o seu limiar; abaixo dele a heurística decide
    origin_span:             { enabled: true,  min_confidence: 0.80 }
    log_join:                { enabled: true,  min_confidence: 0.85 }
    group_membership:        { enabled: true,  min_confidence: 0.90 }   # proposta; só marca dúvida
    group_merge:             { enabled: true,  min_confidence: 0.90 }   # proposta; nunca funde sozinho
    evidence_retention:      { enabled: true,  min_confidence: 0.60 }   # errar custa uma amostra pior
    evidence_pruning:        { enabled: false, min_confidence: 0.70 }   # o único que roda um map sobre a árvore inteira
    triage:                  { enabled: true,  min_confidence: 0.75 }
    investigation_preflight: { enabled: true,  min_confidence: 0.70 }
    diagnosis_verification:  { enabled: true,  min_confidence: 0.00 }   # exibição: sempre mostra o número
triage:
  model: ""                         # vazio = desligado. Título/resumo, hipótese rotulada, onde olhar. Nunca diagnóstico.
investigation:
  model: ""                         # "provider::model"; a UI pode sobrescrever por chamada. Sem tetos: o usuário assiste e para.
  provider: null
repositories:                       # service_name → checkout local (v1: só path)
  - id: workers
    path: /home/me/workspaces/workers
    workers: [harness, ade, session-manager, context-manager, llm-router, state, queue]
  - id: iii
    path: /home/me/workspaces/iii
    workers: [iii]                  # built-ins do engine reportam service_name "iii"
database: primary
workers_ttl_ms: 60000
archive:
  bucket: ""                        # opcional: bucket do storage para cópias JSON
```

Validação no boot e em cada reload: `path` absoluto e existente (aviso, não
erro — um path ausente desabilita a investigação com código para os workers
daquele repositório e `status.repositories[].exists = false`), um worker em no
máximo um repositório.

O passo 1 do produto — "o usuário escolhe um modelo" — é o formulário de
configuração registrado no console (`host.configForms.register("sentinel",
…)`), com o seletor alimentado por `router::models::list` e atualizado em
`router::models::changed`, no mesmo padrão do `security-scan`. ⚠ A escolha é
gravada como o **par** que o schema já tem (`investigation.model` +
`investigation.provider`), não como um `provider::id` que o worker teria de
partir; e um modelo gravado que sumiu do catálogo é **mantido e rotulado**,
nunca apagado em silêncio — um provider cujo credencial caiu volta.

## Console UI

Página injetada (`iii-console-ui`, ativos `sentinel/page.js` e
`sentinel/styles.css`), registrada com `host.pages.register`. Três vistas:

> ⚠ **O que o v1 entregou desta seção.** Tudo, com uma exceção: o renderer da
> evidência em modo chat, bloqueado por falta de consumidor no console (ver
> "A evidência no transcript"). A aba *History* é lida de uma tabela real —
> ⚠ `sentinel_transitions`, schema v2, uma linha por movimento escrita na
> mesma transação que o faz — e não derivada dos campos do grupo. O ator
> gravado é um **papel** (`ingest`, `agent`, `investigation`, `console`) e
> não uma pessoa: o console não entrega identidade de usuário, e o
> `_caller_worker_id` é um uuid que pareceria uma.

**Lista de grupos.** Filtros: estado (padrão: abertos — `new`,
`investigating`, `diagnosed`, `regressed`; chips para `regressed`, `ignored`,
`resolved`), worker, janela (24 h / 7 d / 30 d / tudo), busca. Colunas:
título (com `exception_type` em destaque), worker · função, contagem, sessões,
primeira/última, sparkline 24 h, estado. Linha `regressed` com marcação
própria. Ações em lote: resolver, ignorar.

**Detalhe do grupo.** Cabeçalho com título, estado, versões, contagem e as
ações **Investigate ▾** (um clique com o modelo padrão; o menu oferece
*Investigate with…* para trocar o modelo e *Open in chat* para o modo sem
primeira passada), **Resolve ▾** (`now` / `until version change`),
**Ignore ▾** (`forever` / `N occurrences` / `until version change`). Abaixo
do cabeçalho, quando houver triagem, a **dica** discreta: "untested
hypothesis — …", com *where to look* em mono. Abas:

- *Latest occurrence* — o span de origem com `status_description` e o evento
  `exception` (stack em bloco), o caminho de ancestrais, os spans com erro, os
  logs do trace. Botão **Open trace in console** quando `trace_available`;
  senão o rótulo "snapshot de <hora>".
- *Occurrences* — lista paginada com versão, sessão (link para a conversa),
  turno, disponibilidade de evidência.
- *Diagnosis* — enquanto a primeira passada roda: "live in the chat beside"
  (ou "running — open the session to watch", com **Open session**, quando a
  coluna está fechada), **Stop**, e a dica da triagem. Depois: render do `DiagnosisV1` vigente
  (resumo, categoria, confiança, evidências com link para o arquivo via
  página do `ide` quando o repositório está mapeado, correção proposta, o
  que faltou), o painel da investigação (modelo, turnos e duração observados,
  versão investigada vs. checkout) com **Continue in chat** e **Update
  diagnosis**, e a lista de diagnósticos anteriores (`source`, quando,
  confiança).
- *History* — transições de estado, com quem/quando quando o console
  fornecer identidade.

**Investigação ao lado.** *Investigate* abre a sessão na segunda coluna da
aba do workspace (`host.chat.selectConversation`); a página do grupo fica na
primeira. O usuário vê o transcript ao vivo — a mensagem de evidência, os
`triggered ƒ coder::read-file …`, as hipóteses — e digita no composer da
sessão para intervir. Não há diálogo de orçamento: o único diálogo é o
seletor de modelo, e só quando o usuário pede *Investigate with…*.

**Fechar a coluna não é parar.** A investigação vive no harness e em
`sentinel_investigations`; a coluna é só uma vista dela. Fechar a coluna, ou
voltar à lista, ou abrir outro grupo, esconde a sessão e **nada mais**: a
primeira passada continua, o `record` chega do mesmo jeito, o estado
`investigating` segue na lista e no cabeçalho. O cabeçalho de um grupo com
investigação ativa e coluna fechada mostra **Open session** (com o ponto "ao
vivo" enquanto roda), que reabre a mesma sessão
(`host.chat.selectConversation` com o `session_id` de
`active_investigation_id`); *Continue in chat* na aba Diagnosis faz o mesmo
para a sessão já concluída. Só **Stop** encerra a primeira passada
(`harness::stop`) — e mesmo então a sessão fica, com *Open session* ainda
disponível para continuar na mão.

**Configuração.** Formulário do worker `configuration`: modelo de
investigação, triagem (liga/desliga e modelo), fontes, repositórios (id,
path, workers), retenção. Sem campos de orçamento.

**A evidência no transcript.** Em `mode: "chat"` a evidência é uma entrada
`user` com ⚠ `origin.sentinel_evidence: true`. ⚠ **O renderer que a rotularia
como do Sentinel não existe no v1**: `host.chat.registerTranscriptRenderer` só
está declarado como tipo (`packages/console-ui/index.d.ts:550`) e não há
consumidor em `ade/web/src`, então o console exibe a entrada como fala do
usuário. O cabeçalho `## Sentinel · evidence — <título>` no próprio texto é o
que a identifica, e o renderer vira follow-up no `ade`. A chamada
`sentinel::diagnosis::record` **tem** renderer (`host.functionTriggers`) e
aparece no transcript como a conclusão que é. A chamada `sentinel::diagnosis::record`
aparece no transcript como qualquer `triggered ƒ`, com um marcador do
Sentinel; a aba Diagnosis atualiza no mesmo instante. *Ask for a diagnosis*
compõe "grave o que você tem até agora" na sessão — não há estado
intermediário para mostrar, porque não há turno especial.

**Onde o tier de decisões aparece.** Em nenhum lugar novo: ele marca o que já
existe. Na aba *Diagnosis*, a confiança verificada fica ao lado da
auto-declarada e, quando discordam, a UI diz as duas — nunca esconde o card.
Na lista, um grupo com proposta de fusão ganha um marcador discreto com a
outra ponta e os botões aceitar/recusar. Numa ocorrência com
`membership_doubt` alto, a linha traz "pode não ser deste grupo". Com o tier
desligado, nada disso é renderizado e a página é a mesma.

**Atualização ao vivo.** A página assina `sentinel::group-changed` e
`sentinel::investigation-changed` via `host.iii.registerTrigger` e refaz a
consulta da vista aberta com coalescência (o mesmo modelo notify-then-query
da página de traces); nunca aplica o payload do evento como estado.

**Deep link para Traces.** `host.panels.open({ pageId, context })` só abre
páginas de extensão registradas; a tela Traces é built-in do console
(`openWorkspaceScreen('traces')` em `ade/web/src/App.tsx`) e o `Host` não expõe
navegação para ela com parâmetros. Duas saídas, a decidir na entrega da UI:
adicionar ao `ade` uma navegação de host para telas built-in com contexto
(`{ screen: "traces", trace_id }`), ou aceitar que o sentinel renderiza o
trace a partir do próprio snapshot — que ele precisa fazer de qualquer forma
para o caso em que o trace já sumiu. A primeira é a experiência certa; a
segunda é o mínimo para o v1 funcionar sem tocar no console.

⚠ **Decidido na entrega: a segunda.** A página renderiza a árvore a partir do
bundle congelado e marca com um chip quando o trace já saiu do anel. Nenhuma
mudança no `ade`, e o caminho que importa — "o trace sumiu, e mesmo assim dá
para ver o que aconteceu" — é o mesmo caminho nos dois casos.

## Estado e durabilidade

- **Fila** (`queue`): `sentinel-ingest` (FIFO por `trace_id`, concorrência 4,
  `max_retries 3`, `backoff_ms 1000`, `redeliver_on_engine_restart: true`).
  ⚠ Falha em definir **não** é falha de boot: é retry até existir, com o
  ingest fechado enquanto isso (ver a fase dois abaixo). FIFO exige um campo
  escalar de agrupamento, então ⚠ **todo job carrega `trace_id` no topo** —
  `IngestJob { kind: trace | log | settle | promote, trace_id, … }`, com `"-"`
  quando um log não tem trace. Um trace é processado em ordem, e a janela de
  junção vira uma espera em vez de uma corrida. Investigações não passam por
  fila: são um `harness::send`, e o harness já é durável.
- ⚠ **Prontidão é fazer o trabalho, não consultar o registro.** A sonda
  óbvia — perguntar ao `engine::functions::info` se `database::execute`
  existe — responde a outra pergunta: um id nu resolve entre namespaces, então
  ela diz "sim" para uma função registrada em *outro* namespace enquanto toda
  chamada real falha, porque chamada roteia para o namespace do chamador.
  Migrar o schema e definir a fila **são** a sonda, e são as operações que o
  worker precisa de qualquer forma; a mensagem de falha nomeia o problema real
  (`UNKNOWN_DB`, namespace errado) em vez de um "não encontrado" genérico.
- ⚠ **Transações são one-shot, e transições são compare-and-set.** A forma
  interativa (`beginTransaction`/…/`commit`) prende uma conexão SQLite entre
  RPCs; com quatro jobs de ingest em voo ela serializa o pipeline inteiro
  atrás do job mais lento. Cada escrita é uma única `database::transaction`
  carregando todos os seus statements, e o `UPDATE` do grupo leva o
  `updated_ms` que o snapshot viu e devolve o id que mudou
  (`… WHERE id = ? AND updated_ms = ? RETURNING id`). Nenhuma linha de volta
  significa que alguém moveu o grupo no meio — uma pessoa resolvendo enquanto
  uma ocorrência caía — e o escritor relê e decide de novo. É assim que "um
  registro nunca desfaz uma decisão humana" sobrevive à concorrência em vez de
  ser um comentário numa função.
- **Boot, em duas fases.** Primeiro a **interface**: config (dependência
  obrigatória) → tipos de trigger → funções → UI → doorbell de configuração.
  Depois, em tarefa de fundo, as **dependências duráveis**: cria tabelas/migra
  → define filas → `ready` → registra os triggers de fonte → reconcilia
  investigações `running` → loop de 30 s (reconcile + recover de bindings).
  A ordem não é estética: a captura de interface do release sobe este binário
  contra um engine isolado **sem `database` e sem `queue`** e exige superfície
  não vazia em segundos — reclamar o store antes penduraria a captura e
  esconderia o worker do registro. Enquanto a fase dois não termina o worker
  está *de pé* mas não *habilitado*: `sentinel::status.enabled` é falso,
  entregas de ingest são recusadas com `sentinel/not_ready` (a fila reentrega)
  e contadas em `ingest.dropped_not_ready`. Definir a fila não é falha de
  boot; é retry até existir, visível no status.
- **Configuração inválida não é o mesmo que configuração indisponível.** O
  worker retenta para sempre quando o `configuration` não responde, mas um
  valor *armazenado* que não valida é permanente: retentar seria
  indistinguível do primeiro caso enquanto o operador espera um worker que
  nunca sobe. Nesse caso o boot segue com os **defaults, desabilitado**, e
  `status.config_error` nomeia o campo recusado; o doorbell cura assim que a
  entrada for corrigida. O mesmo vale para um reload: o valor recusado não
  substitui o vigente.
- **Idempotência**: ingest por `dedupe_key`; investigação por
  `idempotency_key` no `harness::send` e pelo índice único de ativa.
- **Restart no meio de um ingest**: a mensagem volta à fila; o `dedupe_key`
  faz o reprocessamento ser no-op para o que já foi gravado.
- **Perda do trace entre o tick e o ingest**: `traces::list` devolve vazio
  para aquele id; o job termina sem ocorrência e incrementa
  `ingest.lost_before_capture` no status. Sob uma tempestade que estoura os
  10 000 spans do engine, essa métrica é o sinal para o operador subir
  `memory_max_spans`.

## Dependências

```yaml
dependencies:
  database: "latest"        # grupos, ocorrências, evidência, investigações
  queue: "latest"           # ingest e investigação duráveis
  configuration: "latest"   # config com hot-reload
  cron: "latest"            # poda diária
  harness: "latest"         # investigação
  session-manager: "latest" # modo chat: session::ensure + session::append
  ide: "latest"             # coder::* (leitura de código) — já vem com o harness
  iii-observability: "latest"
  iii-stream: "latest"
```

`storage` é opcional (`archive.bucket`). Não há `approval-gate` nesta
arquitetura — permissão é sempre `full`, e a fronteira é a política do
harness (risco #8). O tier de decisões é opcional: ele precisa de `router::decide` no `llm-router` e, sem ele,
`decisions.enabled` fica falso e a spec vale como está escrita.

## Fora de escopo do v1

Cada item abaixo é um irmão ou uma versão seguinte, não uma lacuna:

- **Investigação automática** (por limiar de ocorrências, por regressão) —
  sem ninguém olhando, exigiria os tetos e a supressão de picos que o v1
  deliberadamente não tem.
- **Correção**: patch, worktree, PR — a decisão do v1 é diagnóstico apenas.
- **Issue no Linear** — irmão que assina `group-changed` /
  `investigation-changed` e segue o protocolo `linear-bug-reporting`.
- **Alertas** (Slack, Telegram, e-mail) — irmãos sobre `group-changed`.
- **Fontes `harness-turn` e `report`** — adapters já desenhados, não
  implementados.
- **Fusão automática de grupos** e fingerprint por stack trace — o
  normalizador determinístico primeiro; medir a taxa de grupos duplicados
  antes de mudar a identidade. O [tier de decisões](#tier-de-decisões-opcional)
  chega perto disso com `group_merge`, mas só como proposta revisável: a
  identidade continua sendo o `sha256`, sempre.
- **Repositórios remotos / `worktree`** — v1 é path local; o diagnóstico
  declara a deriva de versão.
- **Multi-engine** — um sentinel observa um engine.
- **Amostragem** por grupo além da retenção de evidência.

Adiados **com desenho esboçado**, para não se perderem:

- **Degradação sob tempestade.** O fingerprint precisa de `exception.message`
  ou `status_description`, que o `TraceSummary` não traz, então cada trace com
  erro custa um `traces::tree` só para agrupar. Sob 50 erros/s a fila enche e
  o trace evapora antes do job — a spec mede (`lost_before_capture`) e não
  reage. O esboço: `traces::spans { trace_id, status: "error" }` (para
  `spans` o filtro é substring no status) devolve só os spans de erro e serve
  para o fingerprint; `tree` só na etapa de evidência, amostrada quando a fila
  passa de um limiar.
- **Falhas de built-ins do engine.** O trigger `trace` exclui
  `iii.function.kind=internal` no subscriber, então uma falha em
  `configuration::*` só aparece se propagar a um span de worker. O `iii` como
  repositório mapeado é, por isso, quase sempre vazio no v1. O caminho é um
  `include_internal` no config do trigger — mudança no engine.

## Fronteiras

- O sentinel **lê** telemetria; nunca escreve spans ou logs de negócio no
  engine além dos seus próprios, e esses são `trace_hidden`.
- O sentinel **nunca chama** `shell::*`, `worktree::*`, `github::*`. A sessão
  de investigação também não — a política de funções é fail-closed e está no
  `send`.
- **Nunca ingere a si mesmo**: traces cuja **raiz** é do Sentinel e sessões
  `sentinel-inv-*` são descartados antes de qualquer outra regra — pela
  raiz, porque o erro que o Sentinel provoca aparece no worker que ele
  chamou, não nele.
- Segredos não atravessam: atributos com nomes sensíveis são descartados e
  **todo valor é redigido na captura**, antes de persistir e antes de
  qualquer modelo; o agente só alcança o engine vivo pelos proxies
  redigidos; o system prompt proíbe reproduzir credenciais — terceira
  barreira, não a primeira.
- Conteúdo de trace, log e repositório é **dado não confiável** para o
  agente — nunca instrução.
- Só o humano resolve. O harness diagnostica; um diagnóstico não muda o
  estado além de `investigating`/`new → diagnosed`, e nunca desfaz
  `resolved`, `ignored` ou `regressed`.
- **A única escrita do agente é o próprio diagnóstico**, por
  `sentinel::diagnosis::record`, e só para o grupo que a sessão investiga.
  Nenhuma outra função do Sentinel está na lista de allow.

## Riscos e questões abertas

| # | Questão | Efeito | Encaminhamento |
|---|---|---|---|
| 1 | `service_name` dos spans ≠ nome registrado do worker em algum SDK | versão errada / repositório não mapeado | verificar os defaults dos três SDKs na implementação; fallback: mapa `service_aliases` na config |
| 2 | ~~Precedência allow/deny do harness~~ — **resolvido**: deny vence (`policy.rs:112`); a spec nega ids explícitos | — | teste de manifesto garante que função nova do sentinel entra no deny |
| 3 | Deep link para a tela Traces com `trace_id` — o `Host` não navega para telas built-in com contexto | botão "Abrir trace" sem destino | decidir na entrega da UI: navegação de host no `ade` (certa) ou só o render do snapshot (mínimo) |
| 4 | Volume de linhas em `sentinel_occurrences` numa tempestade (10³/min) | SQLite sob escrita concorrente | `occurrences_per_group` limita; ingest em transação por trace; medir |
| 5 | Ticks para traces pendentes: o span de erro fechou, a raiz não | bundle capturado incompleto | settle único em +5 s; `settled` visível na UI |
| 6 | Erros que só aparecem como span `error` sem `exception` nem `status_description` | fingerprint pobre (`service + function`) | aceitável no v1; a UI mostra o span cru; medir a proporção |
| 7 | `router::models::list` pode listar modelos sem capacidade de JSON output | investigação falha no schema | filtrar por `router::models::supports` no seletor, quando exposto |
| 8 | **Não há gate; permissão é sempre `full`.** Nenhuma chamada do agente pede aprovação humana | uma escrita externa na lista de allow rodaria sem ninguém olhar | a lista de allow da investigação é o modelo de permissão inteiro e nunca contém escrita fora do Sentinel; tickets em sistemas externos são ação humana na UI ou regra determinística — nunca iniciativa do agente |
| 9 | Sem tetos do Sentinel, uma primeira passada esquecida numa aba fechada roda até o `max_turns` do harness | gasto sem ninguém olhando | é aceito pelo desenho (o teto do harness existe); a lista mostra `investigating` ao vivo e o *Stop* fica a um clique |
| 10 | `router::decide` não existe no `llm-router` | o tier de decisões não tem por onde entrar | é a pré-condição do tier; até lá `decisions.enabled: false`. O router já tem `supports_structured_output` no catálogo e o código de erro correspondente — falta a superfície e um `supports_decisions` |
| 11 | Determinismo do modelo de decisão desconhecido (mesma entrada → mesma saída?) | uma releitura poderia contradizer o que a UI mostrou | já mitigado por desenho: decisões são persistidas em `sentinel_decisions`, nunca recalculadas |
| 12 | Deriva de calibração ao longo do tempo, ou fallback alto e silencioso | o tier parece ligado e nunca decide | taxas de `fell_back` e `p50_ms` por ponto no `sentinel::status`; discordância de `diagnosis_verification` exibida na UI |
| 13 | Latência do ponto `evidence_pruning` num map de 200 spans | ingest lento sob carga | desligado por padrão; medir antes de ligar, e o `timeout_ms` corta para a heurística de qualquer forma |
| 14 | **Adiado.** Sem caminho de degradação sob tempestade: fingerprint exige `traces::tree` por trace | fila enche, trace evapora, `lost_before_capture` sobe | esboço em [Fora de escopo](#fora-de-escopo-do-v1): fingerprint via `traces::spans { status: "error" }`, `tree` amostrado |
| 15 | ~~Mensagens, `status_description` e corpos de log seguem crus para o provedor~~ — **resolvido**: redação de valores na captura (Ingest, passo 4), antes de persistir e de qualquer modelo; o engine vivo só chega ao agente pelos proxies `sentinel::trace::get` / `logs::list` | — | o que resta é o risco 23 |
| 16 | Falhas de built-ins do engine são invisíveis ao trigger `trace` (`iii.function.kind=internal` excluído no subscriber) | "todo worker do engine" promete o que o repositório `iii` não entrega | dito na spec; caminho é `include_internal` no config do trigger — mudança no engine |
| 17 | ~~`internal: true` no registro estampa `iii.function.kind=internal`?~~ — **resolvido: não.** O engine só estampa para built-ins (`engine/src/invocation/mod.rs:129`) | os ticks fantasmas do ingest não somem sozinhos | o anel de `trace_id`s é **obrigatório**, não cinto |
| 18 | Qual atributo/resource do span carrega o namespace do worker | sem ele, uma função registrada em dois namespaces só resolve pelo `service_name` do span | verificar nos três SDKs; o mapa é chaveado por `(namespace, function_id)`, e o que não resolve fica `namespace_ambiguous` em `namespace = "?"` — nunca decidido por ordem de chegada |
| 19 | `host.chat.selectConversation` alcança uma sessão `kind: automation` que o sidebar filtra no cliente? | a peça central da UI — abrir a sessão ao lado — pode não abrir | verificar no `ade` antes de qualquer UI. Se não alcançar, a correção é **no console** (`selectConversation` carrega a sessão por id, independente do filtro) — **nunca** criar a sessão como `user` para que ela apareça na lista |
| 20 | **Sem decisão.** Versão auto-reportada é estática em dev (`0.23.1-dev` por semanas) ou ausente (`Option<String>`) | `until_version_change`, ignorar-até-a-versão e regressão por versão nunca disparam — no alvo do v1 | candidatos: `git rev-parse HEAD` do repositório mapeado, ou `connected_at_ms`/`id` do worker como "build mudou" |
| 21 | **Sem decisão.** "Primeira + 5 mais recentes" guarda 5 bundles dos últimos 5 minutos, mesma versão | retenção quase inútil para regressão sem o tier de decisões | ganho barato: um bundle por `worker_version` distinta além de primeira + últimas |
| 22 | A baggage OTel não é autenticada: um worker do engine pode invocar `sentinel::diagnosis::record` num escopo com `iii.session.id` forjado | um diagnóstico gravado em nome de uma investigação por código que não é o agente | aceito: só código de worker no engine invoca funções, e esse código é confiável; o agente — a única parte não confiável — não escreve baggage, e o payload não oferece atalho nenhum |
| 23 | O redator é uma denylist de padrões; um segredo num formato desconhecido passa | vazamento residual para o provedor | `status.ingest.redactions` mostra o redator trabalhando; `redaction.patterns` recebe os formatos do projeto; o system prompt continua proibindo reproduzir credenciais — terceira barreira, não a primeira |
