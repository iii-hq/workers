//! Contrato puro de leitura incremental de `session::messages`.
//!
//! Não faz I/O, extração, filtragem de texto ou escrita de cursor. A âncora
//! `since` já foi consumida: a primeira página é strict-after, e as seguintes
//! usam somente o cursor opaco do servidor. `until` é um corte CLIENTE inclusivo,
//! nunca um campo da API ou uma escolha automática de branch.
//!
//! O chamador deve correlacionar sessão/job/banco, serializar as respostas de cada
//! leitor e confirmar persistência antes de usar qualquer watermark. Complete e
//! AlreadyConsumed descrevem somente este contrato, não autorizam commit, modelo
//! ou inferem idempotência persistente. Uma classificação/falha invalida todo o
//! delta privado; recuperação exige uma nova decisão/leitor do chamador.

use std::collections::HashSet;

use serde_json::{json, Value};

pub const PAGE_SIZE: usize = 100;

/// Limites obrigatórios de recursos; nunca representam truncamento bem-sucedido.
/// `max_entries` conta a página recebida inteira, inclusive após o corte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub max_pages: usize,
    pub max_entries: usize,
}

/// A integração traduz o erro de transporte para uma categoria explícita.
/// Não classificar por substring de uma mensagem de erro arbitrária.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchFailure {
    InvalidCursor,
    Transient,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    InvalidInput,
    MalformedPage,
    PageTooLarge,
    RepeatedCursor,
    RepeatedEntry,
    AnchorInDelta,
    NoProgress,
    PageLimit,
    EntryLimit,
    Fetch(FetchFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassificationReason {
    MissingAnchor,
    MissingBoundary,
    InvalidCursor,
    BoundaryNotFound,
}

/// Envelope preservado integralmente, sem interpretação de blocos de conteúdo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub entry_id: String,
    pub item: Value,
}

/// Somente entradas strict-after `since`, em ordem de resposta, até `until`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteDelta {
    pub entries: Vec<HistoryEntry>,
    /// Candidato ao watermark, não uma escrita/autoridade de persistência.
    pub watermark: String,
}

/// Nenhuma variante incompleta expõe o prefixo como delta elegível.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    InProgress,
    Complete(CompleteDelta),
    AlreadyConsumed { watermark: String },
    NeedsClassification(ClassificationReason),
    Failed(HistoryError),
}

/// Leitor de uma sequência de páginas; não contém transporte nem fallback.
///
/// IDs e cursores são opacos. Só igualdade identifica repetição/backtracking
/// observável; reordenação de IDs inéditos não pode ser provada localmente sem
/// informação de parentesco. A ordem dos IDs inéditos vem do contrato servidor.
#[derive(Debug)]
pub struct DeltaReader {
    session_id: String,
    since: Option<String>,
    until: Option<String>,
    bounds: Bounds,
    pages: usize,
    received_entries: usize,
    cursor: Option<String>,
    seen_cursors: HashSet<String>,
    seen_entries: HashSet<String>,
    pending: Vec<HistoryEntry>,
    outcome: Outcome,
}

impl DeltaReader {
    pub fn new(session_id: &str, since: Option<&str>, until: Option<&str>, bounds: Bounds) -> Self {
        let outcome = if session_id.is_empty()
            || since == Some("")
            || until == Some("")
            || bounds.max_pages == 0
            || bounds.max_entries == 0
        {
            Outcome::Failed(HistoryError::InvalidInput)
        } else {
            match (since, until) {
                (_, None) => Outcome::NeedsClassification(ClassificationReason::MissingBoundary),
                (None, Some(_)) => {
                    Outcome::NeedsClassification(ClassificationReason::MissingAnchor)
                }
                (Some(anchor), Some(end)) if anchor == end => Outcome::AlreadyConsumed {
                    watermark: end.to_owned(),
                },
                _ => Outcome::InProgress,
            }
        };
        Self {
            session_id: session_id.to_owned(),
            since: since.map(str::to_owned),
            until: until.map(str::to_owned),
            bounds,
            pages: 0,
            received_entries: 0,
            cursor: None,
            seen_cursors: HashSet::new(),
            seen_entries: HashSet::new(),
            pending: Vec::new(),
            outcome,
        }
    }

    /// Payload da próxima leitura. Consultar novamente não faz I/O nem avança.
    /// O chamador entrega exatamente uma resposta por página, em sequência.
    pub fn next_request(&self) -> Option<Value> {
        if self.outcome != Outcome::InProgress {
            return None;
        }
        let mut request = json!({
            "session_id": self.session_id,
            "limit": PAGE_SIZE,
            "roles": ["user", "assistant"],
            "include_image_data": false,
        });
        if let Some(cursor) = &self.cursor {
            request["cursor"] = json!(cursor);
        } else if let Some(since) = &self.since {
            request["after_entry_id"] = json!(since);
        }
        Some(request)
    }

    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    /// Valida a página inteira ANTES de liberar qualquer delta. Mesmo se o
    /// corte estiver nesta página, shape/cursor/IDs inválidos não viram sucesso.
    /// Respostas após um resultado terminal são ignoradas, sem reabrir o leitor.
    pub fn accept(&mut self, reply: Result<Value, FetchFailure>) {
        if self.outcome != Outcome::InProgress {
            return;
        }
        let reply = match reply {
            Ok(reply) => reply,
            Err(FetchFailure::InvalidCursor) => {
                self.stop(Outcome::NeedsClassification(
                    ClassificationReason::InvalidCursor,
                ));
                return;
            }
            Err(error) => {
                self.fail(HistoryError::Fetch(error));
                return;
            }
        };
        if let Err(error) = self.accept_page(&reply) {
            self.fail(error);
        }
    }

