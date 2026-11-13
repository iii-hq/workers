# quick-tunnel

Worker binário Rust do iii que gerencia túneis públicos por leases independentes e persistidas, com um provider por instância: **processos filhos cloudflared Quick Tunnel** (`provider: cloudflared`, padrão) ou **OpenTunnel dentro do processo** (`provider: opentunnel`). Não implementa servidor HTTP, não substitui os workers HTTP/queue e não instala nem baixa cloudflared.

## Pré-requisito e segurança

Com `provider: cloudflared`, o operador deve fornecer um executável cloudflared confiável, com suporte a `tunnel --output json` (contrato de logs conferido no upstream 2025.11.1 e no `master` consultado em 2026-09-19). Configure `cloudflared` com um caminho absoluto, ou o nome: um nome solto é procurado **somente** em `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin` e `/bin` (inclui o Homebrew em Apple silicon), e o filho é iniciado pelo caminho absoluto encontrado. O `PATH` herdado do worker não é consultado: uma entrada gravável pelo usuário (`~/.local/bin`, shims) poderia substituir o binário que publica um serviço local na Internet. Para qualquer outro local, configure o caminho absoluto. A interface registra mesmo sem o executável: somente a primeira lease inicia um filho; executável ausente produz `failed` com `cloudflared not found (...); install it from <página de downloads da Cloudflare>`. `quick-tunnel::status` informa `prerequisites.cloudflared` antes de qualquer lease, para que o operador instale o binário antes de habilitar webhooks. Com `provider: opentunnel` não há binário: o campo `cloudflared` é ignorado, `prerequisites` é omitido e a resposta de `status` traz `provider: "opentunnel"`.

**Acquire publica o serviço HTTP autorizado na Internet.** Configure o destino HTTP de webhooks com validação HMAC e sem rotas administrativas. O worker não autentica HTTP público, não cria webhooks GitHub e não fornece SLA: Quick Tunnels são efêmeros e destinados a desenvolvimento. Não exponha o engine iii como target.

Callers não fornecem origin, porta, argumentos, ambiente ou caminho executável. `targets` é uma allowlist exclusiva do operador; aceita apenas HTTP(S) com IP literal loopback, sem credenciais, caminho, query ou fragmento. `localhost` é recusado para evitar ambiguidade DNS. Com `provider: opentunnel`, os targets também precisam ser `http://` (o worker entrega bytes em claro ao target e não fala TLS com ele) e cada ID precisa ser um label DNS minúsculo (`^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$`), porque vira o nome do perfil OpenTunnel; `webhooks` e `mesh` passam. Cada target expõe **o serviço inteiro**, não um prefixo de rota. O worker HTTP e o processo que encaminha (cloudflared, ou o próprio worker com opentunnel) precisam compartilhar o mesmo namespace de rede para que o destino loopback seja alcançável; containers com redes isoladas precisam ser configurados pelo operador antes de habilitar a integração.

O filho usa `env_clear`, HOME temporário privado, configuração YAML vazia explícita, stdin fechado, `--no-autoupdate`, `--metrics 127.0.0.1:0`, `--retries 0`, `--grace-period 0s`, saída limitada a linhas de 16 KiB. Não herda `TUNNEL_*`, tokens, proxies ou configuração cloudflared do host. Métricas do próprio cloudflared ficam somente em loopback. Logs crus, segredos e origin não são retornados nos snapshots. IDs de consumers devem ser identificadores opacos, nunca secrets.

## Provider OpenTunnel

```yaml
provider: opentunnel            # omitido = cloudflared
targets:
  webhooks: http://127.0.0.1:3112
  mesh: http://127.0.0.1:49180
proxy_protocol: [mesh]            # targets cujo serviço lê o header PROXY v2
startup_timeout_ms: 120000
```

Roda dentro do processo pelo crate `opentunnel` (`=0.6.2`), sem binário, contra o relay público `opentunnel.xyz` (URL da API não configurável). Cada target tem sua própria identidade OpenTunnel (perfil = ID do target), ativa só enquanto houver leases, como um filho cloudflared. O worker nunca remove a identidade no relay: ela sobrevive a leases e restarts.

