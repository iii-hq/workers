# quick-tunnel

Worker binário Rust do iii que gerencia **processos filhos cloudflared Quick Tunnel** por leases independentes e persistidas. Não implementa servidor HTTP, não substitui os workers HTTP/queue e não instala nem baixa cloudflared.

## Pré-requisito e segurança

O operador deve fornecer um executável cloudflared confiável, com suporte a `tunnel --output json` (contrato de logs conferido no upstream 2025.11.1 e no `master` consultado em 2026-09-19). Configure `cloudflared` com um caminho absoluto, ou o nome em `/usr/local/bin:/usr/bin:/bin`. A interface registra mesmo sem o executável: somente a primeira lease inicia um filho; executável ausente produz `failed` após as tentativas limitadas.

**Acquire publica o serviço HTTP autorizado na Internet.** Configure o destino HTTP de webhooks com validação HMAC e sem rotas administrativas. O worker não autentica HTTP público, não cria webhooks GitHub e não fornece SLA: Quick Tunnels são efêmeros e destinados a desenvolvimento. Não exponha o engine iii como target.

Callers não fornecem origin, porta, argumentos, ambiente ou caminho executável. `targets` é uma allowlist exclusiva do operador; aceita apenas HTTP(S) com IP literal loopback, sem credenciais, caminho, query ou fragmento. `localhost` é recusado para evitar ambiguidade DNS. Cada target expõe **o serviço inteiro**, não um prefixo de rota. O worker HTTP e o processo cloudflared precisam compartilhar o mesmo namespace de rede para que o destino loopback seja alcançável; containers com redes isoladas precisam ser configurados pelo operador antes de habilitar a integração.

O filho usa `env_clear`, HOME temporário privado, configuração YAML vazia explícita, stdin fechado, `--no-autoupdate`, `--metrics 127.0.0.1:0`, `--retries 0`, `--grace-period 0s`, saída limitada a linhas de 16 KiB. Não herda `TUNNEL_*`, tokens, proxies ou configuração cloudflared do host. Métricas do próprio cloudflared ficam somente em loopback. Logs crus, segredos e origin não são retornados nos snapshots. IDs de consumers devem ser identificadores opacos, nunca secrets.

## Configuração

O worker usa a configuração central (`configuration`), via `iii-config-client`, com ID `III_CONFIG_NAME` ou `quick-tunnel`. `--config arquivo.yaml` é seed somente se o registro ainda não tiver valor; falhas da configuração central não viram defaults silenciosos. Mudanças exigem **reinício**: reconfiguração ao vivo de destinos/executável não é suportada para evitar trocar a autorização de leases ativas. `--local-config` é alternativa explícita para desenvolvimento/testes isolados.

```yaml
targets:
  webhooks: http://127.0.0.1:3112
cloudflared: cloudflared
state_path: data/quick-tunnel/leases.json
startup_timeout_ms: 30000
max_retries: 3
retry_initial_ms: 1000
retry_max_ms: 30000
max_leases: 1024
max_lease_seconds: 2592000
```

Mantenha `state_path` em volume persistente privado e exclusivo por instância. Caminhos relativos são resolvidos no diretório do worker. Um lock de arquivo impede duas instâncias sobre o mesmo estado. Gravação usa arquivo temporário privado, fsync e rename atômico; erro/corrupção falha fechado. Não há estado distribuído: não execute réplicas com arquivos diferentes para a mesma identidade de túnel. Se remover um target com leases persistidas, o próximo boot recusa essas leases; libere-as antes da alteração.

## Contratos

Todos os handlers registram descrição e schemas tipados via serde/schemars, SDK **`=0.22.1-alpha.25`**.

### `quick-tunnel::acquire`

```json
{"consumer_id":"github:owner.repo","tunnel_id":"webhooks","expires_at":"2030-01-01T00:00:00Z"}
```

A identidade deve ser um identificador opaco (caracteres aceitos: alfanuméricos, `-_.:`; máximo 128 bytes). Use uma data futura dentro de `max_lease_seconds`.

```json
{"tunnel_id":"webhooks","lease_id":"uuid","status":"starting","public_url":null,"generation":"uuid","error":null}
```