    fn accept_page(&mut self, reply: &Value) -> Result<(), HistoryError> {
        let object = reply.as_object().ok_or(HistoryError::MalformedPage)?;
        let messages = object
            .get("messages")
            .and_then(Value::as_array)
            .ok_or(HistoryError::MalformedPage)?;
        let next_cursor = match object.get("next_cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.clone()),
            _ => return Err(HistoryError::MalformedPage),
        };
        if messages.len() > PAGE_SIZE {
            return Err(HistoryError::PageTooLarge);
        }
        if let Some(next) = &next_cursor {
            if !self.seen_cursors.insert(next.clone()) {
                return Err(HistoryError::RepeatedCursor);
            }
            if messages.is_empty() {
                return Err(HistoryError::NoProgress);
            }
        }
        self.pages = self.pages.checked_add(1).ok_or(HistoryError::PageLimit)?;
        self.received_entries = self
            .received_entries
            .checked_add(messages.len())
            .ok_or(HistoryError::EntryLimit)?;
        if self.pages > self.bounds.max_pages {
            return Err(HistoryError::PageLimit);
        }
        if self.received_entries > self.bounds.max_entries {
            return Err(HistoryError::EntryLimit);
        }

        let mut page_entries = Vec::with_capacity(messages.len());
        for item in messages {
            let entry = parse_entry(item)?;
            if self.since.as_deref() == Some(entry.entry_id.as_str()) {
                return Err(HistoryError::AnchorInDelta);
            }
            if !self.seen_entries.insert(entry.entry_id.clone()) {
                return Err(HistoryError::RepeatedEntry);
            }
            page_entries.push(entry);
        }
        let cut = page_entries
            .iter()
            .position(|entry| self.until.as_deref() == Some(entry.entry_id.as_str()));
        let take = cut.map_or(page_entries.len(), |position| position + 1);
        self.pending.extend(page_entries.into_iter().take(take));
        if cut.is_some() {
            // O último item retido é precisamente o corte encontrado, nunca
            // o cursor de paginação ou o leaf de uma página posterior.
            let watermark = self
                .pending
                .last()
                .expect("corte presente")
                .entry_id
                .clone();
            let entries = std::mem::take(&mut self.pending);
            self.stop(Outcome::Complete(CompleteDelta { entries, watermark }));
        } else if let Some(next) = next_cursor {
            if self.pages >= self.bounds.max_pages {
                return Err(HistoryError::PageLimit);
            }
            if self.received_entries >= self.bounds.max_entries {
                return Err(HistoryError::EntryLimit);
            }
            self.cursor = Some(next);
        } else {
            // Ausência no delta NÃO distingue job antigo, fork ou ramo errado.
            self.stop(Outcome::NeedsClassification(
                ClassificationReason::BoundaryNotFound,
            ));
        }
        Ok(())
    }

    fn fail(&mut self, error: HistoryError) {
        self.stop(Outcome::Failed(error));
    }

    fn stop(&mut self, outcome: Outcome) {
        self.pending.clear();
        self.seen_entries.clear();
        self.seen_cursors.clear();
        self.cursor = None;
        self.outcome = outcome;
    }
}

/// Validação mínima do envelope solicitado, não um parser de conteúdo/modelo.
/// Campos adicionais e blocos futuros são preservados; texto vazio é válido.
fn parse_entry(item: &Value) -> Result<HistoryEntry, HistoryError> {
    let object = item.as_object().ok_or(HistoryError::MalformedPage)?;
    let id = object
        .get("entry_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(HistoryError::MalformedPage)?;
    let message = object
        .get("message")
        .and_then(Value::as_object)
        .ok_or(HistoryError::MalformedPage)?;
    if !matches!(
        message.get("role").and_then(Value::as_str),
        Some("user" | "assistant")
    ) || !message.get("content").is_some_and(Value::is_array)
        || object.get("custom").is_some_and(|custom| !custom.is_null())
    {
        return Err(HistoryError::MalformedPage);
    }
    Ok(HistoryEntry {
        entry_id: id.to_owned(),
        item: item.clone(),
    })
}

/// Apenas classificação de evidência de root-path fornecida pelo chamador.
/// Não inicia root fetch, não escolhe sessão/banco/branch nem cria payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootClassification {
    Unknown,
    Malformed,
    BoundaryMissing,
    AlreadyCovered,
    DeltaAvailable,
    FallbackRequired,
}

/// `complete` deve significar que o chamador validou TODO o mesmo active path,
/// sua correlação e seus limites, sem erro transitório. Este helper não pode
/// conferir essa autoridade. IDs são comparados somente por igualdade;
/// anterioridade é a posição no path recebido, jamais ordem lexical/temporal.
/// Um root path parcial não autoriza stale/fallback, mesmo contendo a âncora.
/// A chamada pode pedir fallback, mas nenhuma variante o executa ou confirma.
/// O chamador também deve limitar recursos ao produzir `path`.
pub fn classify_root(
    since: Option<&str>,
    until: &str,
    path: &[&str],
    complete: bool,
) -> RootClassification {
    if until.is_empty() || since == Some("") {
        return RootClassification::Malformed;
    }
    let mut ids = HashSet::new();
    if path.iter().any(|id| id.is_empty() || !ids.insert(*id)) {
        return RootClassification::Malformed;
    }
    if !complete {
        return RootClassification::Unknown;
    }
    let Some(end) = path.iter().position(|id| *id == until) else {
        return RootClassification::BoundaryMissing;
    };
    let Some(anchor) = since.and_then(|since| path.iter().position(|id| *id == since)) else {
        return RootClassification::FallbackRequired;
    };
    if end <= anchor {
        RootClassification::AlreadyCovered
    } else {
        RootClassification::DeltaAvailable
    }
}