- **URL estável e difícil de adivinhar.** `https://<rota>.<id>.opentunnel.xyz`. `<id>` é atribuído pelo relay; `<rota>` são 16 caracteres hexadecimais aleatórios que o worker gera uma vez por target e guarda em `<perfil>/route` (modo `0600`). O hostname puro `https://<id>.opentunnel.xyz` não responde. A URL se mantém entre gerações e restarts enquanto o diretório de estado existir (com cloudflared muda a cada processo). Detecte mudança pela `generation`, nunca pela URL. Uma identidade criada antes desta versão mantém hostname e certificado, mas a URL muda uma vez para a forma com rota.
- **Estado.** `<diretório de state_path>/opentunnel/` (padrão `data/quick-tunnel/opentunnel/`), criado pelo worker com modo `0700`; um diretório existente com qualquer permissão de grupo/outros é erro de boot (falha fechada). O worker nunca usa `$HOME`/`XDG_DATA_HOME`. Perder o diretório significa novo hostname: faça backup junto com os dados do worker. Para trocar só a URL, apague o arquivo `route` do perfil; para trocar também o hostname, apague o diretório do perfil. Vale no próximo início do túnel. Um arquivo `route` ilegível ou inválido falha com `opentunnel state unreadable`; ele nunca é regenerado sozinho.
- **Primeiro provisionamento** espera um certificado ACME e pode passar dos 30 s padrão de `startup_timeout_ms`; use `startup_timeout_ms: 120000`. A nova tentativa retoma o provisionamento pendente, então as tentativas convergem. Os padrões não mudam por provider.
- **Erros sanitizados** em `error`: `opentunnel rate limited`, `opentunnel relay unreachable`, `opentunnel certificate failed`, `opentunnel state unreadable`, `opentunnel provisioning failed`, `opentunnel returned an invalid hostname`, `opentunnel route not attached`, `opentunnel stopped`, além de `opentunnel startup/reconnect timeout` e `opentunnel edge connection interrupted`. O erro cru só vai para o log (`warn`). Todos seguem os mesmos retries, `failed` e cooldown do cloudflared.

Segurança, leia antes de habilitar:

- **O TLS dos visitantes termina dentro do processo quick-tunnel.** O relay vê só bytes cifrados, mas o worker guarda a chave privada do certificado e vê toda requisição em claro. Com cloudflared o TLS termina na Cloudflare e o worker nunca vê o tráfego.
- A chave, o certificado e o token do túnel ficam em `data/quick-tunnel/opentunnel/`. Quem lê esse diretório consegue se passar pelo hostname.
- **O serviço local vê todo visitante como `127.0.0.1`**, sem header de encaminhamento, a menos que o target esteja em `proxy_protocol`. Headers como `X-Forwarded-For` ou `CF-Connecting-IP` são escritos pelo visitante. Não confie em peers loopback e autentique tudo que for exposto.
- **`proxy_protocol: [<target>]` faz toda conexão ao target começar com um header PROXY protocol v2** com o endereço do visitante (o que o relay informa), antes de qualquer byte do visitante. Liste um target só se o serviço dele lê esse header nessa porta: os outros recusam todo visitante (o listener do worker HTTP, target `webhooks`, não lê; o hub do mesh lê). Targets na mesma porta precisam estar todos na lista ou nenhum; conferir que o target listado aponta para a porta do serviço que lê o header é responsabilidade do operador. O `status` informa o modo na lease: `leases[].proxy_protocol: "v2"`. Um consumer só deve interpretar o header enquanto um `status` listar a lease dele com esse valor. A lease guarda o modo em que foi concedida: reiniciar o worker com o modo trocado descarta as leases do target, e o hub do mesh readquire sozinho. Só existe com `provider: opentunnel`; com cloudflared é erro de configuração.
- O provider roda no ambiente do worker. Diferente do filho cloudflared (ambiente limpo), ele respeita `HTTPS_PROXY`, `SSL_CERT_FILE` e `OPENTUNNEL_CA_CERT`.
- **O hostname do relay fica público assim que o certificado é emitido, mas não responde.** O certificado ACME lista `<id>.opentunnel.xyz` e `*.<id>.opentunnel.xyz`, e CAs públicas registram esses nomes nos logs de Certificate Transparency (crt.sh e similares) em minutos; scanners leem esses logs. O nome servido, `<rota>.<id>.opentunnel.xyz`, não aparece em nenhum certificado. Ele é difícil de adivinhar, não secreto: resolvedores DNS, quem observa a rede (o nome vai em claro no TLS), o relay, todos que recebem a URL e todo worker ou agente local autorizado a chamar `quick-tunnel::status` o veem. Continue tratando todo target como alcançável pela Internet enquanto houver lease. Com cloudflared cada processo recebe um `*.trycloudflare.com` aleatório que não é registrado pelo nome.
- github, whatsapp e o hub do mesh aceitam a URL do opentunnel. Um worker whatsapp anterior a esta versão só aceita `https://<label>.trycloudflare.com`: atualize-o.
- O relay é um serviço de terceiros (`opentunnel.xyz`) com limites (30 túneis novos por hora por IP, 5 certificados por dia por túnel) e sem SLA declarado.

