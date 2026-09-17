# Avaliação opt-in da busca instalada

O executável `examples/benchmark_jev_search.rs` compara `lexical`, `hybrid`,
`jev` e `jev-shortlist` usando `search::benchmark_installed`, o adapter do
mesmo seletor de produção. O default do executável é **somente lexical**.
Registry, engine, downloads de modelos e aquecimento remoto ficam desativados.
O modo default da aplicação não é alterado por este benchmark.

Esta avaliação não habilita Jev e não demonstra sua superioridade. Nenhuma
credencial foi lida nem chamada à API real foi realizada durante a implementação.
Os resultados locais, quando registrados abaixo, não são resultados Jev.

## Como executar

A partir da raiz deste checkout:

```bash
cargo test --locked --manifest-path iii-directory/Cargo.toml \
  --example benchmark_jev_search

cargo run --locked --manifest-path iii-directory/Cargo.toml \
  --example benchmark_jev_search -- \
  --cases iii-directory/tests/fixtures/jev_search_cases.json \
  --catalog iii-directory/tests/fixtures/discover_catalog.json \
  --output /tmp/jev-search-evaluation.json
```

O programa usa exatamente os paths recebidos; para executar de outro diretório,
passe paths absolutos tanto ao Cargo quanto aos três argumentos. Não sobrescreva
as entradas com `--output`. Os testes do exemplo não consultam serviços nem
leem `TYPESAFE_API_KEY`. O build continua sujeito aos requisitos ONNX/UI existentes
do pacote; o benchmark não instala dependências de build.

Para medir o híbrido, forneça um bundle **já instalado** com MiniLM e reranker:

```bash
cargo run --locked --manifest-path iii-directory/Cargo.toml \
  --example benchmark_jev_search -- \
  --cases iii-directory/tests/fixtures/jev_search_cases.json \
  --catalog iii-directory/tests/fixtures/discover_catalog.json \
  --output /tmp/jev-search-local-evaluation.json \
  --modes lexical,hybrid \
  --model-path /home/anderson/.cache/iii/all-MiniLM-L6-v2-c9745ed1d9f207416be6d2e6f8de32d1f16199bf
```

`bundle_complete` verifica presença/tamanho dos dez artefatos. O relatório
registra seus SHA-256; o loader de produção valida os hashes fixados antes de
usar o modelo. `SemanticSearch::rebuild` apenas inicia o trabalho assíncrono:
o exemplo aguarda uma sondagem local não exata retornar `hybrid_complete`, por
no máximo `--hybrid-ready-timeout-ms` (default 30000). O mesmo índice é reutilizado
até mudar o fingerprint, inclusive nas duas alterações adversariais de descrição.
O aquecimento é registrado separadamente e fica fora dos percentis.

Sem path, bundle completo, suporte MiniLM no target ou prontidão dentro do prazo,
as amostras híbridas são `skipped`. Mesmo após prontidão, cada repetição precisa
retornar `hybrid_complete`; se não retornar, o outcome diagnóstico é preservado,
mas suas métricas de qualidade e latência híbridas não entram no agregado.
O campo confirma a conclusão da política híbrida: queries exatas/intrínsecas
podem dispensar inferência e queries abaixo do piso dense mantêm o ranking lexical
por política. Portanto não afirma que todo resultado passou pelo cross-encoder.

Para uma avaliação remota **futura, deliberada**, com `TYPESAFE_API_KEY` já
disponível no ambiente do processo:

```bash
cargo run --locked --manifest-path iii-directory/Cargo.toml \
  --example benchmark_jev_search -- \
  --cases iii-directory/tests/fixtures/jev_search_cases.json \
  --catalog iii-directory/tests/fixtures/discover_catalog.json \
  --output /tmp/jev-search-remote-evaluation.json \
  --modes jev,jev-shortlist --allow-remote \
  --jev-model jev-1.13.0 --jev-timeout-ms 3000 \
  --jev-min-relevance 0.50 --shortlist-depth 24 --repetitions 3
```

Selecionar um modo remoto sem `--allow-remote` falha antes de ler a chave.
Com a flag, a chave não vazia ainda é obrigatória. `--allow-remote` sozinho não
transforma `lexical`/`hybrid` em execução remota e não faz o programa ler a chave.
A chave não é argumento CLI nem campo do relatório. Cada repetição remota pode
gerar uso cobrado; não há requests extras de aquecimento Jev.

