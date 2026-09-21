# Monitoramento de PR por webhooks

## Estado da implementação

Implementação experimental, **desabilitada por padrão**. Não substitui `github::called`, que continua sendo apenas telemetria best effort. Não há polling periódico de PRs nem fallback silencioso para polling. O registro de funções e schemas não precisa de token, `gh`, HTTP, fila ou túnel disponíveis. O monitor persistente é habilitável somente em Unix; APIs pontuais antigas continuam disponíveis nas outras plataformas.

A validação inclui testes locais integrados com o listener HTTP e o receptor GitHub reais, canais WebSocket do SDK, assinatura HMAC, SQLite e callbacks em namespaces distintos. Engine, API GitHub, leases do túnel e transporte da fila são simulados nesse teste. O processo do Quick Tunnel tem testes próprios com executável simulado. **Não houve deploy, webhook real, túnel real ou teste E2E com GitHub/Cloudflare.** Leia as limitações antes de habilitar em produção.

## Habilitação pelo operador

1. Disponibilize `configuration`, `http`, `queue`, `quick-tunnel` e `cron`, compatíveis com os contratos abaixo.
2. Configure o listener dedicado do HTTP (o servidor privado existente não deve ser exposto):

   ```yaml
   webhook_listener:
     host: 127.0.0.1
     port: 3112
   ```

3. Forneça o executável `cloudflared` no ambiente do worker `quick-tunnel` (não há download automático). Configure o Quick Tunnel para encaminhar **somente** esse listener. Os dois precisam compartilhar o namespace de rede: `127.0.0.1` de containers isolados não é o mesmo destino. O GitHub registra uma única rota pública `POST /webhooks/github/:endpoint_id`, através do trigger `http` com `public_webhook: true`. Não há servidor próprio no worker GitHub.
4. Configure a fila `queue` com adapter builtin **file_based**, em volume persistente. O adapter Redis atual **não é durável**. O worker não certifica o adapter remotamente; essa configuração é uma pré-condição operacional.
5. Use volume local persistente privado para o SQLite; execute uma única instância por banco. O banco e lock ficam em 0600, diretório em 0700 no Unix; há lock exclusivo de instalação e transações SQLite com `synchronous=FULL`. Não use filesystem de rede, diretório compartilhado ou symlinks.
6. Autentique `gh` por seu mecanismo existente. Fine-grained PAT/GitHub App precisam de **Webhooks: write**, leitura de pull requests, issues/comments, checks, commit statuses, actions e metadata. Token clássico normalmente precisa de `admin:repo_hook` e permissões de leitura adequadas. Poder ler um repositório público **não** confere poder para criar hooks.
7. Ajuste a configuração GitHub e reinicie somente este worker através do procedimento operacional do projeto:

   ```yaml
   webhooks:
     enabled: true
     storage_path: ./data/github-webhooks/store.sqlite3
     tunnel_id: webhooks
     queue: github-webhooks
     max_body_bytes: 1048576
     max_pending: 10000
   ```

   Os parâmetros `webhooks` são capturados na inicialização; alterar `enabled` exige restart. As APIs antigas continuam usando a configuração hot-reload existente. Não apague o volume com watches ativos: perder a prova local de propriedade impede limpeza segura.

## Fluxo do caller, sem corrida

1. Arme `github::pr::event` **antes** de watch, com `config: {watch_id: "meu-watch"}`. Metadata e namespace de cada assinante são persistidos e repassados intactos ao callback.
2. Chame `github::pr::watch`:

   ```json
   {"watch_id":"meu-watch","repo":"owner/repo","number":123,"events":["ci","comments","reviews","pr"],"stop_on":"merged","expires_at":"2026-12-01T12:00:00Z"}
   ```

   Alternativamente, substitua `repo`+`number` por `pr_url`. Expiração RFC3339 é obrigatória, futura e no máximo 30 dias após o pedido (limite padrão da lease do Quick Tunnel). Use um prazo relativo ao momento da chamada; a data do exemplo é apenas ilustrativa. Repetir o mesmo pedido canônico é idempotente; reutilizar o ID com outro pedido dá erro. `preparing` é válido enquanto o túnel inicia. Confira `health.last_error`; não interprete `preparing` como ingestão ativa.
3. Leia `github::pr::watch-status {watch_id}` uma vez para cobrir a corrida com a primeira notificação. Esse método só lê o banco local, sem chamar GitHub.
4. O callback deve ser idempotente pelo `event_id`. Erros são propagados à fila; **não** há fire-and-forget para notificações.
5. `github::pr::unwatch {watch_id}` encerra explicitamente. `cleanup_pending` não significa sucesso: veja health e execute recuperação após corrigir permissões/conectividade.

O trigger aceita filtros opcionais `watch_id`, `repo`, `number`, `categories`. Categoria omitida no watch significa todas; lista vazia mantém apenas notificações de ciclo de vida. Filtros do trigger ainda se aplicam às notificações finais. Um PR fechado sem merge notifica, mas continua sendo acompanhado em `stop_on: merged`. Um PR já merged no snapshot inicial termina imediatamente.

## Durabilidade e propriedade