## Configuração

O worker usa a configuração central (`configuration`), via `iii-config-client`, com ID `III_CONFIG_NAME` ou `quick-tunnel`. `--config arquivo.yaml` é seed somente se o registro ainda não tiver valor; falhas da configuração central não viram defaults silenciosos. Mudanças exigem **reinício**: reconfiguração ao vivo de provider/destinos/executável não é suportada para evitar trocar a autorização de leases ativas. `--local-config` é alternativa explícita para desenvolvimento/testes isolados.

```yaml
provider: cloudflared        # ou opentunnel
targets:
  webhooks: http://127.0.0.1:3112
proxy_protocol: []           # só com opentunnel
cloudflared: cloudflared     # ignorado com opentunnel
state_path: data/quick-tunnel/leases.json
startup_timeout_ms: 30000
max_retries: 3
failed_cooldown_ms: 300000
retry_budget_reset_ms: 600000
retry_initial_ms: 1000
retry_max_ms: 30000
max_leases: 1024
max_lease_seconds: 2592000
```

Mantenha `state_path` em volume persistente privado e exclusivo por instância. Caminhos relativos são resolvidos no diretório do worker. Um lock de arquivo impede duas instâncias sobre o mesmo estado. Gravação usa arquivo temporário privado, fsync e rename atômico; erro/corrupção falha fechado. Não há estado distribuído: não execute réplicas com arquivos diferentes para a mesma identidade de túnel. Se remover um target com leases persistidas, o próximo boot recusa essas leases; libere-as antes da alteração. Se mudar `proxy_protocol` de um target, o próximo boot descarta as leases dele; o hub do mesh readquire sozinho.

## Contratos

Todos os handlers registram descrição e schemas tipados via serde/schemars, SDK **`=0.26.0`**.

### `quick-tunnel::acquire`

```json
{"consumer_id":"github:owner.repo","tunnel_id":"webhooks","expires_at":"2030-01-01T00:00:00Z"}
```

A identidade deve ser um identificador opaco (caracteres aceitos: alfanuméricos, `-_.:`; máximo 128 bytes). Use uma data futura dentro de `max_lease_seconds`.

```json
{"tunnel_id":"webhooks","lease_id":"uuid","status":"starting","public_url":null,"generation":"uuid","error":null}
```

`tunnel_id` omitido é `webhooks`. Campos desconhecidos, inclusive `origin`, são recusados. Retorno imediato; não bloqueia esperando DNS/conexão. Idempotência por `(consumer_id,tunnel_id)`: retorna a mesma lease e estende a expiração pelo máximo entre a existente e a solicitada (nunca encurta uma renovação concorrente). Consumers diferentes possuem leases independentes. A lease é gravada antes do início do processo. `error` é um campo aditivo do snapshot comum.

Com opentunnel a resposta de `status` traz também `provider: "opentunnel"` e, nas leases de targets em `proxy_protocol`, `proxy_protocol: "v2"`. Com cloudflared nenhum dos dois campos existe. A resposta de `acquire` e o evento `on-changed` não mudam.

### `quick-tunnel::release`

Entrada `{"lease_id":"uuid"}`; saída `{"released":true}`. Repetição/ID desconhecido retorna `false`. A última lease encerra e coleta o filho antes da resposta. Não há autorização por consumer dentro do worker: callers confiáveis dependem da política iii; leases não são credenciais de segurança.

### `quick-tunnel::status`