O worker também aceita `function_search_jev_api_key` em sua configuração, com
hot reload e fallback para o ambiente. Este executável independente continua
usando apenas `TYPESAFE_API_KEY` e a flag `--allow-remote`; ele não lê a
configuração nem as credenciais persistidas de uma instalação.

`jev` percorre o catálogo elegível. `jev-shortlist` usa exclusivamente o helper
de produção `lexical_candidate_ids`: união dos primeiros `--shortlist-depth`
resultados BM25 brutos **por query**, considerando todas as capabilities do caso.
Esse catálogo reduzido é passado ao mesmo adapter, com seus lotes/deadline normais.
Não é a união da resposta lexical final de doze candidatos. O pool da variante é
comum aos lotes; é uma ablação de recuperação, sem nova regra de produção.
Uma shortlist vazia pode concluir sem request remoto, e não comprova desempenho
do modelo. Uma falha remota na variante faz fallback no catálogo reduzido.

## Fixture e qrels

`tests/fixtures/jev_search_cases.json` contém 37 casos, 22 de calibração e 15 de
holdout. Os IDs foram conferidos contra as 512 entradas de `discover_catalog.json`.
Os casos derivados de `src/functions/search_relevance.rs` preservam a exigência
de retornar os grupos necessários, incluindo `state::set` + `state::get` e
`github::pr::merge` + `github::pr::checks`.

Há casos simples, compostos, cargas de 6 e 18 capabilities, paráfrases com vocabulário
distante, IDs exatos/camelCase, pedidos físicos sem ferramenta, gibberish, capability
intrínseca, ID excluído e descrição adversarial. O split é fixado no arquivo e
nunca escolhido pelos resultados. Os casos de carga do holdout recombinam
consultas de regressão/calibração: medem composição e lotes; **não são um conjunto
semântico inteiramente inédito**. Repetições também não ampliam o número de
julgamentos independentes.

O formato estende o `required_groups` plano do plano original para qrels por
capability, evitando atribuir a relevância de uma lane às demais:

```json
{
  "id": "store-and-read",
  "split": "calibration",
  "tags": ["match", "regression"],
  "source": "src/functions/search_relevance.rs",
  "capabilities": ["store a value under a key in the state scope and read it back"],
  "qrels": [{
    "acceptable_ids": ["state::set", "state::get"],
    "required_groups": [["state::set"], ["state::get"]],
    "expect_empty": false
  }]
}
```

Campos aceitos em cada caso: `id`, `split` (`calibration|holdout`), `tags`,
`source`, `capabilities`, `qrels` e o opcional `description_overrides`, um mapa
de ID real para descrição substituta. Em cada qrel: somente `acceptable_ids`,
`required_groups` e `expect_empty`. Campos desconhecidos, IDs inexistentes,
capabilities vazias/duplicadas, casos duplicados, grupos vazios/incoerentes e
qrels desalinhados são erros. Há de 1 a 18 capabilities por caso.

`qrels[i]` corresponde a `capabilities[i]`, inclusive para IDs exatos e necessidades
intrínsecas. Uma lane vazia tem `acceptable_ids: []`, `required_groups: []` e
`expect_empty: true`. Um grupo representa alternativas OR; todos os grupos são
necessários (AND). As alternativas precisam pertencer a `acceptable_ids`.
Por exemplo, retornar somente `github::repo::view` satisfaz o grupo
`["github::repo::view", "github::search::repos"]`, mas recupera apenas metade
dos IDs julgados relevantes. Os julgamentos são explícitos e conservadores, não
uma anotação exaustiva de todas as alternativas possíveis do catálogo.

As duas descrições adversariais substituem apenas `state::delete` em uma cópia
do snapshot; seu ID e schema permanecem iguais. A instrução maliciosa está na
primeira frase, para sobreviver à projeção de produção de até 160 bytes. Todos
os modos recebem a mesma alteração; o catálogo original não é modificado.

## Métricas e interpretação do JSON

O adapter conserva os mesmos lotes de até seis capabilities, até três lotes,
doze candidatos por lote, limite de workers e seleção de IDs de produção.
As repetições usam sessões novas; não medem supressão de contratos já entregues.

