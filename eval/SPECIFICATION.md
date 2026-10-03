# Especificação do eval: monitor de sessões do Harness

**Status:** proposta para implementação.  
**Data:** 2026-10-02.  
**Escopo deste documento:** comportamento esperado, responsabilidades, resultados e critérios de aceite. Os passos técnicos estão no [guia de implementação](IMPLEMENTATION.md).

## 1. Objetivo

Transformar o worker `eval` em um monitor que observa o uso do Harness, identifica oportunidades de melhoria e produz sugestões acompanhadas de evidências e de um plano de validação no E2E.

O objeto das melhorias é o projeto do Harness: execução de ferramentas, recuperação de erros, gerenciamento de contexto, coordenação de sessões e comportamento de execução. Uma tarefa pode terminar corretamente e ainda revelar trabalho desnecessário.

O monitor deve responder:

- O que aconteceu durante a execução?
- Qual comportamento merece investigação?
- Qual mudança no Harness pode resolver o problema?
- Como reproduzir o comportamento e verificar se a mudança ajudou?

O fluxo de avaliações de prompt será substituído pelo monitor de sessões. A comparação objetiva de sessões continua disponível como apoio à investigação.

## 2. Decisões de escopo

| Tema | Decisão |
| --- | --- |
| Execução | Análise automática e assíncrona de sessões elegíveis, sem bloquear a tarefa original. |
| Jev | Usar o provider TypeSafe existente para julgamentos com respostas delimitadas. |
| LLM convencional | Usar o modelo e o provider escolhidos pelo usuário do Harness para investigar e redigir sugestões. |
| Configuração | O monitor começa inativo e exige configuração explícita antes da primeira análise com modelos. |
| Interface | Substituir avaliações de prompt por configuração, histórico e detalhes do monitor. |
| Validação | O E2E verifica a mudança no Harness com cenários e critérios independentes da sugestão. |
| Autonomia inicial | Observar, analisar e sugerir. Aplicar alterações e executar campanhas de validação são ações separadas. |

Esta etapa não inclui edição automática do código, criação automática de PR, merge, deploy ou alteração da sessão observada. O monitor também não substitui o executor do Harness, o router, o armazenamento de sessões ou o runner E2E.

## 3. Responsabilidade de cada camada

| Camada | Responsabilidade | Resultado |
| --- | --- | --- |
| Código determinístico | Ler evidências, correlacionar chamadas e resultados, contar eventos, calcular métricas e identificar padrões conhecidos. | Observações verificáveis e referências para a sessão. |
| Jev | Classificar sinais em categorias previamente definidas e expressar incerteza. | Triagem estruturada. |
| LLM convencional | Relacionar sinais, formular hipóteses, propor mudanças e descrever como testá-las. | Sugestões fundamentadas. |
| E2E | Executar o mesmo caso contra o Harness de referência e o candidato, verificar a tarefa e comparar esforço. | Evidência de melhoria, regressão ou resultado inconclusivo. |