Entrada `{}` ou `{"tunnel_id":"webhooks"}`. Retorna snapshot acima **sem `lease_id` singular**, mais `leases:[{lease_id,consumer_id,tunnel_id,expires_at,proxy_protocol?}]`, `provider?` e `prerequisites:{cloudflared:{found,path,version,error,install_url}}` (sondado com `cloudflared --version`, limitado a 5 s, fora da task dona das leases; o resultado de um binário inalterado — mesmo caminho e mtime — é reutilizado por 60 s, e um binário ausente é reavaliado a cada leitura sem executar processo). Nunca inclui secrets do cloudflared. O worker nunca baixa nem instala o cloudflared: `install_url` aponta a página oficial da Cloudflare.

### `quick-tunnel::on-changed`

Função observável e privada: só este worker a chama, qualquer worker a observa. Cada mudança de estado de um túnel é uma chamada dela, e o handler devolve o snapshot recebido. Não recebe args: selecione o túnel com um filtro em `input.tunnel_id` (sem filtro, todos os targets). Entrega padrão do iii: cada observer cujos filtros casam recebe a chamada.

Evento `{tunnel_id,generation,status,public_url,error}`. Estados: `starting`, `ready`, `reconnecting`, `failed`, `stopped`. `public_url` só é não-nulo em `ready`; HTTPS estrito, sem credenciais, porta, caminho, query ou fragmento: exatamente um label DNS antes de `.trycloudflare.com` (cloudflared), ou `<rota>.<id>` antes de `.opentunnel.xyz` (opentunnel).

```ts
iii.observe(
  { function_id: 'quick-tunnel::on-changed', filters: { 'input.tunnel_id': { $eq: 'webhooks' } }, capture: ['output'] },
  'github::on-tunnel-change',
  { metadata: { repo: 'owner/repo' } },
)
```

Com `capture: ['output']` o target recebe o envelope `Observed`, com o snapshot em `output`. A metadata do observer e o namespace do target ficam com o engine, que os aplica a cada entrega.

**Integração GitHub: observe `quick-tunnel::on-changed` ANTES de acquire e leia status DEPOIS.** Mudanças podem ocorrer antes da resposta do acquire. Geração é UUID opaco, não ordenável; em dúvida reconcilie status. Nova tentativa/restart sempre ganha nova geração. Com cloudflared ela obtém uma URL nova do serviço (não há garantia de que o serviço nunca reutilize um hostname); com opentunnel a URL é a mesma. Use a geração, não a URL, para detectar mudança. Reconexão interna do mesmo filho/sessão conserva geração. Eventos são best-effort, não uma fila durável: entregas podem falhar, consumidores devem reconciliar status após reconectar; overflow de 256 eventos provoca reenvio dos snapshots atuais. O worker chama a função sem esperar os targets: um target lento não bloqueia leases, expiração nem outros observers. Sem observers, nenhum evento é enviado.

## Ciclo de vida

- Um task proprietário serializa aquisições/liberações, vencimentos, saídas e retries.
- Expirações são checadas pelo runtime a cada 50 ms, sem agente ou chamadas externas. O relógio UTC é a autoridade; atrasos de scheduling/IO são possíveis.
- A última lease termina o processo. Shutdown preserva leases válidas; o próximo boot as reconstrói e inicia nova geração, nunca restaura URL anterior como pronta.
- Startup e reconexão têm deadline monotônico; repetidas mensagens de retry não prolongam o deadline. Backoff exponencial limitado; no máximo `1 + max_retries` processos rápidos por orçamento (depois, um por `failed_cooldown_ms`). O orçamento só é restaurado depois que uma geração fica `ready` continuamente por `retry_budget_reset_ms` (padrão 10 min), então loops de crash logo após conectar continuam consumindo o orçamento. Esgotado o orçamento, o status fica `failed` (observável), mas enquanto houver leases o worker tenta um novo processo a cada `failed_cooldown_ms` (padrão 5 min): uma queda transitória do trycloudflare se recupera sozinha, sem loop rápido. `failed_cooldown_ms: 0` restaura o comportamento terminal (libere/expire todas as leases ou reinicie o worker); `retry_budget_reset_ms: 0` nunca restaura o orçamento.
- SIGTERM e CTRL-C encerram/recolhem os filhos (com opentunnel: fecham a sessão, abortada se passar de 2 s), abortam leitores e desligam o SDK. `kill_on_drop(true)` protege cancelamento/panic; encerramento normal usa kill + wait, inclusive se o filho ignorar SIGTERM. Requisições em andamento no serviço público são interrompidas, sem drain.
- SIGKILL/queda do host não executa destructors; use o supervisor/container com grupo de processos/cgroup para limpeza nesse cenário. Não se promete cleanup sob SIGKILL nem de descendentes criados por executável malicioso. O prerequisite deve ser cloudflared oficial sem auto-update, não um wrapper que daemonize.

