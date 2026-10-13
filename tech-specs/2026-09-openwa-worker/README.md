# Viabilidade: OpenWA como worker iii

This directory preserves the original OpenWA research and Rust prototype notes.
The current [WhatsApp worker](../../whatsapp/README.md) uses Go and whatsmeow.
The [verification](verification.md) records the historical Rust prototype and
its boundaries; current build and validation instructions are in the
[Go development guide](../../whatsapp/docs/development.md).

Pesquisa em 2026-09-30, sem implementação. Upstream inspecionado:
[`rmyndharis/OpenWA`, commit `261fd4e902649328eb2eefd8ee592237cfd94fa2`][commit],
com versão `0.24.0` em [package.json][package].

## Conclusão

É viável empacotar o OpenWA junto de um worker e expor suas operações como
funções iii, usando chamadas HTTP internas e um catálogo gerado de um snapshot
versionado do OpenAPI. Para distribuição pelo Compose sem microVM, a proposta é
um wrapper Rust com `deploy: binary`, supervisionando o OpenWA como processo
Node no host. O provisionamento de Node, OpenWA e suas dependências precisa ser
definido na implementação. Uma imagem OCI também pode agrupar os processos,
mas o Compose inspecionado ainda não instala pacotes de tipo `image` pelo
registry. Fontes: [bootstrap][main], [contrato publicado][openapi],
[execução no Compose][compose-lifecycle] e [instalação de pacotes][compose-registry].
Essa recomendação é uma inferência de arquitetura; não houve teste de execução
da integração.

Para o requisito adicional de eliminar Node completamente, há uma alternativa:
incorporar `whatsapp-rust` ao worker e implementar a compatibilidade desejada
com a API do OpenWA em Rust. Isso substitui o backend OpenWA. A seção
"Alternativa nativa sem Node" descreve a evidência e o limite dessa proposta.

"Toda a API" precisa significar operações REST mais os adaptadores necessários
para arquivos e eventos. O OpenAPI cobre uma superfície grande, mas não descreve
sozinho todos os protocolos, as rotas dinâmicas de plugins ou cada possibilidade
dos engines. As diferenças estão registradas abaixo.

## O que pode ser reaproveitado

O servidor é uma aplicação TypeScript/NestJS 11 sobre Node, com requisito
`>=22.19`; o Dockerfile usa Node 22. O pacote raiz é privado, inicializa via
`node dist/main` e não declara `main`/`exports` para consumo como biblioteca.
O SDK `@rmyndharis/openwa` é um cliente HTTP: importá-lo não inicializa o servidor.
Fontes: [package.json][package], [Dockerfile][dockerfile],
[entrada do SDK][sdk].

O `openapi.json` inspecionado contém **160 paths e 202 operações HTTP**, todas
com `operationId` distinto. A contagem foi feita por leitura do JSON, considerando
`get`, `put`, `post`, `delete`, `options`, `head`, `patch` e `trace`.
Esses números descrevem o snapshot desse commit, não uma garantia de disponibilidade
de cada operação em toda configuração. Há scripts upstream para exportar e
comparar o contrato; o export fixa um ambiente sem sessões e sem conexões Redis.
Fontes: [snapshot][openapi], [exportador][export], [scripts do pacote][package].

Proposta de mapeamento: uma função tipada por operação, com separação explícita
de parâmetros de path/query/body, schemas resolvidos do OpenAPI, nomes iii
estáveis e preservação de erros e metadados HTTP relevantes. Usar o snapshot
como entrada do build torna o catálogo independente do login no WhatsApp.
As adaptações seguintes precisam de implementação explícita; a geração sozinha
não demonstra cobertura total.