`tunnel_id` omitido é `webhooks`. Campos desconhecidos, inclusive `origin`, são recusados. Retorno imediato; não bloqueia esperando DNS/conexão. Idempotência por `(consumer_id,tunnel_id)`: retorna a mesma lease e estende a expiração pelo máximo entre a existente e a solicitada (nunca encurta uma renovação concorrente). Consumers diferentes possuem leases independentes. A lease é gravada antes do início do processo. `error` é um campo aditivo do snapshot comum.

### `quick-tunnel::release`

Entrada `{"lease_id":"uuid"}`; saída `{"released":true}`. Repetição/ID desconhecido retorna `false`. A última lease encerra e coleta o filho antes da resposta. Não há autorização por consumer dentro do worker: callers confiáveis dependem da política iii; leases não são credenciais de segurança.

### `quick-tunnel::status`

Entrada `{}` ou `{"tunnel_id":"webhooks"}`. Retorna snapshot acima **sem `lease_id` singular**, mais `leases:[{lease_id,consumer_id,tunnel_id,expires_at}]`. Nunca inclui secrets do cloudflared.

### `quick-tunnel::changed`

Config `{}` (todos os targets) ou `{"tunnel_id":"webhooks"}`. Evento `{tunnel_id,generation,status,public_url,error}`. Estados: `starting`, `ready`, `reconnecting`, `failed`, `stopped`. `public_url` só é não-nulo em `ready`; HTTPS estrito, exatamente um label DNS antes de `.trycloudflare.com`, sem credenciais, porta, caminho, query ou fragmento. Metadata e namespace resolvido do `TriggerConfig` são preservados no disparo; namespace legado ausente significa `default`, não o namespace do provider.

**Integração GitHub: assine o trigger ANTES de acquire e leia status DEPOIS.** Mudanças podem ocorrer antes da resposta do acquire. Geração é UUID opaco, não ordenável; em dúvida reconcilie status. Nova tentativa de processo/restart sempre ganha nova geração e obtém uma URL nova do serviço (não há garantia de que o serviço nunca reutilize um hostname). Reconexão interna do mesmo filho conserva geração. Eventos são best-effort, não uma fila durável: bindings/entregas podem falhar, consumidores devem reconciliar status após reconectar; overflow de 256 eventos provoca reenvio dos snapshots atuais. Callback lento não bloqueia leases/expiração, mas atrasa outros callbacks (timeout de 5 s por entrega).

## Ciclo de vida

- Um task proprietário serializa aquisições/liberações, vencimentos, saídas e retries.
- Expirações são checadas pelo runtime a cada 50 ms, sem agente ou chamadas externas. O relógio UTC é a autoridade; atrasos de scheduling/IO são possíveis.
- A última lease termina o processo. Shutdown preserva leases válidas; o próximo boot as reconstrói e inicia nova geração, nunca restaura URL anterior como pronta.
- Startup e reconexão têm deadline monotônico; repetidas mensagens de retry não prolongam o deadline. Backoff exponencial limitado; no máximo `1 + max_retries` processos por grupo contínuo de leases. O orçamento não é resetado ao ficar pronto, evitando loops infinitos de crashes. `failed` permanece observável; libere/expire todas as leases antes de nova tentativa, ou reinicie o worker.
- SIGTERM e CTRL-C encerram/recolhem os filhos, abortam leitores e desligam o SDK. `kill_on_drop(true)` protege cancelamento/panic; encerramento normal usa kill + wait, inclusive se o filho ignorar SIGTERM. Requisições em andamento no serviço público são interrompidas, sem drain.
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

Os testes de processo Unix requerem Python 3 em `/usr/bin/python3` e `/bin/kill`; usam somente fake cloudflared, sem sockets externos. O teste de contratos usa engine WebSocket simulado em loopback. Incluem concorrência/idempotência, leases independentes, expiração autônoma, timeout/limite de retries, crash, reconexão, restart, exclusão de estado, cleanup e sinais. SDK/lockfile usam somente a versão acordada; regenerar lockfile sem pin transitivo pode selecionar duas versões via config-client: mantenha o lockfile entregue.

O worker está registrado no catálogo privado `.deploy/workers.yaml` e nas permissões agregadas. A publicação inicial é para Unix; o ambiente do processo filho e seus testes ainda não suportam Windows. `iii-permissions.yaml` permite somente status sem aprovação, deixando acquire/release no default de aprovação obrigatória. Não há install script que finja prover cloudflared.