Jev retorna decisões tipadas, como Choice, Noul e Score. Não gera explicações ou código; por isso a investigação e a redação ficam com a LLM. A calibração de probabilidades não garante que uma resposta individual esteja correta. [Referência: System One](https://docs.typesafe.ai/concepts/system-one).

Contagens, durações, custos e diferenças entre execuções devem ser calculados em código. O contexto enviado a Jev deve conter apenas os dados relevantes para a pergunta, com critérios diretos. [Referência: limitações de Jev](https://docs.typesafe.ai/model-jaggedness/jev-1.13).

`confidence` descreve a distribuição entre as opções de uma pergunta; não representa a probabilidade de uma sugestão melhorar o Harness. Os limiares de triagem precisam ser avaliados com dados do projeto. [Referência: confidence](https://docs.typesafe.ai/confidence).

A divisão híbrida é uma escolha de responsabilidade. Sua qualidade e seu custo devem ser medidos; não se presume que Jev seja superior à LLM para qualquer análise.

## 4. Sessões observadas

### 4.1 Elegibilidade

O monitor deve observar sessões do Harness concluídas, com erro ou canceladas. Sessões bem-sucedidas também precisam ser analisadas: sucesso não elimina repetição, desperdício de contexto ou coordenação ineficiente.

Uma sessão raiz e seus descendentes formam uma unidade de observação. O monitor deve preservar a origem dos sinais em cada sessão e evitar contabilizar um descendente como uma segunda execução independente da mesma tarefa.

Uma conclusão intermediária com continuação pendente não deve ser tratada como encerramento definitivo. Se houver descendentes em execução ou métricas incompletas, a coleta permanece pendente dentro do orçamento da análise.

Sessões criadas pelo próprio monitor para investigar evidências devem ser excluídas da observação automática, inclusive seus descendentes. Essa exclusão evita um ciclo de avaliações das próprias avaliações.

### 4.2 Captura e rastreabilidade

Cada análise deve identificar a sessão e o turno que a originaram, o momento da captura e a versão das regras usadas.

As evidências devem incluir, quando disponíveis:

- Estado de execução, motivo de parada e erros.
- Mensagens, chamadas de ferramentas, resultados e avisos apresentados ao modelo.
- Relações entre sessões raiz e descendentes.
- Chamadas, erros, tokens, custo, duração e informações de contexto.
- Modelo e provider usados pela tarefa observada.

Métricas acumuladas da sessão inteira devem ser identificadas como tal. Elas não podem ser apresentadas como métricas exclusivas do último turno. Sinais históricos já reportados devem conservar sua identidade para evitar alertas repetidos sem nova ocorrência.

O resultado deve preservar referências para as entradas originais e a identificação da evidência capturada. Prévias reduzidas precisam indicar omissões ou truncamento. Leitura parcial não equivale a ausência de problema.

Se a sessão mudar durante a coleta e impedir uma captura coerente, a análise deve registrar a limitação. Não deve misturar turnos e apresentar o conjunto como uma reprodução fiel da execução anterior.

## 5. Detecção e triagem

### 5.1 Sinais iniciais

| Sinal | Evidência necessária | Limite da conclusão |
| --- | --- | --- |
| Redescoberta de contratos já disponíveis | Resultado de descoberta que informa contratos inalterados em contexto, ligado às chamadas e aos resultados anteriores que forneceram esses contratos. | Indica trabalho potencialmente redundante. Não prova sozinho que a chamada era desnecessária. |
| Aviso do Harness seguido de redescoberta | Aviso apresentado ao modelo entre a descoberta original e a chamada repetida. | Indica correlação temporal; a causa precisa ser validada por uma mudança controlada. |
| Repetição do mesmo erro | Chamadas consecutivas ao mesmo alvo, argumentos equivalentes, resultados ligados aos IDs corretos e mesmo código explícito de erro. | Indica recuperação potencialmente ineficaz. Erros transitórios podem justificar novas tentativas. |

Uma segunda chamada com argumentos corrigidos, uma tentativa que termina com sucesso ou uma descoberta que traz contratos novos não deve ser marcada como a mesma repetição problemática. Sondagens esperadas pelo protocolo também precisam ser distinguíveis de falhas.

Os detectores devem registrar a observação antes de qualquer julgamento do modelo. Jev e a LLM não podem apagar uma ocorrência verificável; podem explicar por que ela talvez seja esperada ou por que exige mais evidência.

### 5.2 Triagem com Jev

A triagem deve usar perguntas independentes e respostas delimitadas. A classificação inicial deve distinguir:

- **Investigar:** existe um sinal que merece análise de melhoria no Harness.
- **Comportamento esperado:** a evidência disponível sustenta uma explicação normal para a execução.
- **Evidência insuficiente:** o material não permite concluir se há um problema.

A análise deve registrar o modelo efetivamente retornado pelo provider, as respostas, as probabilidades, a versão dos critérios e o uso reportado.

O acesso a Jev deve reutilizar o hub `judge` e o provider `judge-typesafe` existentes. Credenciais, transporte e controles do provider permanecem nessa integração; o `eval` não cria outro cliente HTTP nem armazena uma segunda chave TypeSafe.

### 5.3 Investigação com a LLM

A LLM deve ser acionada para sinais conhecidos relevantes, casos indicados pela triagem e uma amostra de sessões sem sinais conhecidos. Essa amostra permite descobrir problemas que os detectores ainda não reconhecem.

A seleção da amostra deve ser rastreável. Sua taxa, os limiares de triagem e os limites de análise serão definidos no guia técnico e avaliados com dados reais. Uma classificação de comportamento esperado não deve excluir definitivamente uma sessão de auditoria.

A LLM recebe um resumo das evidências e usa o modelo escolhido pelo usuário. Uma mudança posterior dessa configuração não deve alterar silenciosamente a identidade de uma análise já iniciada.

O monitor pode concluir sem sugestões. Não deve inventar uma melhoria para preencher um resultado.

## 6. Conteúdo de uma sugestão

Toda sugestão deve conter:

| Campo | Conteúdo esperado |
| --- | --- |
| Título | Problema ou oportunidade em uma frase. |
| Observação | Fato encontrado na execução. |
| Evidências | Sessões, turnos e entradas que sustentam a observação. |
| Hipótese | Explicação possível, distinguindo-a de uma causa comprovada. |
| Componente do Harness | Área plausível para investigação, sem inventar arquivos ou linhas não inspecionados. |
| Mudança proposta | Intervenção concreta no comportamento do Harness. |
| Efeito esperado | O que deve melhorar e sob quais condições. |
| Plano E2E | Reprodução, invariantes de correção, métrica de esforço e expectativa de comparação. |
| Limitações | Evidências ausentes, explicações alternativas e necessidade de um cenário novo. |

Referências inexistentes devem invalidar a sugestão. Um plano E2E precisa permitir a alguém reproduzir o problema e avaliar a mudança sem depender da opinião da LLM que o propôs.

## 7. Integração com o E2E

### 7.1 Caminho da observação à prova

1. O monitor registra um comportamento e uma sugestão com evidências.
2. A sugestão é revisada e ligada a um cenário existente, ou a um caso novo que reproduz o problema.
3. O cenário e seus critérios são fixados antes da comparação. Se for necessário ampliá-lo, a mesma versão ampliada deve ser executada na referência e no candidato.
4. A mudança é desenvolvida no Harness.
5. O runner E2E executa o caso nas duas versões e produz os resultados e os assets de evidência.
6. A comparação verifica a tarefa correta, o comportamento investigado e as métricas de esforço.
7. O resultado é associado à sugestão, com as versões e execuções que o sustentam.

O monitor produz o plano de validação. A execução da campanha e a aplicação da mudança dependem de uma ação explícita nesta primeira versão.

### 7.2 Condições para uma comparação válida

Referência e candidato devem usar o mesmo caso, entradas, versão do cenário, critérios de avaliação, modelo/provider e parâmetros suportados. Dependências, Engine e condições de execução precisam ser comparáveis; a diferença planejada é a versão do Harness.

As sementes do cenário devem ser fixadas. Quando o provider oferecer controles de aleatoriedade, eles também devem ser registrados. Repetir o mesmo caso não garante respostas idênticas de um modelo; métricas sujeitas a variação exigem repetições e um critério de avaliação definido antes da campanha.

Versões, revisões, contratos relevantes e evidências devem acompanhar o resultado. Execuções incompletas, falhas de infraestrutura e medidas ausentes precisam ser identificadas, sem exclusão silenciosa para favorecer o candidato. Custo desconhecido não deve ser convertido em zero.

### 7.3 Correção e esforço

| Dimensão | O que comprova |
| --- | --- |
| Correção da tarefa | Entregável e invariantes satisfeitos por um avaliador independente. |
| Comportamento investigado | O padrão indesejado deixou de ocorrer, ou diminuiu conforme o critério declarado. |
| Esforço | Chamadas, erros, tokens, tempo e custo, respeitando disponibilidade e variação. |
| Não regressão | Comportamentos necessários continuam funcionando em cenários relevantes. |

Cada resultado deve distinguir **melhoria validada**, **sem melhoria**, **regressão** e **inconclusivo**, indicando o critério aplicado. Esses resultados pertencem ao experimento E2E e não ao estado de execução da análise do monitor.

Um ganho em chamadas não autoriza afirmar redução de tempo ou custo se essas medidas não sustentarem a afirmação. Um candidato mais barato que entrega a tarefa errada é uma regressão, não uma melhoria.

A comparação de sessões do `eval` auxilia a leitura das métricas. Ela, sozinha, não comprova equivalência entre tarefas nem substitui os critérios do E2E.

### 7.4 Exemplo: aviso que induz redescoberta de ferramentas

**Observação:** o modelo recebe contratos de ferramentas. Depois de um aviso de mudança no registry, consulta novamente os mesmos contratos e recebe apenas `unchanged_in_context`.

**Hipótese:** um aviso amplo sobre mudanças no registry pode induzir nova consulta mesmo quando os contratos usados pela tarefa continuam válidos.

**Mudança proposta:** revisar no Harness quando o aviso é emitido e o que ele informa, preservando a recuperação quando um contrato realmente muda.

**Reprodução:** partir do cenário `tool_contract_recovery`, no qual um runbook antigo deve ser resolvido para as ferramentas atuais de perfil e agendamento. Garantir uma alteração controlada e não relacionada no registry entre a descoberta inicial e a próxima decisão do modelo. Se esse gatilho não estiver no caso existente, acrescentá-lo antes de executar ambas as versões.

**Invariantes:** resolver o contrato antigo, obter os dados atuais de perfil, agendar exatamente uma vez, retornar o recibo correto e não executar a ferramenta destrutiva de distração. A correção deve ser conferida com o transcript, o registro independente das ferramentas e o estado final da fixture.

**Métrica principal:** quantidade de redescobertas dos mesmos contratos sem informação nova, vinculadas ao aviso. Comparar também chamadas totais, tokens, tempo e custo.

**Expectativa:** reduzir a redescoberta induzida pelo aviso, mantendo os invariantes. O experimento deve incluir o controle em que o contrato realmente muda: a otimização não pode eliminar a recuperação necessária.

Este exemplo descreve o teste que deve ser feito; não declara que o novo monitor já foi implementado ou que a hipótese já foi validada por ele.

## 8. Interface e operação

A tela principal do `eval` deve oferecer configuração do monitor, seleção da LLM, ativação ou pausa da observação automática, análise manual de uma sessão elegível, histórico e detalhes das análises.

Os detalhes devem permitir ler evidências, classificação de Jev, hipóteses, sugestões e o plano E2E. A comparação de sessões permanece acessível. A interface de experimentos de prompt será removida.

O usuário deve conseguir distinguir coleta pendente, triagem, investigação, conclusão sem sugestão, conclusão com sugestões, falha e cancelamento. A falha de uma análise não deve aparecer como resultado saudável da sessão.

Pausar a observação impede novas análises automáticas. Cancelar uma análise em andamento é uma ação separada e afeta apenas o trabalho do monitor.

Análises devem ter limites de duração, chamadas, tokens e concorrência. Quando o custo puder ser medido, o consumo também deve ficar visível. Valores e política de retenção serão definidos no guia técnico.

Eventos repetidos e retomadas após reinício não devem criar análises ou chamadas de modelo duplicadas. Trabalho pendente precisa ser recuperável e rastreável.

Se Jev ou a LLM estiverem indisponíveis, o monitor deve conservar as observações já obtidas e registrar a etapa que falhou. Não deve substituir silenciosamente o provider ou o modelo escolhido.

O consumo de Jev e da LLM deve ser contabilizado separadamente do consumo da tarefa observada. O custo do monitor faz parte da avaliação de viabilidade do projeto.

## 9. Tratamento das evidências

Antes de enviar material a providers, o monitor deve minimizar o contexto e remover credenciais dos campos estruturados e dos resultados que conseguir interpretar. Os registros originais permanecem no armazenamento de sessões conforme sua política de acesso.

Texto da sessão, respostas de ferramentas e avisos são dados de análise. Não podem conceder autorização ao monitor para executar ações ou modificar o projeto. Máscara de campos e truncamento não garantem, sozinhos, que todo conteúdo sensível de texto livre foi removido; essa cobertura precisa ser especificada e testada.

## 10. Critérios de aceite

- A observação é inativa até que o usuário configure e ative o monitor; a LLM utilizada corresponde à seleção registrada.
- Uma sessão concluída, com erro ou cancelada pode gerar análise automática; sessões saudáveis também participam da amostragem.
- Sessões do próprio monitor não produzem novas análises automáticas.
- Eventos duplicados e retomadas não duplicam a análise de uma mesma ocorrência.
- Coleta parcial, turno alterado e provider indisponível ficam explícitos no resultado.
- Os padrões iniciais são detectados por evidência correlacionada, com casos de controle para recuperações legítimas.
- Jev é acessado pelo provider existente e retorna triagem estruturada, sem substituir contagens determinísticas.
- A LLM pode produzir uma lista vazia e não pode publicar uma sugestão com referências inexistentes.
- Cada sugestão contém uma reprodução E2E, um invariante de correção e uma métrica de esforço.
- O exemplo de redescoberta pode ser reproduzido e comparado em duas versões do Harness com a mesma versão do cenário.
- O controle de contrato realmente alterado continua exigindo recuperação válida.
- Nenhuma sugestão aparece como melhoria comprovada sem resultados E2E independentes e rastreáveis.
- O consumo da análise fica separado das métricas da tarefa observada.
- A interface substitui avaliações de prompt pelo monitor e mantém a comparação objetiva de sessões.

A qualidade do monitor deve ser avaliada com um conjunto de sessões revisadas: problemas detectados, falsos alertas, problemas perdidos na amostra auditada, utilidade das sugestões e custo por análise. Isso mede o monitor; a campanha antes/depois mede a melhoria proposta no Harness.

## 11. Guia de implementação

O [guia técnico](IMPLEMENTATION.md) transforma esta especificação em passos de implementação e validação, cobrindo contratos públicos, configuração, captura de evidências, detecção, integração com o judge, chamadas à LLM, persistência, recuperação, limites, interface, testes e ligação com os resultados E2E.

O guia também define o destino dos registros antigos de avaliações de prompt e a transição da interface. Este documento não estabelece compatibilidade automática com o fluxo que será substituído.

## Referências

- [Comportamento atual documentado do eval](README.md).
- [Hub judge](../judge/README.md) e [contratos de avaliação](../judge/reference.md).
- [Provider TypeSafe existente e sua configuração](../judge-typesafe/README.md).
- [Eventos do Harness](../harness/src/events.rs), [estado de execução](../harness/src/functions/status.rs), [árvore de sessões](../harness/src/functions/session_tree.rs) e [métricas](../harness/src/functions/metrics.rs).
- [Cenário tool_contract_recovery no Harness E2E](https://github.com/iii-hq/harness-e2e/blob/f42b9008af2da184ca83dfec49062d029be70130/src/scenarios/tool_contract_recovery.rs).
- [TypeSafe: System One](https://docs.typesafe.ai/concepts/system-one), [limitações de Jev](https://docs.typesafe.ai/model-jaggedness/jev-1.13) e [confidence](https://docs.typesafe.ai/confidence).
