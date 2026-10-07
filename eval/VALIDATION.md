# Validação de sugestões: reproduzir no ponto de decisão

**Status:** proposta.  
**Data:** 2026-10-05.  
**Escopo:** o que acontece depois que uma análise termina com sugestões. Substitui o E2E como caminho principal de validação ([SPECIFICATION.md §7](SPECIFICATION.md#7-integração-com-o-e2e)); o E2E passa a ser a etapa opcional de não regressão (§7 deste documento).

## 1. Problema

Hoje, validar uma sugestão significa rodar ou vincular execuções do E2E. Isso tem três custos para quem usa o monitor:

- **O plano pede código.** Quando nenhum cenário existente serve, a LLM propõe "construir um caso novo no harness-e2e", o que exige escrever um cenário em Rust e publicá-lo. Isso aconteceu até com uma sessão que já era uma execução do E2E (`state_machine_canvas_build`).
- **Muitas decisões antes do primeiro resultado.** Cada sugestão apresenta plano E2E, anexar execuções, iniciar validação, critério, veredito, ciclo de vida e recorrência.
- **Nenhum passo responde se o problema é real.** A primeira evidência só chega depois de alguém implementar a mudança e rodar duas execuções completas em Docker.

## 2. Evidência

Os dois experimentos abaixo foram feitos à mão em 04/10, com o método descrito na §6.

| Caso | O que foi testado | Resultado |
| --- | --- | --- |
| `kanban_c1_foundation`, deepseek-flash, passo 39 | O aviso `registry-changed` muda a próxima ação? 20 amostras com aviso, 20 sem. | Mesma distribuição de ações (13/20 escrevem o arquivo de teste nos dois casos); 16/20 mencionam o aviso no raciocínio. **Sugestão fraca.** US$ 0,40. |
| `eval_8383b53c` (chat no ADE), Opus 5.5 xhigh, passo 4 | A busca `coder::search` cai na armadilha dos globs relativos a `path`? Contrato original × contrato esclarecido. | A chamada exata do original reaparece em **3/20**; com o texto novo, **0/20** (p = 0,12). Efeito colateral: amostras com `coder::tree` passam de **7/20 para 16/20** (p = 0,005). US$ 0,87. |

A reconstrução foi fiel. No ADE, a contagem de tokens do provider bateu com o uso gravado em todos os 6 passos, com uma diferença constante de 3.187 tokens: um custo fixo que o caminho de chat soma e a contagem não. No kanban, a estimativa local ficou a 0,9% do valor gravado.

Os dois casos mostram o que uma execução E2E isolada não mostra: se o problema se repete, com que frequência, e o que mais muda quando o texto muda.

## 3. Princípios de experiência

1. **Três perguntas, uma de cada vez.** O problema é real? A mudança ajuda? Ela quebra algo? O cartão mostra só a próxima ação.
2. **Nenhum formulário por padrão.** Ponto de decisão, sinal, mudança, modelo e número de amostras vêm da sugestão e da sessão. A pessoa pode editar a mudança, mas não precisa preencher nada.
3. **Custo antes de gastar.** Cada botão que chama um modelo mostra a estimativa. O resultado mostra o custo real.
4. **Resultado em uma frase, com exemplos.** Os números aparecem numa tabela pequena; as amostras podem ser abertas e comparadas com o passo original.
5. **Inconclusivo vira uma ação, não um formulário estatístico:** "Run 30 more".
6. **O mesmo fluxo para pessoa e automação.** Cada passo é uma função com entradas derivadas. Na etapa 1 a pessoa aprova; numa etapa futura, uma regra registrada antes da execução faz isso.

## 4. Fluxo do usuário

A seção **Validation** do cartão de sugestão substitui o bloco "E2E plan" como caminho principal. Os rótulos ficam em inglês, como o resto da interface.

```
análise concluída
   │  Reproduce · 20 samples · up to US$ 1.00
   ▼
reproduzindo (12/20, ao vivo)
   │
   ├─ não reproduziu ──► "Not reproduced in 20 samples."     [Dismiss] [Run 30 more]
   │
   └─ reproduziu ──────► "Reproduced in 3 of 20 (15%)."      [Test change]
                            │  diálogo: a mudança proposta como diff do texto que o modelo viu
                            ▼
                         testando (base × mudança)
                            ▼
                         resultado                           [Approve] [Dismiss] [Run 30 more]
                            ▼
                         aprovada ──► implementação (Copy brief / Draft in chat)
                                      Non-regression in E2E (opcional, recolhido)
```

### 4.1 Estados

| Estado | O que o cartão mostra | Ação principal |
| --- | --- | --- |
| Pronto | Linha "Decision point: step 4 · `coder::search`" com link para a entrada; o sinal em uma frase. | **Reproduce** com estimativa de custo. |
| Reproduzindo | Progresso `k/N` ao vivo; a pessoa pode sair da página. | Nenhuma; *Cancel* discreto. |
| Reproduzido | "Reproduced in 3 of 20 (15%). The original call appeared 3 times." Exemplos expansíveis, cada um lado a lado com o passo original. Selo de fidelidade ("Reconstruction matches the original" ou "Approximate: 4% off"). | **Test change**. |
| Não reproduzido | "Not reproduced in 20 samples. This suggestion is probably not worth it." | **Dismiss**; secundária: *Run 30 more*. |
| Testando | Progresso das duas variantes. | Nenhuma. |
| Comparado | Frase do resultado do alvo, tabela base × mudança e linha de efeitos colaterais (§6.6). | **Approve** ou **Dismiss**; *Run 30 more* quando inconclusivo. |
| Aprovada | Status, autor, nota opcional e a evidência que sustentou a decisão. | **Copy brief** / **Draft in chat**, com a evidência no brief. |
| Não reproduzível | `StatusPanel` com o motivo: sessão anterior a 28/09, ponto de decisão ausente, exposição não suportada ou contexto podado. | *Validate in E2E*, o caminho atual. |
| Falha | `StatusPanel variant="alert"` com o erro do provider e *Retry*. Amostras já obtidas são mantidas. | **Retry**. |

**Approve** e **Dismiss** gravam o ciclo de vida que já existe (`accepted` e `rejected` em `eval::review set_lifecycle`), com o motivo pré-preenchido a partir do resultado ("Not reproduced in 20 samples"). O ciclo detalhado (`in_progress`, `shipped` com PR e versão, `duplicate`) continua disponível no menu do cartão.

### 4.2 Diálogo "Test change"

O diálogo mostra a mudança proposta pela sugestão como um diff do texto que o modelo viu: o trecho do contrato, do aviso, do resultado de ferramenta, da skill ou do system prompt. A pessoa pode editar o texto novo antes de rodar. É a única entrada livre do fluxo. Se a sugestão não traz mudança aplicável ao contexto, por exemplo uma mudança só de código interno, o diálogo explica isso e oferece apenas o E2E.

### 4.3 Larguras e acessibilidade

- **Celular e painel estreito:** a tabela base × mudança vira linhas empilhadas (rótulo e dois valores), e as ações ocupam a largura toda.
- **Painel largo:** tabela com colunas e exemplos lado a lado.
- **Progresso:** anunciado em `aria-live`. O resultado é texto e não depende de cor.

## 5. O que muda na sugestão

A LLM investigadora passa a devolver, em cada sugestão, um objeto `check` ao lado do plano E2E:

```json
"check": {
  "decision_point": "e_t_4c79…_4_assistant",
  "signal": { "question": "Does this step call coder::search with include_globs written relative to `path` instead of the session root?" },
  "change": [
    { "entry_id": "e_t_4c79…_toolu_01Pw…", "find": "globs match relative to its root", "replace": "globs match paths relative to the session root, never to `path`" }
  ]
}
```

- **`decision_point`:** a entrada do assistente em que o comportamento aconteceu. Precisa estar entre as referências da sugestão.
- **`signal`:** como reconhecer o comportamento numa única resposta. Pode ser `{ "rule": "<id>" }`, quando existe regra em código (§6.5), ou `{ "question": "…" }`, uma pergunta de sim/não que o Jev responde por amostra.
- **`change`:** edições de texto que representam a mudança proposta. Formas aceitas:
  - `{entry_id, find, replace}`;
  - `{entry_id, remove: true}`;
  - `{system_prompt: {find, replace}}`.

  O backend recusa um `find` que não existe no texto e registra o motivo, como já faz com referências inexistentes. `change: []` é válido quando a mudança não pode ser expressa como texto; nesse caso o fluxo para em "Reproduzido" e oferece o E2E.

O plano E2E (`validation`) deixa de ser obrigatório e passa a ser pedido só quando a mudança age em todos os passos (system prompt inteiro, skills, orquestração). Quando a sessão observada veio do E2E, `validation.scenario_id` é preenchido em código com `metadata.e2e_scenario`, nunca pela LLM.

Análises anteriores a esta mudança não têm `check`. O cartão delas mostra "Reanalyze to enable reproduction".

## 6. Como a reprodução funciona

### 6.1 Função pública

`eval::reproduce {evaluation_id, suggestion_index, change?: "none" | "proposed" | [edits], samples?: 20, extend?: reproduction_id}` devolve `{reproduction_id}` e roda em segundo plano na fila `eval-run`.

- `change: "none"` reproduz a base. `"proposed"` usa o `check.change` da sugestão. Uma lista de edições usa o texto da pessoa.
- `extend` acrescenta amostras a uma reprodução anterior, com a mesma configuração.
- O resultado fica na linha de revisão da sugestão (`eval_suggestion`, campo `reproductions[]`) e aparece em `eval::result.reviews`. Por isso sobrevive a `eval::delete` e à retenção, como o resto da revisão.
- A interface acompanha ao vivo pelo evento que a página já escuta.

### 6.2 Captura no momento da análise

O registro `harness_turn` guarda só o **último** turno de cada sessão. Se a conversa continua, o system prompt e a política do turno analisado se perdem. A coleta passa a copiar para os assets da análise:
- `options.system_prompt`;
- `options.skill_context.baseline`;
- `options.functions` (com `expose`);
- modelo, provider, `thinking_level`, `provider_options`, `max_output_tokens`;
- o `context_snapshot` do turno.

O eval roda logo depois que o turno termina, então essa cópia é barata e exata.

### 6.3 Reconstrução do pedido

O procedimento abaixo é o usado nos experimentos. Desde o append-only window (#1257, 28/09), a janela do modelo é derivada só do log gravado (`harness/src/window.rs`).

1. Ler as entradas da sessão (`session::messages`, `include_custom: true`) até a entrada anterior ao `decision_point`.
2. Converter as entradas especiais:
   - cada `model_notice` vira a mensagem gravada em `data.message`;
   - `runtime_context` fornece o texto `aid`;
   - `message_order` reordena as mensagens como o Harness faz.
3. Montar o system prompt como o Harness monta:
   - concatenar `system_prompt`, `baseline` e `aid`, separados por linha em branco;
   - enviar em seções: a parte estável (`cache_boundary: true`) e o `aid`;
   - incluir o `cache_intent` com o digest gravado no snapshot.
4. Montar as ferramentas a partir da exposição gravada:
   - `agent_trigger` usa o schema fixo dessa ferramenta;
   - `submit_result`, quando o turno tinha contrato de saída;
   - a exposição `native` fica fora da v1 e cai em "não reproduzível".
5. Aplicar as edições de `change`.
6. Chamar `context::assemble` com os mesmos parâmetros do Harness, incluindo `allow_prune: false` para modelos que vinculam o raciocínio ao prefixo. Se a montagem podar ou compactar, a reprodução é marcada como aproximada.
7. **Conferir a fidelidade.** Chamar `router::count_tokens` no ponto de decisão e no primeiro passo do turno, e comparar com o uso gravado (`input + cache_read + cache_write`).
   - Mesma diferença nos dois pontos: reconstrução **idêntica**.
   - Caso contrário: **aproximada**, com a diferença percentual exibida.
   - Sem contagem do provider: aproximada pela estimativa local.

### 6.4 Amostragem

- Uma chamada de aquecimento (para o cache do provider) e depois até 4 em paralelo, via `router::complete`.
- Nenhuma ferramenta é executada. As amostras nunca são gravadas na sessão observada.
- Por reprodução: de 1 a 50 amostras; por variante, no máximo 100 somadas as extensões.
- Timeout de 120 s por chamada.

### 6.5 Classificação

Cada amostra recebe:

- **Sinal:**
  - Regras em código derivadas dos detectores que já existem:
    - `contract_rediscovery`: a resposta pede contratos que já estão no contexto;
    - `repeated_error_call`: a resposta repete a última chamada que falhou, com payload equivalente.
  - Sem regra: a pergunta do `signal` respondida pelo Jev (`judge::evaluate`, Choice `yes`/`no`/`unclear`), recebendo só as chamadas e o texto da amostra.
  - Regras novas entram no catálogo quando o mesmo tipo de pergunta se repete.
- **Ação:** as funções chamadas (via `agent_trigger`) ou "resposta final sem chamada". Isso alimenta a linha de efeitos colaterais.

### 6.6 Estatística e frase do resultado

Calculadas em código:

- **Reprodução:** "Reproduced in k of N (p%)". Com `k = 0`: "Not reproduced in N samples".
- **Comparação:** taxa do sinal na base e na mudança, com teste exato de Fisher.
  - Com `p ≥ 0,05`, a frase diz que pode ser acaso e indica quantas amostras a mais resolveriam se a diferença observada se mantiver, até o limite de 100 por variante.
  - Exemplo: "Signal dropped from 15% to 0%. This may be chance (p = 0.12); 30 more samples per side would settle it."
- **Efeitos colaterais:** cada tipo de ação cuja frequência muda com `p < 0,05` aparece numa linha. Exemplo: "Side effect: `coder::tree` in 16 of 20 samples (base: 7 of 20)."

### 6.7 Custo

- **Estimativa:** custo gravado no passo do ponto de decisão × N.
- **Custo real:** a soma do `usage` das amostras, mais as chamadas do Jev quando o sinal é uma pergunta.
- Tudo entra no consumo do monitor, separado das métricas da tarefa observada, no balde `replay` (`today_replay_usd`); o balde `capture` é o das análises. A reprodução é manual, sempre disparada por uma pessoa: o limite diário (`daily_cost_cap_usd`) compara só o balde `capture`, então uma reprodução nunca trava a observação automática. Amostra sem custo reportado fica desconhecida (`cost_unknown_samples`), nunca zero; as chamadas do Jev ficam em tokens.

## 7. Onde o E2E entra

O E2E continua, como passo opcional e recolhido ("Non-regression in E2E"), para dois casos:

- mudanças que agem em todos os passos (system prompt inteiro, skills, orquestração), em que um único ponto de decisão não prova o efeito geral;
- sessões que vieram do E2E, rodando o mesmo cenário (`metadata.e2e_scenario`) sem caso novo.

`eval::start-validation`, `eval::attach-validation`, critério e veredito ficam como estão, dentro dessa seção.

## 8. Mudanças no Harness

Nenhuma é necessária para a v1. Estas três lacunas limitam a fidelidade e podem entrar depois:

| Lacuna | Efeito hoje | Na v1 |
| --- | --- | --- |
| O system prompt final, depois dos hooks `pre-generate`, não é gravado | Sessões com hooks (fp, ade) não são reconstruídas exatamente | `context_snapshot.categories.hook_guidance > 0` marca a reprodução como aproximada |
| A diferença no registry que motivou cada `model_notice` não é gravada | Não dá para montar o contexto de uma nova política de avisos | A mudança vem como texto: remover ou reescrever o aviso |
| Não há digest das skills por turno | Não dá para saber qual versão do texto rodou | O texto vem do snapshot do system prompt capturado na análise |

## 9. Limites conhecidos

- **Sessões anteriores a 28/09** não têm os avisos gravados e caem em "não reproduzível".
- **Um passo só mostra o efeito local.** O `tree` extra do ADE pode ser trabalho apenas adiantado. A evolução natural são execuções curtas de alguns passos, executando de verdade as chamadas de ferramentas só de leitura.
- **N = 20 só detecta efeitos grandes.** Eventos de ~15% pedem ~50 amostras por lado para uma conclusão firme; "Run 30 more" cobre isso.
- **A mudança é texto, não o código candidato.** Para uma mudança de código que altera o que o modelo vê, a pessoa confirma que o texto representa o que o código novo mostraria. O teste do mecanismo (a chamada gravada contra o código novo) acompanha a implementação e entra no brief.
- **Custo comparado depende do cache.** Por isso a chamada de aquecimento vem antes de cada variante.

## 10. Critérios de aceite

Página: `#/worker/eval/eval-benchmarks`.

- **VAL-01** Um cartão de sugestão com `check` mostra o ponto de decisão, o sinal e **Reproduce** com estimativa de custo. Verify: abrir uma análise nova com sugestão.
- **VAL-02** Reproduce no caso `eval_8383b53c` (reanalisado) termina com "Reproduced in k of 20", exemplos ao lado do passo original e selo "Reconstruction matches the original". Verify: rodar e abrir dois exemplos.
- **VAL-03** O progresso aparece ao vivo e sobrevive a recarregar a página. Verify: recarregar durante a reprodução.
- **VAL-04** Test change abre o diff pré-preenchido. Editar o texto e rodar mostra a tabela base × mudança, a frase do resultado e a linha de efeitos colaterais quando houver. Verify: rodar no mesmo caso.
- **VAL-05** Com resultado inconclusivo, Run 30 more acrescenta amostras às duas variantes e atualiza a frase. Verify: estender e conferir N = 50.
- **VAL-06** Approve e Dismiss gravam `accepted` e `rejected` com o motivo pré-preenchido, e o brief copiado inclui a evidência. Verify: aprovar e colar o brief.
- **VAL-07** Uma análise de sessão anterior a 28/09 mostra "não reproduzível" com o motivo e oferece Validate in E2E. Verify: abrir a análise do `state_machine_canvas_build`.
- **VAL-08** Provider indisponível no meio da reprodução mantém as amostras obtidas, mostra o erro e Retry completa o que falta. Verify: parar o provider durante a reprodução.
- **VAL-09** Celular, painel estreito e largo, nos dois temas, sem avisos no manifesto. Verify: capturas nas três larguras.
- **VAL-10** O custo real aparece no resultado e no consumo do monitor, separado da tarefa observada. Verify: comparar com `eval::config.cost`.

## 11. Estado da implementação (05/10, local, sem commit)

Feito: os passos 1 a 6 abaixo, com testes (Rust: 81 de biblioteca, 42 de fluxo, schemas; UI: 307). Conferido ao vivo:
- dry run nos três casos reais (ADE exato; kanban 0,9%; canvas aproximado por causa de hook e de sessão antiga);
- uma reprodução real de 5 amostras no kanban (US$ 0,03).

Diferenças em relação ao texto acima:
- A estatística usa o teste de Fisher **bicaudal**. No caso do ADE, 3/20 × 0/20 dá p = 0,23, e não os 0,12 unicaudais citados na §2.
- Quando o provider não conta tokens (deepseek, claude-code), a fidelidade usa a estimativa do context-manager.
- "Run N more per side" estende a base e depois a mudança: uma reprodução por vez por sugestão.
- Não suportado ainda: exposição `native`, turnos com contrato de saída e janelas podadas ou compactadas. Esses casos caem em "não reproduzível", com o motivo.
- Falta conferir: uma análise nova com `check` preenchido pela LLM. O prompt já pede o campo; isso exige uma investigação real.

## 12. Implementação em passos

Cada passo pode ser revisado e entregue sozinho.

1. **Captura** (§6.2) nos assets da análise. Teste: `flow.rs` confere que a captura existe e é a do turno analisado, mesmo depois de outro turno na mesma sessão.
2. **Reconstrução** como função pura: entradas + captura + mudança → pedido. Testes unitários com as duas sessões dos experimentos exportadas como fixture: ordem das mensagens, avisos convertidos, composição do system prompt e edições aplicadas ou recusadas.
3. **`eval::reproduce`**: fila, armazenamento em `reproductions[]`, conferência de fidelidade, amostragem e custo. Teste: `flow.rs` com `router::complete` e `router::count_tokens` simulados; teste unitário do Fisher e da frase do resultado.
4. **Classificação:** as duas regras e a pergunta via Jev. Testes com respostas gravadas dos experimentos (a armadilha do ADE; aviso mencionado e não mencionado no kanban).
5. **Contrato da LLM:** o objeto `check`, o prompt e a validação no backend (`decision_point` entre as referências, `find` presente no texto); `scenario_id` vindo do metadata em sessões do E2E.
6. **Interface:** a seção Validation com os estados da §4.1, o diálogo da §4.2 e o E2E recolhido. Testes do modelo de estados e conferência visual nas três larguras (skill `design-diff`).
7. **Verificação ao vivo:** reproduzir os dois casos dos experimentos e conferir que os resultados ficam na mesma faixa (ADE: armadilha em torno de 15% na base; kanban: mesma distribuição de ações com e sem aviso).