| Campo | Definição |
| --- | --- |
| `lanes[].recall_at_12` | IDs aceitáveis presentes nos primeiros 12 do ranking da lane / IDs aceitáveis da lane. |
| `lanes[].reciprocal_rank` | Inverso da primeira posição relevante no ranking completo daquela lane; 0 se ausente. Nunca deriva da ordem da resposta agrupada por worker. |
| `covered_groups_at_12` | Grupos da lane com alguma alternativa no seu top 12. |
| `covered_groups_in_response` | Grupos da lane cobertos nos IDs efetivamente selecionados para a resposta inteira. |
| `required_group_coverage` | Soma dos grupos cobertos na resposta / grupos exigidos. |
| `all_required_groups_covered` | Todos os grupos do caso foram entregues. |
| `batches` | Recall e MRR médios das lanes do lote; cobertura considera a resposta inteira, que pode também satisfazer outro lote. |
| `false_positive_candidates_at_12` | Número de candidatos no top 12 de uma lane com `expect_empty`; em lanes com match fica 0, sem presumir qrels exaustivos. |
| `false_positive_response_candidates` | Tamanho da resposta somente quando todas as lanes esperam vazio; `null` nos casos mistos. |
| `false_positive_lane_rate_at_12` | Fração de lanes no-match com algum candidato no top 12. |
| `false_positive_response_rate` | Fração de casos inteiramente no-match com resposta não vazia. |
| `candidate_count` | Quantidade de IDs selecionados, deduplicados, após os lotes. |
| `selected_ids_json_bytes` | Bytes UTF-8 do JSON compacto contendo apenas o array de IDs selecionados; proxy de tamanho, não o wire payload. |
| `settings.response_size_basis` | Registra uma única vez que tamanho e tokens da resposta pública não são medidos; os resultados contêm somente o proxy `selected_ids_json_bytes`. |
| `installed_wall_ms` | Relógio externo incluindo preparação da shortlist, construção das dependências e chamada instalada; sem aquecimento/registry. |
| `outcome.elapsed_ms` | Latência instalada informada pelo adapter; não é a etapa Jev isolada. |
| `jev_stage_ms` | Medição real `outcome.jev_elapsed_ms`, em milissegundos inteiros, acumulada pelas etapas Jev dos lotes da chamada, inclusive etapas falhas. `null` para modos locais, skipped, bypass bem-sucedido sem requests e falha sem requests com elapsed zero. |
| `jev_stage_p50_ms`, `jev_stage_p95_ms` | Percentis das medições `jev_stage_ms` nas amostras do resumo; a visão `all` inclui fallback com medição. `jev_stage_samples` informa o denominador. |
| `reported_jev_requests`, `reported_jev_questions` | Soma de blocos/perguntas completados e validados, inclusive blocos anteriores a erro no mesmo lote; não conta todas as tentativas. |
| `available_input_tokens`, `available_output_tokens` | Soma do uso conhecido em blocos validados, preservado mesmo se um bloco posterior causar fallback do lote inteiro. |
| `usage_status` | `complete` quando a avaliação concluiu; `partial` em fallback com contadores conhecidos; `unavailable` em falha sem uso conhecido; `null` nos modos locais/skipped. |
| `accounted_cost_usd` | `(input_tokens × preço_entrada + output_tokens × preço_saída) / 1e6`, sobre uso conhecido. Falha sem uso conhecido tem custo `null`, não zero. |
| `accounted_cost_is_lower_bound` | `true` para falhas e para agregados que incluem alguma falha; `false` quando todo o uso remoto do grupo está completo; `null` quando não houve execução remota. |

Recall, MRR e cobertura sem denominador ficam `null`. MRR/Recall agregados são
médias por lane com qrels não vazios; cobertura de grupos é ponderada por grupo.
Os resumos separam modo, split, número de capabilities e status, além da visão
`all` que inclui o comportamento entregue com fallback Jev. Percentis usam o
método empirical nearest-rank; não há percentil para um conjunto sem amostras.
Três repetições são default para smoke, insuficientes para estimar uma cauda de
latência de produção com confiança.

`complete`, `fallback` e `skipped` são explícitos. O agregado híbrido exclui
amostras incompletas; o agregado Jev `all` inclui seu fallback e o agregado
`complete` permite estudar apenas conclusões. `remote_evaluation_completion_rate`
inclui falhas mesmo com contador zero, e exclui conclusões sem request
(IDs exatos, intrínsecas ou pool vazio). Isso evita transformar a ausência de
telemetria de falhas em uma taxa artificial de 100%.

Uma falha como chave ausente com `jev_requests == 0`, `jev_elapsed_ms == 0` e
`jev_complete == false` conta como avaliação falha na taxa de conclusão. Não
entra no percentil Jev: não há observação temporal remota útil nesse caso.
Uma falha com elapsed positivo entra, mesmo sem uso retornado. Um request
validado com elapsed zero também entra (resolução de milissegundos). A etapa
inclui preparação/espera por permit do cliente, não apenas HTTP em voo; sua
medição não prova que um request chegou ao serviço. O CLI normalmente rejeita
chave ausente antes da avaliação; a definição também cobre telemetria sintética
e falhas de configuração do adapter.