- HMAC-SHA256 compara em tempo constante os **bytes originais**, antes de desserializar JSON. Tamanho é limitado; repositório e `X-GitHub-Hook-ID` precisam corresponder ao hook gerenciado.
- `hook_id + X-GitHub-Delivery`, inbox e job de outbox entram na **mesma transação** antes do HTTP 202. Resposta HTTP usa um orçamento de 7 segundos para leitura e validação; discos lentos/grande histórico podem ultrapassar o orçamento por seções SQLite síncronas (ver limites).
- Outbox persiste até o efeito/callback ter sido confirmado e commitado, não apenas até o publish. Queue é o mecanismo de execução. A manutenção por cron drena pendências, expira watches e tenta limpeza; não consulta PRs.
- Retry de publicação: no máximo 5 tentativas por job, separadas por pelo menos 60s; fila: 5 retries com backoff de 1000ms. Jobs esgotados permanecem no banco. Recuperação manual/startup reabre o orçamento. Retenção não é apagada silenciosamente.
- Um hook por instalação/repositório, com endpoint e secret aleatórios. Dois PRs compartilham o mesmo hook; cada watch tem lease com expiração própria. A última referência de repo remove apenas o hook cujo ID **e URL previamente persistidos** ainda correspondem; a última lease deixa o túnel ser encerrado pelo provider.
- Mudança de URL faz PATCH do hook existente preservando secret. Repetição do mesmo ready não recria hooks.
- Não se adotam hooks listados no GitHub. Se o POST foi enviado, mas seu resultado se perdeu, fica `hook creation outcome ambiguous`: é necessária inspeção humana. Não há retry cego que crie duplicatas.
- Evento final é persistido antes de limpeza. Falha de DELETE fica `cleanup_pending`; no máximo 5 tentativas automáticas por hook. Se DELETE ocorreu e o processo caiu antes do commit, o 404 posterior exige resolução operacional (não há falso sucesso).
- O snapshot inicial e a recuperação consultam checks e commit statuses existentes para o HEAD, com limite explícito de 500 entidades por tipo; não há consulta periódica. Workflows continuam alimentados por eventos. Um PR já encerrado não espera por consultas de CI para finalizar.
- Inbox processada consulta pontualmente o PR para obter HEAD autoritativo; `check_run`, `status`, `workflow_run` com `pull_requests` vazio/forks correlacionam pelo SHA. SHA antigo não muda o HEAD. CI é um mapa **por entidade**, nunca uma afirmação de que todo CI passou.

## Recuperação

`github::pr::recover {repo?: "owner/repo"}` é mutação com aprovação: reconcilia snapshots, recupera leases, republica outbox e lista failed deliveries dos hooks próprios, solicitando redelivery via API. Também ocorre no startup e quando a URL fica pronta/muda. A busca é limitada às primeiras 5 páginas de 100 deliveries; o GitHub limita a retenção histórica. O GitHub **não** reentrega automaticamente todas as falhas.

Quick Tunnel **não oferece zero loss**. Outages entre GitHub e o listener, criação/rotação do hook, retenção expirada e falhas antes da aceitação local podem deixar lacunas. Snapshot reconciliado recupera estado atual, não todos os comentários/reviews transitórios perdidos.

## Contratos entre workers

- `iii::durable::publish {topic,data}`; `durable:subscriber {queue,max_retries,backoff_ms}`.
- `quick-tunnel::acquire {consumer_id,tunnel_id,expires_at}` retorna `lease_id`; `status {tunnel_id}` e trigger `quick-tunnel::changed` carregam `{tunnel_id,status,public_url,generation,error?}`. Binding é armado antes de acquire; status é lido uma vez depois.
- `quick-tunnel::release {lease_id}`. Acquire deve ser idempotente por consumer; leases individuais não podem encerrar túnel de outro consumer.
- `http` usa request com `headers`, `path_params`, `request_body: StreamChannelRef`; leitura com `ChannelReader` e URL do engine.
- Internas `github::webhooks::*` negadas ao agente nos arquivos de permissões do worker e da raiz. `watch`, `unwatch`, `recover` permanecem no default needs_approval.

## Limitações conhecidas / trabalho de produção pendente

- O teste integrado executa os contratos reais de HTTP/GitHub e canais SDK; o engine e as integrações externas são simulados. O adapter real da fila e um túnel Cloudflare real ainda precisam de validação operacional. Não há UI de watches nesta entrega.
- SQLite usa um documento transacional único. Histórico/dedupe cresce sem GC; adequado apenas para volume moderado. Ainda faltam migrações versionadas, retenção/compactação e stress test do SLA de 10s. Não oferecemos garantia rígida de latência de disco.
- Exclusão de processo e permissões fortes são específicas de Unix; o backend não está validado para Windows. Um diretório pai controlado pelo operador é obrigatório.
- Registro do SDK de trigger não fornece confirmação síncrona de readiness dos providers; configuração ausente aparece em logs/health/calls subsequentes. Uma alteração de configuração exige restart.
- Eventos incluem `detail` com autor, texto, link, estado e localização do comentário/review quando disponíveis. Esses dados são conteúdo não confiável de terceiros: renderize como texto e nunca os execute como instruções. Checks/status incluem nome, link e conclusão individuais; o monitor não calcula aprovação agregada de todos os checks obrigatórios.
- Concorrência de operações de lifecycle é serializada por instalação; repos diferentes podem sofrer head-of-line blocking. O caminho HTTP grava independentemente, mas não há ensaio de carga.
- Recuperação de hook create ambíguo e de DELETE concluído com commit perdido é manual e conservadora. A rotação de URL também pode ficar bloqueada por PATCH concluído antes de commit; nunca se sobrescreve uma URL diferente sem verificação.
- A publicação e redelivery têm limites; ao esgotar, é necessário `recover`. Uma callback concluída antes de crash pode receber novamente o mesmo `event_id` (at-least-once, não exactly-once).
- Testes integrados cobrem HTTP/ChannelReader, callback em namespace separado, fila indisponível, callback com erro, duplicação, assinatura inválida, rotação e cleanup de dois PRs. O roteador é um engine de teste, não o engine de produção. A suíte GitHub foi validada serialmente; validações externas e carga continuam pendentes.
