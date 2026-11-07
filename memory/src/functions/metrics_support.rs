//! Medidas puras: somente escalares e enums, sem conteúdo ou identidade.
//! A integração mede os bytes UTF-8 efetivos e particiona cada região uma vez.
//! Adaptadores de telemetria aceitam somente campos conhecidos do router.

use serde::Serialize;

/// Limite aproximado em bytes; saturação explícita evita wrap para u64::MAX.
/// O limite não inclui automaticamente cabeçalhos ou outros componentes.
pub fn byte_budget(recall_budget_tokens: u64) -> u64 {
    recall_budget_tokens.saturating_mul(4)
}

/// ceil(bytes / 4), sem somar 3 ao numerador (que poderia transbordar).
/// Estimativa de bytes UTF-8, nunca contagem de tokens do provider.
pub fn estimate_tokens_from_bytes(total_bytes: u64) -> u64 {
    total_bytes.div_ceil(4)
}

/// Falhas fechadas, sem mensagem livre que possa carregar conteúdo sensível.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricsError {
    ArithmeticOverflow,
    SizeMismatch,
    StageMismatch,
    InvalidCost,
}

/// Partição dos bytes realmente injetados; os dois campos excluded ficam fora
/// do total injetado. Decorações não incluem texto, separadores ou wrappers;
/// rule_body_kept não inclui marcadores de corte, títulos ou lembretes.
/// Cabeçalhos/wrappers incluem suas próprias quebras de linha, sem repeti-las
/// em line_separator_bytes (separadores somente entre linhas de memórias).
///
/// O chamador deve medir regiões disjuntas. validate compara a soma com uma
/// medição independente; só escalares não permitem provar a origem das regiões.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SizeBreakdown {
    pub memory_text_bytes: u64,
    pub memory_line_decoration_bytes: u64,
    pub line_separator_bytes: u64,
    pub recall_wrapper_header_bytes: u64,
    pub rule_body_kept_bytes: u64,
    pub rule_formatting_marker_bytes: u64,
    pub ambient_header_bytes: u64,
    pub reminder_bytes: u64,
    pub rules_update_wrapper_bytes: u64,
    pub excluded_memory_line_bytes: u64,
    pub excluded_rule_body_bytes: u64,
}

impl SizeBreakdown {
    pub fn injected_bytes(&self) -> Result<u64, MetricsError> {
        [
            self.memory_text_bytes,
            self.memory_line_decoration_bytes,
            self.line_separator_bytes,
            self.recall_wrapper_header_bytes,
            self.rule_body_kept_bytes,
            self.rule_formatting_marker_bytes,
            self.ambient_header_bytes,
            self.reminder_bytes,
            self.rules_update_wrapper_bytes,
        ]
        .into_iter()
        .try_fold(0_u64, |sum, bytes| {
            sum.checked_add(bytes)
                .ok_or(MetricsError::ArithmeticOverflow)
        })
    }

    pub fn excluded_bytes(&self) -> Result<u64, MetricsError> {
        self.excluded_memory_line_bytes
            .checked_add(self.excluded_rule_body_bytes)
            .ok_or(MetricsError::ArithmeticOverflow)
    }

    pub fn validate(&self, observed_injected_bytes: u64) -> Result<(), MetricsError> {
        let injected = self.injected_bytes()?;
        self.excluded_bytes()?;
        if injected != observed_injected_bytes {
            return Err(MetricsError::SizeMismatch);
        }
        Ok(())
    }
}

/// Uma transição numa unidade estável: input + added = output + dropped.
/// Acréscimos de atualizações são explícitos, nunca descartes negativos ou
/// candidate - selected. O chamador fornece os descartes efetivamente observados.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct StageCounts {
    pub input: u64,
    pub added: u64,
    pub output: u64,
    pub dropped: u64,
}

impl StageCounts {
    pub fn validate(&self) -> Result<(), MetricsError> {
        let available = self
            .input
            .checked_add(self.added)
            .ok_or(MetricsError::ArithmeticOverflow)?;
        let accounted = self
            .output
            .checked_add(self.dropped)
            .ok_or(MetricsError::ArithmeticOverflow)?;
        if available != accounted {
            return Err(MetricsError::StageMismatch);
        }
        Ok(())
    }
}