Os resumos `status: complete` e `status: fallback` permitem comparar amostras
completas e limites inferiores separadamente. Na visão `all`, tokens e custo
somam também o consumo parcial conhecido; `partial_usage_runs` e
`unavailable_usage_runs` indicam a cobertura da contabilidade. Se todos os
custos forem desconhecidos, a soma é `null`; se houver algum conhecido, o
valor é sua soma, com `accounted_cost_is_lower_bound: true` se houver falha.

O preço default de entrada, US$ 0,042/milhão, e saída zero são **premissas do
plano de 17/09/2026**, não uma consulta de preço vigente. Podem ser alterados por
`--input-usd-per-million` e `--output-usd-per-million`. Blocos validados antes de
uma falha preservam seus tokens e custo conhecido: o descarte dos rankings
parciais não descarta a contabilidade. Requests falhos/cancelados ainda podem
consumir tokens cobrados sem uso retornado. Por isso o custo de uma falha é um
limite inferior, nunca uma afirmação de custo total faturado. Se não houver
uso conhecido, é `null`; se houver uso conhecido realmente zero, permanece
zero com o marcador de limite inferior. Não inferir o número de tentativas dos
contadores de blocos validados.

O JSON (`schema_version: 3`) registra hashes SHA-256 dos bytes das duas fixtures, fingerprints dos
catálogos efetivos, hashes dos fontes de ranking/benchmark compilados, target,
versão do pacote, settings e hashes dos artefatos do bundle. O modelo Jev
**solicitado** fica registrado; o efetivo é `null`, pois não está no contrato do
adapter. A latência exclusiva Jev é medida pelo adapter; não é estimada a partir
da latência instalada nem da soma das durações de requests concorrentes.

A versão 3 remove dos resultados `response_bytes`, `response_tokens` e
`response_tokens_basis`, que eram sempre nulos ou constantes. A limitação
continua em `settings.response_size_basis`; as métricas calculadas permanecem.

## Smoke local observado em 17/09/2026

Execução concluída às `2026-09-17T14:53:51Z`, com o comando lexical/hybrid acima,
três repetições, build `dev` sem otimização, pacote `1.2.5-rc.3`, target
`x86_64-unknown-linux-gnu`, MiniLM compilado e Intel Core i9-14900K (32 CPUs
lógicas). São medições de desenvolvimento nesta máquina, com outros trabalhos
no checkout; não uma caracterização controlada da latência de produção.
Este artefato histórico precede a adição de `jev_elapsed_ms` e da preservação de
uso parcial no adapter. Seus números locais permanecem os observados nessa
execução; não preenchem retrospectivamente os novos campos de telemetria remota.

O bundle passou pela validação local. Os três aquecimentos (snapshot original e
as duas descrições adversariais) concluíram em 7,17 s, 4,57 s e 4,57 s, fora da
latência medida. Foram **111 execuções lexicais e 111 híbridas completas**, sem
fallback operacional/skipped. As 111 híbridas retornaram `hybrid_complete`.
Não houve requests, perguntas ou uso remoto; Jev e Jev-shortlist não foram rodados.

| Modo | Split | Capabilities | Amostras | Recall@12 | MRR | Cobertura de grupos | p50 / p95 (ms) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| lexical | calibration | 1 | 60 | 0,974 | 0,904 | 1,000 | 30,81 / 33,39 |
| hybrid | calibration | 1 | 60 | 1,000 | 0,939 | 1,000 | 202,54 / 283,56 |
| lexical | calibration | 3 | 3 | 1,000 | 1,000 | 1,000 | 30,84 / 31,08 |
| hybrid | calibration | 3 | 3 | 1,000 | 0,833 | 1,000 | 565,17 / 580,01 |
| lexical | calibration | 6 | 3 | 1,000 | 1,000 | 1,000 | 31,64 / 31,97 |
| hybrid | calibration | 6 | 3 | 1,000 | 0,917 | 1,000 | 1039,89 / 1097,20 |
| lexical | holdout | 1 | 39 | 0,714 | 0,643 | 0,625 | 30,26 / 31,54 |
| hybrid | holdout | 1 | 39 | 0,857 | 0,857 | 0,750 | 40,60 / 268,85 |
| lexical | holdout | 6 | 3 | 1,000 | 1,000 | 1,000 | 30,68 / 30,69 |
| hybrid | holdout | 6 | 3 | 1,000 | 1,000 | 1,000 | 56,67 / 64,42 |
| lexical | holdout | 18 | 3 | 1,000 | 0,944 | 1,000 | 71,48 / 72,18 |
| hybrid | holdout | 18 | 3 | 1,000 | 0,898 | 0,950 | 3163,92 / 3180,40 |

