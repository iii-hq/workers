# TypeSafe/Jev: pesquisa para busca de funções

Consulta às fontes: **17/09/2026**. Escopo definido pelo solicitante: `function_search_mode: lexical | hybrid | jev`, com **Jev independente do MiniLM**. Este documento reúne contratos publicados, resultados divulgados pelo fornecedor e inferências de arquitetura; não contém implementação nem medições próprias da API.

Foram consultadas páginas públicas oficiais, descobertas pelo [índice llms.txt](https://docs.typesafe.ai/llms.txt), incluindo versões Markdown quando o navegador não abriu a página HTML. Nenhuma chamada autenticada ou paga foi realizada; nenhuma credencial foi lida. Os cookbooks não foram executados.

## Dados decisivos para o plano

| Tema | Dado verificado | Fonte primária |
| --- | --- | --- |
| Endpoint de avaliação | `POST https://api.typesafe.ai/v1/systemone` | [API HTTP](https://docs.typesafe.ai/api) |
| Modelo publicado | `jev-1.13.0`; `jev-latest` e `jev-preview` apontam para ele na consulta | [Models](https://docs.typesafe.ai/models.md) |
| Entrada | `state`, `model`, `questions`; autenticação Bearer e JSON | [Quick start](https://docs.typesafe.ai/introduction/quickstart) |
| Contexto total | **64k tokens** para `state` e todas as `questions` juntos | [Limitações Jev 1.13](https://docs.typesafe.ai/model-jaggedness/jev-1.13.md) |
| Contexto por avaliação | **32k tokens** para `state` mais a maior pergunta | [Limitações Jev 1.13](https://docs.typesafe.ai/model-jaggedness/jev-1.13.md) |
| Quantidade de perguntas | Não encontrei teto numérico separado nas referências consultadas; não interpretar como ilimitada | [API](https://docs.typesafe.ai/api), [limitações](https://docs.typesafe.ai/model-jaggedness/jev-1.13.md) |
| Opções de uma Choice | Até **255** | [Choice](https://docs.typesafe.ai/primitives/choice) |
| Níveis de um Score | De **2 a 10** | [API](https://docs.typesafe.ai/api), [Score](https://docs.typesafe.ai/primitives/score) |
| Preço publicado | **US$ 0,042 por milhão de tokens de entrada**; saída gratuita | [Models](https://docs.typesafe.ai/models.md) |
| Limites de taxa publicados | **250.000 tokens/s**, **1.200 requests/min**; valores podem mudar sem aviso | [Models](https://docs.typesafe.ai/models.md) |

A fonte escreve “64k” e “32k”; não esclarece ali a convenção decimal/binária ou o tokenizer. Não converter esses limites em bytes garantidos. O aviso está identificado como aplicável ao Jev 1.13 e revisado em 16/09/2026. [Limitações Jev 1.13](https://docs.typesafe.ai/model-jaggedness/jev-1.13.md)

## Catálogo local: fatos fornecidos e viabilidade

**Medição local realizada pelo agente principal durante esta pesquisa:** [`discover_catalog.json`](../tests/fixtures/discover_catalog.json) contém 512 funções em 48 namespaces; schemas completos ocupam 799.339 bytes; a projeção de IDs, descrições completas e nomes de parâmetros ocupa 141.419 bytes em JSON. Esses números descrevem a fixture, não o catálogo vivo.

**Inferência:** bytes não demonstram que qualquer representação cabe no orçamento em tokens. A aceitação do catálogo inteiro permanece não verificada. Uma única Choice com 512 funções excede o teto publicado; 512 perguntas Noul/Score são uma estrutura diferente e não herdam automaticamente o limite de 255 opções. [Choice](https://docs.typesafe.ai/primitives/choice)

O contrato permite um estado como string, objeto ou array. Todas as perguntas leem esse mesmo estado e são independentes. Uma resposta da mesma chamada não se torna contexto de outra pergunta. [State](https://docs.typesafe.ai/concepts/state), [Primitives](https://docs.typesafe.ai/primitives)

| Alternativa para o modo `jev` | Viabilidade e tradeoff — inferências, não medições |
| --- | --- |
| Catálogo inteiro no estado; Noul/Score por função | Preserva a cobertura dos 512 candidatos. Cada instrução precisa identificar a função avaliada. O estado inteiro participa do limite por avaliação; muitas descrições irrelevantes podem atrapalhar o julgamento. [State](https://docs.typesafe.ai/concepts/state), [limitações](https://docs.typesafe.ai/model-jaggedness/jev-1.13.md) |
| Consulta no estado; descrição de cada função na respectiva pergunta | Também pode cobrir todo o catálogo, com menos contexto compartilhado. O cookbook de skills demonstra descrições de candidatos dentro de instruções Noul. A distribuição do orçamento entre estado e perguntas muda; a capacidade para 512 candidatos ainda precisa ser confirmada. [Skill suggestion](https://docs.typesafe.ai/cookbooks/skill_suggestion.md) |
| Catálogo inteiro dividido em grupos | Conserva cobertura se todos os grupos forem avaliados, mas aumenta chamadas e repete contexto comum. Manter a mesma pergunta/rubrica torna a comparação conceitualmente coerente; a qualidade da ordenação entre grupos não foi medida aqui. [Guia oficial para agentes](https://raw.githubusercontent.com/typesafe-ai/skills/main/skills/typesafe-ai/SKILL.md) |
| Shortlist BM25 e julgamento Jev | Reduz candidatos e texto enviados; candidatos descartados na recuperação não podem ser recuperados pelo reranker. Não exige MiniLM. [Re-ranking](https://docs.typesafe.ai/cookbooks/rerank_typesafe) |
| Seleção por namespace, depois funções | As 48 categorias cabem no teto de Choice, se a descrição total também couber no contexto. Pode excluir funções adequadas quando a primeira seleção erra; manter mais de um caminho é alternativa conceitual, não decisão deste documento. [Choice](https://docs.typesafe.ai/primitives/choice) |

O formato HTTP não exige embeddings ou recuperação prévia. **Conclusão de pesquisa:** um modo Jev autônomo é compatível com a interface publicada. A escolha entre catálogo completo, grupos e shortlist depende do orçamento real e da qualidade observada; as fontes não justificam assumir que o payload local inteiro cabe numa chamada. [Introdução](https://docs.typesafe.ai/introduction)

## Contrato wire e exemplos originais

Os exemplos abaixo são **payloads ilustrativos**, com funções fictícias. Não foram enviados. O comando demonstra o HTTP documentado; o cliente calcula o enquadramento e o tamanho do corpo. [Quick start](https://docs.typesafe.ai/introduction/quickstart)

```bash
curl --request POST 'https://api.typesafe.ai/v1/systemone' \
  --header 'Authorization: Bearer <API_KEY>' \
  --header 'Content-Type: application/json' \
  --data-binary @- <<'JSON'
{
  "model": "jev-1.13.0",
  "state": {
    "query": "Consultar o andamento de uma entrega",
    "functions": {
      "shipping.track": "Consulta o andamento de uma entrega pelo identificador.",
      "billing.refund": "Solicita o estorno de um pagamento."
    }
  },
  "questions": {
    "candidate_0": {
      "type": "noul",
      "instructions": "A funcao de chave shipping.track, no objeto functions, atende diretamente ao pedido em query?",
      "criteria": {
        "true": "Executa a capacidade solicitada.",
        "false": "Nao executa a capacidade solicitada."
      }
    },
    "candidate_1": {
      "type": "noul",
      "instructions": "A funcao de chave billing.refund, no objeto functions, atende diretamente ao pedido em query?"
    }
  }
}
JSON
```

Noul exige `type: "noul"` e `instructions` na referência HTTP; `criteria` é opcional e usa as chaves JSON textuais `"true"` e `"false"`. Sua resposta é `{"type":"noul","noul":0.9}`: aqui **0,9 é fictício**, apenas para ilustrar o formato. Não existe campo separado `confidence`. [Noul](https://docs.typesafe.ai/primitives/noul)

**Atenção à referência da função:** os IDs em `questions` são apenas correlação da aplicação; o modelo não os recebe. A função alvo precisa aparecer nas instruções. As referências ao objeto `functions` acima são linguagem descritiva, não uma sintaxe de referência resolvida pela API. [Primitives](https://docs.typesafe.ai/primitives), [guia oficial para agentes](https://raw.githubusercontent.com/typesafe-ai/skills/main/skills/typesafe-ai/SKILL.md)

Uma pergunta Score, inserida no mesmo mapa `questions`, tem este formato:

```json
{
  "type": "score",
  "instructions": "Quanto a funcao shipping.track atende ao pedido em query?",
  "criteria": [
    "Nao atende ao pedido.",
    "Contribui parcialmente para atender ao pedido.",
    "Atende diretamente ao pedido."
  ]
}
```

Score recebe uma lista ordenada, com índices iniciados em zero. Retorna `score`, `legend`, `probabilities` e `confidence`. `score` é a média ponderada dos índices: com três níveis, varia de 0 a 2, podendo ser fracionário; **não é automaticamente uma probabilidade entre 0 e 1**. As chaves de `legend` e `probabilities` são strings como `"0"`, `"1"`, `"2"`. Os níveis precisam de descrições autossuficientes. [Score](https://docs.typesafe.ai/primitives/score)

Uma Choice para a seleção entre candidatos conhecidos:

```json
{
  "type": "choice",
  "instructions": "Qual opcao atende melhor ao pedido em query?",
  "criteria": {
    "shipping.track": "Consultar o andamento de uma entrega.",
    "billing.refund": "Solicitar o estorno de um pagamento.",
    "none": "Nenhuma funcao atende ao pedido."
  }
}
```

Choice retorna `choice`, `probabilities` e `confidence`; seleciona a opção de maior probabilidade. As probabilidades somam 1 dentro daquela Choice. A opção `none` ocupa uma das 255 posições. **Inferência:** probabilidades de Choices feitas em grupos diferentes não representam automaticamente um ranking global comparável. [Choice](https://docs.typesafe.ai/primitives/choice)

O envelope de resposta tem `model`, `answers` e `usage`; respostas ficam sob os mesmos IDs das perguntas. `usage` contém `input_tokens` e `output_tokens`. [Quick start](https://docs.typesafe.ai/introduction/quickstart)

Não encontrei endpoint dedicado `/rerank` nas referências consultadas: os cookbooks usam a avaliação genérica. [llms.txt](https://docs.typesafe.ai/llms.txt)

## Paralelismo, batching e preço

A TypeSafe declara que perguntas sobre o mesmo estado são processadas paralelamente e que acrescentá-las normalmente altera pouco a latência. Isso é uma propriedade anunciada, **não um SLA nem uma medição com 512 funções**. Batching aqui significa várias perguntas numa chamada; não foi identificado um contrato de job assíncrono de batch nas páginas consultadas. [Speculative fan-out](https://docs.typesafe.ai/patterns/fan-out), [llms.txt](https://docs.typesafe.ai/llms.txt)

No cookbook de batching, o fornecedor compara 13 perguntas sobre um documento em chamadas separadas e juntas: relata redução de custo de **12,2 vezes** e de tempo de **10 vezes**, com cinco repetições por estratégia. O documento domina o custo de entrada naquele experimento. Não são resultados desta pesquisa nem promessa para o catálogo local. [Parallel questions](https://docs.typesafe.ai/cookbooks/parallel_questions)

Perguntas extras consomem tokens; paralelismo não significa entrada gratuita. Não encontrei tarifa monetária separada por pergunta. [Guia oficial para agentes](https://raw.githubusercontent.com/typesafe-ai/skills/main/skills/typesafe-ai/SKILL.md), [Models](https://docs.typesafe.ai/models.md)

**Cálculo derivado do preço publicado:** custo em USD = `input_tokens × 0.042 / 1_000_000`. Uma entrada hipotética contabilizada como 10.000 tokens custaria US$ 0,00042; 50.000 custariam US$ 0,0021. São contas ilustrativas, não estimativas de tokens do fixture, nem confirmação de que uma distribuição específica entre estado e perguntas seria aceita. [Anúncio oficial](https://typesafe.ai/blog/introducing-system-one-models-and-jev)

Aliases podem mudar sem alteração no cliente. A página Models diz que `response.model` identifica a versão efetiva, embora exemplos em outras páginas ainda mostrem `jev-latest`. `GET https://api.typesafe.ai/v1/models`, com Bearer, lista nomes disponíveis à conta; a referência informa que IDs versionados podem ser aceitos mesmo ausentes dessa lista. Nenhuma consulta autenticada foi feita. [Models](https://docs.typesafe.ai/models.md)

## Evidência publicada sobre ranking

O cookbook de reranking usa **`jev-1.12`**, 3.565 passagens jurídicas, 40 consultas e shortlists BM25 de 30 candidatos. Faz **1.200 chamadas**, uma por par consulta/candidato, com Noul e concorrência no cliente. Relata top-1 de 5% para 18% e top-10 de 38% para 62%; custo agregado de US$ 0,0645. É uma medição publicada pelo fornecedor, com modelo anterior e outro domínio. Não comprova batching de 512 pares, desempenho em funções ou ganho sobre MiniLM. [Re-ranking](https://docs.typesafe.ai/cookbooks/rerank_typesafe)

O cookbook de skills também usa `jev-1.12`. Primeiro aplica **uma Choice com 182 opções**, acompanhada de perguntas Noul; depois examina três candidatas com informações mais completas e Noul por candidata. Portanto, “182 skills numa chamada” **não significa 182 perguntas por par**. O exemplo inclui uma seleção incorreta persistente após a segunda etapa, mostrando que candidatos próximos podem passar pelos critérios. [Skill suggestion](https://docs.typesafe.ai/cookbooks/skill_suggestion.md)

O anúncio de lançamento menciona respostas de 70–500 ms e reconhece que seus testes foram geralmente feitos na costa oeste dos EUA, próxima do serviço. Esses números são declarações/resultados do fornecedor, sem garantia de latência para a rede, payload e concorrência desta aplicação. [Anúncio oficial](https://typesafe.ai/blog/introducing-system-one-models-and-jev)

## Confidence, qualidade e consequências para Rust

`confidence` resume a concentração da distribuição de Choice/Score. **Não é igual à probabilidade da opção vencedora**, nem demonstra correção factual. A página não publica a fórmula exata; não assumir entropia, margem ou máximo. Limiares devem ser avaliados no domínio de uso. [Confidence](https://docs.typesafe.ai/confidence)

Para Noul, valores próximos de 0,5 significam equilíbrio entre sim e não, não “relevância média”. Score representa intensidade segundo a rubrica; Noul responde a uma condição. **Inferência:** ordenar por Noul exige uma definição consistente de “atende ao pedido”; ordenar por Score exige a mesma escala entre candidatos. Não ordenar relevância por `confidence`, pois uma avaliação negativa pode ser muito confiante. [Noul](https://docs.typesafe.ai/primitives/noul), [Score](https://docs.typesafe.ai/primitives/score), [Confidence](https://docs.typesafe.ai/confidence)

A página de limitações admite perda de precisão com estado irrelevante, dificuldade com indireções e suscetibilidade a conteúdo adversarial. A promessa de independência entre perguntas não elimina distração dentro do estado compartilhado. [Limitações Jev 1.13](https://docs.typesafe.ai/model-jaggedness/jev-1.13.md), [Introdução](https://docs.typesafe.ai/introduction)

**Implicações de integração, sem desenho de implementação:** em Rust, o contrato necessário é HTTP/JSON com respostas discriminadas por `type`, IDs de correlação e números fracionários. Uma ordenação precisa distinguir ausência/erro de resposta de baixa relevância. A API documenta `401`, `422`, `429` e `529`; recomenda backoff para os dois últimos. Isso introduz dependência de rede e latência variável no modo Jev. [API](https://docs.typesafe.ai/api)

Há divergências documentais: a referência HTTP simplifica descrições de Choice para string/null, enquanto os tipos Python admitem objetos/arrays; estes também mostram `instructions` opcional, ao contrário da referência HTTP. O exemplo deste documento usa o subconjunto comum conservador: instruções explícitas e descrições textuais. Não inferir obrigatoriedade/aceitação do servidor apenas pelos tipos do SDK. [API](https://docs.typesafe.ai/api), [tipos de perguntas Python](https://docs.typesafe.ai/sdk/python/api/types/questions)

## Incertezas restantes

- **Capacidade real:** não houve tokenização compatível com Jev ou submissão dos payloads locais. Quantidade máxima independente de perguntas, limites em bytes e contabilização exata do overhead não ficaram estabelecidos nas fontes consultadas. [Tipos de perguntas](https://docs.typesafe.ai/sdk/python/api/types/questions), [State](https://docs.typesafe.ai/concepts/state)
- **Qualidade do modo Jev:** não há medição própria de recall, qualidade do top-k, abstenção, estabilidade entre grupos, português ou latência de cauda. Os cookbooks citados são evidência do fornecedor sobre outros conjuntos e versões. [Re-ranking](https://docs.typesafe.ai/cookbooks/rerank_typesafe), [Skill suggestion](https://docs.typesafe.ai/cookbooks/skill_suggestion.md)
- **Garantias operacionais:** preço e taxas acima são os publicados na consulta. Não foi estabelecido SLA de disponibilidade/latência, gratuidade de tentativas repetidas ou cache de estado entre requests. [Models](https://docs.typesafe.ai/models.md), [Speculative fan-out](https://docs.typesafe.ai/patterns/fan-out)

**Decisão preservada:** `jev` permanece um modo independente. Catálogo completo e shortlist são alternativas internas a avaliar; esta pesquisa não determina uma dependência de `hybrid` ou MiniLM.