| Superfície verificada | Adaptação proposta |
| --- | --- |
| Operações REST JSON | Encaminhamento tipado para a API interna, preservando autenticação, validação e respostas. |
| Dois downloads de mídia: mensagens e status | Representação explícita de arquivo, MIME e bytes ou referência a arquivo; não presumir JSON. |
| `POST /api/plugins/install` | Construir upload `multipart/form-data` a partir de uma entrada de arquivo definida pelo worker. |
| Métricas `text/plain` e interface de plugin `text/html` | Tratar texto e MIME explicitamente; decidir se a interface HTML permanece acessível como recurso HTTP. |
| Ingress de integrações/plugins | Preservar método, headers, query e corpo bruto usados na verificação de assinatura; não reconstruir JSON assinado. |
| Socket.IO no namespace `/events` e webhooks | Ponte assíncrona de eventos para iii, com contrato próprio de assinatura, entrega e reconexão. |

Os tipos de conteúdo da tabela constam do [OpenAPI][openapi]. O ingress lê
`req.rawBody`, aceita contrato definido pelo plugin e usa `@All`; o export
remove métodos que o OpenAPI 3 não representa, como `SEARCH`, embora a rota
continue respondendo a eles. Os eventos usam um gateway Socket.IO, e webhooks
têm serviço próprio de entrega. Fontes: [ingress][ingress],
[normalização do Swagger][swagger], [gateway de eventos][events],
[entrega de webhooks][webhooks].

Dashboard, Swagger UI, Bull Board e MCP também não se reduzem a chamadas REST
JSON geradas. O bootstrap monta parte dessas superfícies diretamente, e o módulo
principal condiciona dashboard, busca, filas e MCP à configuração. Fontes:
[bootstrap][main], [AppModule][app], [documentação MCP][mcp].

## Rodar dentro do worker

| Forma de execução | Viabilidade e custo |
| --- | --- |
| Wrapper Rust como binário no host + processo Node do OpenWA | Proposta para distribuição no Compose sem microVM. O wrapper controla inicialização, readiness, saída e encerramento do filho; é necessário prover Node e as dependências do OpenWA. |
| Wrapper + OpenWA em processos separados na mesma imagem | Viável como empacotamento externo; não é instalável como pacote `image` pelo Compose inspecionado. Funciona com wrapper Node ou Rust. |
| Wrapper Node e OpenWA no mesmo processo | Possível com adaptação do bootstrap e do encerramento. Exige manter uma pequena integração com o código upstream. |
| Importar o pacote npm do SDK para hospedar OpenWA | O SDK é cliente; não fornece o servidor embutido. |

A distinção entre as opções é operacional: uma única unidade
instalável não exige um único processo. Essa comparação é uma inferência baseada
nos pontos de entrada e no empacotamento existentes. Fontes:
[bootstrap][main], [Dockerfile][dockerfile], [SDK][sdk].

Hoje `main.ts` inicia ao ser importado e sua função `bootstrap` não é exportada.
Ela carrega configuração, instala middleware, validação, autenticação de
superfícies auxiliares, adapter WebSocket e sinais, e chama `app.listen(port)`.
O `ShutdownService` termina o processo com `process.exit`. Uma integração no
mesmo processo deve expor criação/fechamento da aplicação, coordenar sinais e
reinícios e preservar essa configuração. Invocar métodos de controller/service
diretamente não percorre a cadeia HTTP configurada; por isso a recomendação
inicial continua sendo reutilizar a API interna. Fontes: [bootstrap][main],
[configuração HTTP][http-config], [ShutdownService][shutdown].

**Detalhe de rede:** o bootstrap atual chama `listen(port)` sem host explícito.
Não se deve descrever a versão inalterada como limitada a `127.0.0.1`. Na imagem,
a porta upstream pode ficar sem publicação no host; isolamento também depende
da rede do container. Para execução nativa com escuta somente local, cabe uma
alteração pequena para definir o host de bind. Fonte: [bootstrap][main].

## Rust e necessidade de sandbox

O wrapper pode usar `iii-sdk` Rust e iniciar o OpenWA como processo filho;
esse padrão já existe no [worker codex](../../codex/src/codex/mod.rs).
Isso mantém o runtime Node requerido pelo OpenWA. Eliminar Node exigiria
substituir ou reimplementar o backend, ampliando o escopo além do wrapper.

No Compose inspecionado, o modo de instalação determina o isolamento:

| Distribuição | Execução |
| --- | --- |
| Worker Node local por `path://`, sem `runtime.base_image` | Processo no host; Node e dependências precisam estar disponíveis. |
| Bundle Node instalado pelo registry | MicroVM pelo executável `iii-worker` e seu runtime libkrun. |
| Worker Rust publicado como `binary` | Processo no host, podendo supervisionar um processo Node filho. |

O caminho de bundles usa diretamente `iii-worker`; não exige instalar o
worker separado `iii-sandbox` ou `sandbox-code-runner`. Ainda exige suporte
ao runtime de microVM. Fontes: [resolução local][compose-manifest],
[execução no Compose][compose-lifecycle] e
[documentação do Compose][compose-docs]. Docker não é necessário para a
proposta de wrapper Rust com processos no host.

## Alternativa nativa sem Node

A biblioteca [`oxidezap/whatsapp-rust`][native-readme] é uma candidata para
integrar o protocolo diretamente ao worker Rust. Sua documentação descreve
pareamento por QR/código, sessões persistentes, mensagens, mídia, grupos,
comunidades, contatos e status. Esses são recursos documentados pelo projeto,
não uma demonstração de equivalência com todas as operações do OpenWA.

A arquitetura proposta seria `iii → worker Rust com iii-sdk e whatsapp-rust →
WhatsApp`, com a biblioteca incorporada ao executável. O [Dockerfile upstream][native-docker]
compila um exemplo para musl e copia somente o binário para uma imagem `scratch`;
isso evidencia um caminho de distribuição nativa sem Node ou Chromium para
o exemplo. A [documentação de instalação][native-install] também descreve
SQLite incorporado. Essa evidência não substitui compilar e testar o futuro
worker em cada plataforma alvo. Arquivos persistentes de sessão continuam
necessários.

O OpenAPI do OpenWA pode orientar o contrato das funções, mas gerar nomes e
schemas não implementa seus handlers. Manter o requisito de toda a API exige
comparar as 202 operações documentadas, preencher diferenças de comportamento
e implementar as partes de gateway, como administração, webhooks e plugins,
quando incluídas no contrato. Ainda não foi feita essa matriz de cobertura.
Portanto, um worker nativo é viável como arquitetura; a paridade integral com
OpenWA permanece não verificada. Não foram compilados binários nem vinculadas
contas para esta avaliação.

## Dependências e limites herdados

O engine é escolhido para o deployment por `ENGINE_TYPE`: `whatsapp-web.js`
é o padrão e usa Chromium; Baileys trabalha sem navegador. A mesma API tem
capacidades diferentes conforme o engine, com respostas `501` para operações
indisponíveis. Portanto, registrar todas as funções não elimina essas limitações,
e remover Chromium escolhendo Baileys não preserva toda a capacidade do outro
engine. A matriz upstream documenta essa diferença. Fontes:
[matriz de capacidades][matrix], [Dockerfile][dockerfile].

O empacotamento precisa preservar dependências nativas, os patches que OpenWA
aplica às bibliotecas WhatsApp e, para os recursos correspondentes, Chromium
e ffmpeg. O Dockerfile upstream já contém essas etapas; é uma base mais fiel
para avaliar uma imagem do worker do que copiar apenas `dist/`.
Fontes: [Dockerfile][dockerfile], [patches na matriz][matrix].

Há um banco principal SQLite e um banco de dados configurável entre SQLite e
PostgreSQL. Credenciais das sessões e outros dados ficam em caminhos persistentes;
as opções incluem `MAIN_DATABASE_NAME`, `DATABASE_NAME`, `SESSION_DATA_PATH`,
`BAILEYS_AUTH_DIR` e `STORAGE_LOCAL_PATH`. Redis e o módulo de filas são opcionais;
habilitar recursos que dependam deles mantém essas dependências. As operações
de infraestrutura que gerenciam containers exigem acesso Docker correspondente,
mesmo que o uso básico do gateway não o exija. Fontes:
[configuração][configuration], [módulos][app], [serviço Docker][docker-service].

O MCP existente oferece **51 ferramentas selecionadas**, desabilitado por padrão,
com modo somente leitura como padrão ao habilitar. Seu `POST /mcp` usa Streamable
HTTP com respostas enquadradas em SSE; `GET` e `DELETE` retornam `405`. É uma
alternativa para um subconjunto de automação, mas não substitui o mapeamento de
toda a API REST. Fonte: [documentação MCP][mcp].