O holdout de seis capabilities mistura IDs exatos e lanes vazias; sua latência
não representa seis consultas semânticas comuns. Nos casos inteiramente no-match
do holdout, ambos os modos tiveram uma resposta não vazia em seis casos (16,7%,
ou 3/18 repetições): `excluded-search`. O ID excluído em si não apareceu; os
falsos positivos foram outras funções. Portanto a exclusão do ID permanece,
mas a expectativa de resposta vazia desse caso falhou. A calibração no-match
teve 0/3 respostas não vazias em ambos os modos.

`remember-later` não entregou nenhum dos dois grupos em ambos os modos.
`freeze-tab` falhou no lexical e foi recuperado no híbrido. Em `multi-eighteen`,
o híbrido colocou `database::query` no top 12 de sua lane, mas não o entregou
na resposta: cobertura de 19/20 grupos, apesar de Recall@12 1,0. Isso ilustra
por que a cobertura após seleção precisa acompanhar a métrica de ranking.
As expectativas foram preservadas, inclusive nesses resultados negativos.

O artefato completo foi escrito em `/tmp/jev-search-local-evaluation.json`, fora
do repositório. Identidades desta execução:

| Entrada/artefato | SHA-256 |
| --- | --- |
| Cases | `99c423873b2645b43976fcf741b6a758e767440d4e80bf9152b24f749e2b190a` |
| Catalog | `003484bd1c576a5e02f0f33ed32e7e2c2287b97986a696c150d612b0775455fc` |
| JSON completo | `40aafaeccd9a970363d353bb4e4773a8d2e1c030dcc45fb04bcbc79b71a3eba5` |
| `search.rs` compilado | `4207138e0de4b68fc4b85f086370fa91aea206c6f443b392aef0469d20b6bb20` |
| Exemplo compilado | `dc8537d2330a70999078074fc652548081c77916a842c2f0b2ad0848787f465a` |

MiniLM: `sentence-transformers/all-MiniLM-L6-v2`, revisão
`c9745ed1d9f207416be6d2e6f8de32d1f16199bf`; reranker:
`cross-encoder/ms-marco-MiniLM-L6-v2`, revisão
`233902d25c440f23af6f7d6e94d2946bac0bee0a`. Os dez hashes de artefatos constam no JSON.

## Validação e decisão

Os testes do exemplo verificam também a medição real da etapa, uso parcial
preservado, falha sem uso com custo `null` e percentis que incluem falhas
observadas sem confundir bypass com latência zero. A validação usa `cargo clippy --locked
--manifest-path iii-directory/Cargo.toml --example benchmark_jev_search -- -D warnings`
e o check de formatação do exemplo. Os testes verificam opt-in, parsing estrito, alinhamento de qrels,
alternativas OR, Recall@12 versus MRR, falsos positivos, percentis, SHA-256,
skipped híbrido sem métricas inventadas, lotes de 18 lanes e soma dos contadores,
inclusive falha sem telemetria. Todos usam dados locais ou contadores sintéticos.
Falhas HTTP/deadline determinísticas pertencem aos mocks da biblioteca;
os testes do exemplo não substituem os testes de protocolo de `search_jev`.

Os gates propostos no plano ainda dependem de uma avaliação Jev autorizada:
nenhuma regressão de invariantes/IDs exatos, cobertura holdout pelo menos igual
ao melhor baseline, ganho em paráfrases, falsos positivos não superiores ao
baseline, conclusão remota de pelo menos 95%, p95 medido da etapa Jev dentro da
deadline e custo relatado. A métrica `jev_stage_p95_ms` usa as medições reais,
incluindo falhas; depende de execução remota autorizada para produzir resultados.
O adapter ainda não fornece modelo efetivo nem total faturado de falhas; o
benchmark não certifica esses gates sem telemetria adicional.
A latência total com registry ligado é uma
avaliação separada. A shortlist deve apresentar sua perda de recall, mesmo
quando for mais barata ou rápida; não muda silenciosamente o modo Jev.