## Contrato real de JSON

[try.cloudflare.com](https://try.cloudflare.com/) anuncia hostname/edge/health em JSON no stdout. Porém as fontes oficiais consultadas ainda mostram **logs JSON no stderr**, ativados por **`--output json`**, e URL em linha ASCII dentro de `message`; não encontramos ali flag dedicada `--json` ou campo estável `public_url`. Não inventamos esse contrato.

O parser faz primeiro `serde_json` para registros tipados. Somente uma mensagem inteira contendo URL (opcionalmente envolvida em `| ... |`) pode anunciar hostname. Não procura URLs arbitrárias com regex em texto. `ready` exige também `message == "Registered tunnel connection"`, `level == "info"`, `connIndex == 0`, UUID `connection` e protocolo `quic`/`http2`, emitidos após registro efetivo no edge. URL impressa sozinha **não** basta; não se faz probe HTTP ao hostname público. Isso confirma edge conectado, não saúde do origin ou propagação DNS global. Mudanças upstream de formato podem causar timeout; são detectáveis, não presumidas como sucesso.

Fontes:
- [SDK Rust](https://iii.dev/docs/reference/sdk-rust.md)
- [quick_tunnel.go 2025.11.1](https://raw.githubusercontent.com/cloudflare/cloudflared/2025.11.1/cmd/cloudflared/tunnel/quick_tunnel.go)
- [quick_tunnel.go master consultado](https://raw.githubusercontent.com/cloudflare/cloudflared/master/cmd/cloudflared/tunnel/quick_tunnel.go)
- [flags.go: output=json](https://raw.githubusercontent.com/cloudflare/cloudflared/master/cmd/cloudflared/flags/flags.go)
- [logger/create.go: stderr JSON](https://raw.githubusercontent.com/cloudflare/cloudflared/master/logger/create.go)
- [connection/observer.go: registro efetivo](https://raw.githubusercontent.com/cloudflare/cloudflared/2025.11.1/connection/observer.go)
- [supervisor/tunnel.go: desconexão/retry](https://raw.githubusercontent.com/cloudflare/cloudflared/2025.11.1/supervisor/tunnel.go)
- [cmd.go: flags, config e métricas](https://raw.githubusercontent.com/cloudflare/cloudflared/2025.11.1/cmd/cloudflared/tunnel/cmd.go)

## Validação local, sem túnel real

```bash
CARGO_TARGET_DIR="$PWD/target/isolated" cargo fmt --all -- --check
CARGO_TARGET_DIR="$PWD/target/isolated" cargo clippy --locked --all-targets --all-features -- -D warnings
CARGO_TARGET_DIR="$PWD/target/isolated" cargo test --locked --all-features
```

`tests/provider.rs` cobre o manager com um provider falso, inclusive o modo `proxy_protocol` gravado na lease (uma troca de modo descarta a lease) e o JSON do cloudflared sem campos novos; os testes unitários de `src/provider/opentunnel.rs` cobrem o arquivo `route` e conferem o header PROXY v2 contra o mesmo vetor que o hub do mesh usa. `tests/opentunnel.rs` usa o crate real com a porta da API fechada em loopback e storage temporário, sem relay falso nem rede; leva ~30 s porque o crate repete chamadas à API por até ~16 s. Os testes de processo Unix requerem Python 3 em `/usr/bin/python3` e `/bin/kill`; usam somente fake cloudflared, sem sockets externos. O teste de contratos usa engine WebSocket simulado em loopback. Incluem concorrência/idempotência, leases independentes, expiração autônoma, timeout/limite de retries, crash, reconexão, restart, exclusão de estado, cleanup e sinais. SDK/lockfile usam somente a versão acordada; regenerar lockfile sem pin transitivo pode selecionar duas versões via config-client: mantenha o lockfile entregue.

O worker está registrado no catálogo privado `.deploy/workers.yaml` e nas permissões agregadas. A publicação inicial é para Unix; o ambiente do processo filho e seus testes ainda não suportam Windows. `iii-permissions.yaml` permite somente status sem aprovação, deixando acquire/release no default de aprovação obrigatória. Não há install script que finja prover cloudflared.