O upstream usa licença MIT, com exigência de inclusão dos avisos de copyright e
permissão nas cópias pertinentes. Fonte: [LICENSE][license].

## Integração com este repositório

O iii já suporta workers Node e publicação por imagem OCI, conforme o
[modelo de workers](../../docs/architecture/worker-model.md). O worker
[opencode](../../opencode/src/index.ts) mostra a conexão com o engine; seu
[registro de funções](../../opencode/src/run.ts) fornece `request_format` e
`response_format`, e seu [emissor](../../opencode/src/events.ts) publica eventos
por `stream::set`.

A [convenção para novos workers](../../docs/sops/new-worker.md) exige que o
catálogo tipado seja capturável sem invocar o backend. Por isso, a proposta é
gerar o catálogo no build a partir do OpenAPI fixado, independentemente da
inicialização do OpenWA ou do login no WhatsApp. Os nomes abaixo são exemplos
propostos, ainda não implementados:

| Função iii proposta | Operação OpenWA |
| --- | --- |
| `openwa::sessions::create` | `POST /api/sessions` |
| `openwa::sessions::list` | `GET /api/sessions` |
| `openwa::sessions::qr` | `GET /api/sessions/{sessionId}/qr` |
| `openwa::groups::list` | `GET /api/sessions/{sessionId}/groups` |
| `openwa::contacts::list` | `GET /api/sessions/{sessionId}/contacts` |

As rotas vêm do [snapshot upstream][openapi]. O wrapper também supervisionaria
o processo OpenWA, encaminharia chamadas e disponibilizaria eventos como
triggers ou streams iii. O comportamento WhatsApp continuaria no upstream.

## Limite da verificação

Foi feita inspeção estática do checkout e das fontes públicas e contagem do
snapshot OpenAPI. Não foram instaladas dependências, compilado o OpenWA,
iniciado o stack de workers nem vinculada uma conta WhatsApp. A viabilidade do
wrapper está sustentada pelos contratos e pontos de entrada; comportamento
integrado, sinais/reinícios, binários, eventos e paridade de cobertura ainda
precisam de uma prova de execução quando houver implementação.

[commit]: https://github.com/rmyndharis/OpenWA/commit/261fd4e902649328eb2eefd8ee592237cfd94fa2
[package]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/package.json
[main]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/main.ts
[dockerfile]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/Dockerfile
[sdk]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/sdk/javascript/src/index.ts
[openapi]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/openapi.json
[export]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/scripts/export-openapi.ts
[ingress]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/modules/integration/ingress.controller.ts
[swagger]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/config/swagger.config.ts
[events]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/modules/events/events.gateway.ts
[webhooks]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/modules/webhook/webhook-delivery.service.ts
[app]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/app.module.ts
[mcp]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/docs/24-mcp-integration.md
[http-config]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/configure-app.ts
[shutdown]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/common/services/shutdown.service.ts
[matrix]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/docs/29-engine-capability-matrix.md
[configuration]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/config/configuration.ts
[docker-service]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/src/modules/docker/docker.service.ts
[license]: https://github.com/rmyndharis/OpenWA/blob/261fd4e902649328eb2eefd8ee592237cfd94fa2/LICENSE
[compose-lifecycle]: ../../../iii/crates/iii-compose/src/lifecycle.rs
[compose-registry]: ../../../iii/crates/iii-compose/src/registry.rs
[compose-manifest]: ../../../iii/crates/iii-compose/src/manifest.rs
[compose-docs]: ../../../iii/docs/using-iii/compose.mdx
[native-readme]: https://github.com/oxidezap/whatsapp-rust/blob/dd0763c4635b353c4ed52def39d030fb2f375f9a/README.md
[native-docker]: https://github.com/oxidezap/whatsapp-rust/blob/dd0763c4635b353c4ed52def39d030fb2f375f9a/Dockerfile
[native-install]: https://whatsapp-rust.jlucaso.com/installation