/// Uso reportado de uma operação Memory, não totais da sessão Harness.
/// Campos privados garantem custo finito e não negativo antes de Debug/Serialize.
/// Sem Deserialize ou campos de texto/Value; o adaptador passa só escalares.
/// None sempre serializa null; Some(0) é uma medição conhecida e distinta.
/// Cache e reasoning não são somados a input/output, pois podem sobrepor-se.
/// cost_usd é o valor recebido, sem inferir preço ou promessa de faturamento.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct ReportedUsage {
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    reasoning: Option<u64>,
    cost_usd: Option<f64>,
}

impl ReportedUsage {
    pub fn new(
        input: Option<u64>,
        output: Option<u64>,
        cache_read: Option<u64>,
        cache_write: Option<u64>,
        reasoning: Option<u64>,
        cost_usd: Option<f64>,
    ) -> Result<Self, MetricsError> {
        if let Some(cost) = cost_usd {
            if !cost.is_finite() || cost < 0.0 {
                return Err(MetricsError::InvalidCost);
            }
        }
        Ok(Self {
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
            cost_usd,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    Absent,
    TopLevel,
    Message,
    Duplicate,
    Conflict,
}

/// Uma única observação por resposta. Estado privado evita pares contraditórios.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CanonicalUsage {
    status: UsageStatus,
    usage: ReportedUsage,
}

impl CanonicalUsage {
    pub fn status(&self) -> UsageStatus {
        self.status
    }

    pub fn usage(&self) -> ReportedUsage {
        self.usage
    }
}

/// Dois usos iguais são cópias da mesma resposta, nunca duas cobranças.
/// Qualquer diferença (inclusive presença versus ausência de um campo) torna
/// a observação inteira desconhecida e sinaliza conflito. Não escolhe backend
/// nem combina parciais complementares sem evidência de identidade da medição.
pub fn canonical_usage(
    top_level: Option<ReportedUsage>,
    message: Option<ReportedUsage>,
) -> CanonicalUsage {
    let (status, usage) = match (top_level, message) {
        (None, None) => (UsageStatus::Absent, ReportedUsage::default()),
        (Some(usage), None) => (UsageStatus::TopLevel, usage),
        (None, Some(usage)) => (UsageStatus::Message, usage),
        (Some(top), Some(message)) if top == message => (UsageStatus::Duplicate, top),
        (Some(_), Some(_)) => (UsageStatus::Conflict, ReportedUsage::default()),
    };
    CanonicalUsage { status, usage }
}

/// total só existe com ao menos uma observação e nenhuma ausência.
/// subtotal_observed=0 sem conhecidos é identidade de soma, não uso gratuito.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TokenTotal {
    pub total: Option<u64>,
    pub subtotal_observed: u64,
    pub known_count: u64,
    pub missing_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CostTotal {
    pub total: Option<f64>,
    pub subtotal_observed: f64,
    pub known_count: u64,
    pub missing_count: u64,
}

struct PartialTotal<T> {
    total: Option<T>,
    subtotal_observed: T,
    known_count: u64,
    missing_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageAggregate {
    pub input: TokenTotal,
    pub output: TokenTotal,
    pub cache_read: TokenTotal,
    pub cache_write: TokenTotal,
    pub reasoning: TokenTotal,
    pub cost_usd: CostTotal,
    pub conflict_count: u64,
}

fn aggregate_field<T: Copy>(
    values: impl Iterator<Item = Option<T>>,
    zero: T,
    checked_add: impl Fn(T, T) -> Option<T>,
) -> Result<PartialTotal<T>, MetricsError> {
    let mut subtotal_observed = zero;
    let mut known_count = 0_u64;
    let mut missing_count = 0_u64;
    for value in values {
        if let Some(value) = value {
            subtotal_observed =
                checked_add(subtotal_observed, value).ok_or(MetricsError::ArithmeticOverflow)?;
            known_count = known_count
                .checked_add(1)
                .ok_or(MetricsError::ArithmeticOverflow)?;
        } else {
            missing_count = missing_count
                .checked_add(1)
                .ok_or(MetricsError::ArithmeticOverflow)?;
        }
    }
    Ok(PartialTotal {
        total: (known_count > 0 && missing_count == 0).then_some(subtotal_observed),
        subtotal_observed,
        known_count,
        missing_count,
    })
}

fn aggregate_tokens(values: impl Iterator<Item = Option<u64>>) -> Result<TokenTotal, MetricsError> {
    let result = aggregate_field(values, 0, u64::checked_add)?;
    Ok(TokenTotal {
        total: result.total,
        subtotal_observed: result.subtotal_observed,
        known_count: result.known_count,
        missing_count: result.missing_count,
    })
}

fn aggregate_cost(values: impl Iterator<Item = Option<f64>>) -> Result<CostTotal, MetricsError> {
    let result = aggregate_field(values, 0.0, |a, b| {
        let sum = a + b;
        sum.is_finite().then_some(sum)
    })?;
    Ok(CostTotal {
        total: result.total,
        subtotal_observed: result.subtotal_observed,
        known_count: result.known_count,
        missing_count: result.missing_count,
    })
}

/// A fatia contém uma observação canônica por chamada Memory distinta.
/// Não deduplica chamadas diferentes nem aceita acumuladores de sessão.
/// Overflow em qualquer campo falha explicitamente, sem publicar total parcial
/// como completo. Custo usa soma f64 finita, sem fabricar exatidão monetária.
pub fn aggregate_usage(reports: &[CanonicalUsage]) -> Result<UsageAggregate, MetricsError> {
    let conflict_count = reports.iter().try_fold(0_u64, |count, report| {
        count
            .checked_add(u64::from(report.status() == UsageStatus::Conflict))
            .ok_or(MetricsError::ArithmeticOverflow)
    })?;
    Ok(UsageAggregate {
        input: aggregate_tokens(reports.iter().map(|r| r.usage().input))?,
        output: aggregate_tokens(reports.iter().map(|r| r.usage().output))?,
        cache_read: aggregate_tokens(reports.iter().map(|r| r.usage().cache_read))?,
        cache_write: aggregate_tokens(reports.iter().map(|r| r.usage().cache_write))?,
        reasoning: aggregate_tokens(reports.iter().map(|r| r.usage().reasoning))?,
        cost_usd: aggregate_cost(reports.iter().map(|r| r.usage().cost_usd))?,
        conflict_count,
    })
}

/// Telemetria do caminho produtivo; nenhuma identidade de memória ou conteúdo.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RetrievalMetrics {
    pub budget_tokens: u64,
    pub budget_bytes: u64,
    pub sizes: SizeBreakdown,
    pub estimated_tokens: u64,
    pub candidates: u64,
    pub selected: u64,
    pub unsupported: u64,
    pub selection_policy_dropped: u64,
    pub already_visible: u64,
    pub budget_dropped: u64,
    pub updates_added: u64,
    pub updates_removed: u64,
    pub mode: &'static str,
    pub duration_ms: u64,
    /// Construção atual; entrega pode repetir a seção congelada da sessão.
    pub delivered_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionOutcome {
    Success,
    ModelUnavailable,
    TransportError,
    Timeout,
    ProviderError,
    InvalidReply,
}

/// Identificadores restritos, nunca texto livre nem JSON de resposta.
pub fn metric_id(value: Option<&str>) -> Option<String> {
    value
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
                && crate::redact::redact(s).labels.is_empty()
        })
        .map(str::to_owned)
}

/// Somente Usage documentado: ignora campos extras e não herda totais Harness.
pub fn reply_usage(reply: &serde_json::Value) -> CanonicalUsage {
    fn read(value: &serde_json::Value) -> Option<ReportedUsage> {
        value.as_object()?;
        ReportedUsage::new(
            value["input"].as_u64(),
            value["output"].as_u64(),
            value["cache_read"].as_u64(),
            value["cache_write"].as_u64(),
            value["reasoning"].as_u64(),
            value["cost_usd"].as_f64(),
        )
        .ok()
    }
    canonical_usage(read(&reply["usage"]), read(&reply["message"]["usage"]))
}

#[derive(Debug, Serialize)]
pub struct ExtractionMetrics {
    pub outcome: ExtractionOutcome,
    pub routing: &'static str,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub usage: CanonicalUsage,
    pub duration_ms: u64,
}

impl ExtractionMetrics {
    pub fn new(model: &str, routing: &'static str) -> Self {
        Self {
            outcome: ExtractionOutcome::ModelUnavailable,
            routing,
            requested_model: metric_id(Some(model)),
            effective_model: None,
            provider: None,
            usage: canonical_usage(None, None),
            duration_ms: 0,
        }
    }

    pub fn emit(&self, session_id: &str, turn_id: Option<&str>) {
        tracing::info!(target: "memory_metrics", session_id = ?metric_id(Some(session_id)),
            turn_id = ?metric_id(turn_id), metrics = ?self, "memory extraction metrics");
    }
}
