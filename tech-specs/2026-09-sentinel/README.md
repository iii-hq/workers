# sentinel — spec

Monitor de erros do iii: observa a telemetria do engine (spans e logs OTel),
agrupa as falhas por fingerprint determinístico, congela a evidência antes que
o store em memória a descarte, e abre investigações assistidas no harness —
a sessão ao lado da página, o usuário acompanha e direciona — que devolvem um
diagnóstico estruturado. Resolver e ignorar são decisões humanas; regressão é
detectada automaticamente.

| Documento | Conteúdo |
|---|---|
| [sentinel.md](sentinel.md) | A spec: fontes, ingest, fingerprint, ciclo de vida, evidência, tier de decisões, investigação (`DiagnosisV1`), modelo de dados, funções, triggers, configuração, UI, durabilidade, fronteiras, questões abertas |

O worker tem dois tiers de modelo, os dois opcionais e os dois desligados por
padrão. O **tier de decisões** troca as heurísticas da spec (janela de junção,
span folha, quais bundles reter) por decisões calibradas de um modelo System
One, cada uma com a heurística atrás como fallback obrigatório — ligar melhora
a qualidade, nunca muda o contrato. O **modelo de triagem** escreve a prosa
que um modelo de decisão não escreve (título, resumo, hipótese rotulada).
A investigação é outra coisa: um modelo forte, no harness, sob o olho do
usuário.

Prior art no repositório: [`security-scan`](../../security-scan) (fila durável
→ `harness::send` jailed com `output_schema` → doorbell `turn-completed` →
reconcile), a página de traces do console (`ade/web/src/pages/TracesV2`) para
os contratos `engine::traces::*` / `engine::logs::*`, e o SOP
[`trace-hidden-functions.md`](../../docs/sops/trace-hidden-functions.md) para
a regra "o pipeline não observa a si mesmo".
