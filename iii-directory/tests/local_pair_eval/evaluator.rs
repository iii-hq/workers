use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const CORPUS: &str = include_str!("corpus.json");
const CORPUS_SHA256: &str = "a58a8e0a1c419e223ad9ab1a0197fb9ca2d23abe1aa952e9036a62e376bb023b";
const LOCK: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));
const MAX_INPUT_BYTES: usize = 128 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const PAIR_LIMIT: usize = 256;
const FIXED_NOW: &str = "2026-06-10T12:00:00Z";

type EvalResult<T> = Result<T, &'static str>;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    fixed_now: String,
    repeat_query_id: String,
    queries: Vec<Query>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    id: String,
    split: String,
    domain: String,
    language: String,
    answerable: bool,
    required_facts: Vec<String>,
    query: String,
    docs: Vec<Doc>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Doc {
    id: String,
    language: String,
    cross_language: bool,
    label: String,
    kind: String,
    supports: Vec<String>,
    text: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 48 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn parse_corpus(bytes: &[u8]) -> EvalResult<Corpus> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("corpus_bytes_limit");
    }
    let corpus: Corpus = serde_json::from_slice(bytes).map_err(|_| "corpus_schema")?;
    validate_corpus(&corpus)?;
    Ok(corpus)
}

fn validate_corpus(c: &Corpus) -> EvalResult<()> {
    if c.schema_version != 1 || c.fixed_now != FIXED_NOW {
        return Err("corpus_version_or_now");
    }
    if c.queries.is_empty() || c.queries.len() > 48 {
        return Err("query_limit");
    }
    let mut ids = BTreeSet::new();
    let mut pairs = BTreeSet::new();
    let mut domains: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for q in &c.queries {
        if !safe_id(&q.id) || !ids.insert(q.id.as_str()) {
            return Err("duplicate_or_invalid_id");
        }
        if !matches!(q.split.as_str(), "dev" | "holdout")
            || (q.split == "dev" && !q.id.starts_with('d'))
            || (q.split == "holdout" && !q.id.starts_with('h'))
            || !safe_id(&q.domain)
            || !matches!(q.language.as_str(), "pt" | "en")
            || q.query.is_empty()
            || q.query.len() > 4096
            || q.docs.len() != 4
        {
            return Err("query_contract");
        }
        domains.entry(&q.split).or_default().insert(&q.domain);
        let required: BTreeSet<_> = q.required_facts.iter().collect();
        if required.is_empty()
            || required.len() != q.required_facts.len()
            || required.iter().any(|id| !safe_id(id))
        {
            return Err("required_facts");
        }
        let mut supported = BTreeSet::new();
        let mut positives = 0;
        let mut negatives = 0;
        let mut content_labels = BTreeMap::new();
        for d in &q.docs {
            if !safe_id(&d.id) || !ids.insert(d.id.as_str()) {
                return Err("duplicate_or_invalid_id");
            }
            if !pairs.insert((q.id.as_str(), d.id.as_str())) {
                return Err("duplicate_pair");
            }
            if !matches!(d.language.as_str(), "pt" | "en")
                || d.cross_language != (d.language != q.language)
                || d.text.is_empty()
                || d.text.len() > 8192
                || !matches!(
                    d.kind.as_str(),
                    "support"
                        | "complement"
                        | "near_topic"
                        | "contradiction"
                        | "wrong_relation"
                        | "wrong_entity"
                        | "wrong_period"
                        | "wrong_attribute"
                        | "wrong_event"
                        | "missing_evidence"
                )
            {
                return Err("document_contract");
            }
            if content_labels
                .insert(digest(d.text.as_bytes()), d.label.as_str())
                .is_some_and(|previous| previous != d.label.as_str())
            {
                return Err("document_label_conflict");
            }
            let facts: BTreeSet<_> = d.supports.iter().collect();
            if facts.len() != d.supports.len() || !facts.is_subset(&required) {
                return Err("unknown_or_duplicate_fact");
            }
            match d.label.as_str() {
                "positive"
                    if !facts.is_empty() && matches!(d.kind.as_str(), "support" | "complement") =>
                {
                    positives += 1;
                    supported.extend(facts);
                }
                "negative"
                    if facts.is_empty() && !matches!(d.kind.as_str(), "support" | "complement") =>
                {
                    negatives += 1
                }
                _ => return Err("label_support_conflict"),
            }
        }
        if negatives == 0
            || (q.answerable && (positives == 0 || supported != required))
            || (!q.answerable && positives != 0)
        {
            return Err("answerability_or_coverage");
        }
    }
    if pairs.len() > 200 || pairs.len() > PAIR_LIMIT {
        return Err("unique_pair_limit");
    }
    let dev = domains.get("dev").ok_or("missing_dev")?;
    let holdout = domains.get("holdout").ok_or("missing_holdout")?;
    if !dev.is_disjoint(holdout) {
        return Err("domain_leakage");
    }
    let repeat = c
        .queries
        .iter()
        .find(|q| q.id == c.repeat_query_id)
        .ok_or("missing_repeat_query")?;
    if repeat.split != "dev" || pairs.len() + repeat.docs.len() > PAIR_LIMIT {
        return Err("repeat_budget");
    }
    Ok(())
}

fn frozen_corpus() -> EvalResult<Corpus> {
    if digest(CORPUS.as_bytes()) != CORPUS_SHA256 {
        return Err("corpus_frozen_hash");
    }
    parse_corpus(CORPUS.as_bytes())
}

fn guard_values(optin: &str, model: &str, offline: &str, cpu_ack: &str) -> EvalResult<PathBuf> {
    if optin != "1" {
        return Err("optin_required");
    }
    if offline != "1" {
        return Err("hf_hub_offline_required");
    }
    if cpu_ack != "600" {
        return Err("coordinator_cpu_limit_ack_required");
    }
    let path = PathBuf::from(model);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("absolute_model_dir_required");
    }
    Ok(path)
}

fn env_value(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

fn preflight() -> EvalResult<PathBuf> {
    let path = guard_values(
        &env_value("III_MEMORY_PAIR_EVAL"),
        &env_value("III_MEMORY_PAIR_MODEL_DIR"),
        &env_value("HF_HUB_OFFLINE"),
        &env_value("III_MEMORY_PAIR_CPU_LIMIT"),
    )?;
    if !LOCK.contains("name = \"fastembed\"\nversion = \"6.0.0\"")
        || !LOCK.contains("name = \"ort\"\nversion = \"2.0.0-rc.13\"")
        || MINILM_INTRA_THREADS != 2
        || RERANKER_MAX_LENGTH != 512
    {
        return Err("library_or_loader_contract");
    }
    let canonical = path.canonicalize().map_err(|_| "model_dir_missing")?;
    if canonical != path || !canonical.is_dir() {
        return Err("canonical_model_dir_required");
    }
    for (name, _, _) in RERANKER_FILES {
        let file = path.join(name);
        if file.canonicalize().map_err(|_| "artifact_missing")? != file {
            return Err("symlink_artifact_rejected");
        }
    }
    verify_artifacts(&path, &RERANKER_FILES).map_err(|_| "artifact_verification_failed")?;
    Ok(path)
}

#[derive(Default)]
struct Budget {
    submitted: usize,
}

impl Budget {
    fn reserve(&mut self, count: usize) -> EvalResult<()> {
        let next = self.submitted.checked_add(count).ok_or("pair_budget")?;
        if count == 0 || next > PAIR_LIMIT {
            return Err("pair_budget");
        }
        self.submitted = next;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
struct Row {
    query_id: String,
    doc_id: String,
    split: String,
    language: String,
    doc_language: String,
    cross_language: bool,
    label: String,
    status: String,
    score: Option<f64>,
    raw_tokens: Option<usize>,
    input_tokens_with_padding: Option<usize>,
    padding_tokens: Option<usize>,
    truncated_tokens: Option<usize>,
}

fn initial_rows(c: &Corpus) -> Vec<Row> {
    c.queries
        .iter()
        .flat_map(|q| {
            q.docs.iter().map(move |d| Row {
                query_id: q.id.clone(),
                doc_id: d.id.clone(),
                split: q.split.clone(),
                language: q.language.clone(),
                doc_language: d.language.clone(),
                cross_language: d.cross_language,
                label: d.label.clone(),
                status: "not_run".into(),
                score: None,
                raw_tokens: None,
                input_tokens_with_padding: None,
                padding_tokens: None,
                truncated_tokens: None,
            })
        })
        .collect()
}

fn restore_order(ids: &[String], results: &[RerankResult]) -> EvalResult<Vec<f64>> {
    let mapped = map_rerank_results(ids, results).map_err(|_| "score_membership_or_finitude")?;
    let by_id: BTreeMap<_, _> = mapped.into_iter().collect();
    ids.iter()
        .map(|id| by_id.get(id).copied().ok_or("missing_score"))
        .collect()
}

fn measure_tokens(model: &TextRerank, q: &Query) -> EvalResult<Vec<(usize, usize, usize, usize)>> {
    let truncation = model
        .tokenizer
        .get_truncation()
        .ok_or("missing_truncation")?;
    if truncation.max_length != 512 || model.tokenizer.get_padding().is_none() {
        return Err("tokenizer_limits");
    }
    let mut raw = model.tokenizer.clone();
    raw.with_padding(None);
    raw.with_truncation(None)
        .map_err(|_| "tokenizer_raw_config")?;
    let inputs: Vec<_> = q
        .docs
        .iter()
        .map(|d| (q.query.as_str(), d.text.as_str()))
        .collect();
    let original = raw
        .encode_batch(inputs.clone(), true)
        .map_err(|_| "token_count_raw")?;
    let actual = model
        .tokenizer
        .encode_batch(inputs, true)
        .map_err(|_| "token_count_batch")?;
    if original.len() != q.docs.len()
        || actual.len() != q.docs.len()
        || actual
            .iter()
            .any(|encoding| encoding.len() != actual[0].len())
    {
        return Err("token_count_membership");
    }
    original
        .iter()
        .zip(actual.iter())
        .map(|(raw, padded)| {
            let content = padded
                .get_attention_mask()
                .iter()
                .filter(|&&m| m != 0)
                .count();
            if padded.len() > 512 || padded.is_empty() || content > raw.len() {
                return Err("token_count_limit");
            }
            Ok((
                raw.len(),
                padded.len(),
                padded.len() - content,
                raw.len() - content,
            ))
        })
        .collect()
}

fn score_query(model: &mut TextRerank, q: &Query, budget: &mut Budget) -> EvalResult<Vec<f64>> {
    let ids: Vec<_> = q.docs.iter().map(|d| d.id.clone()).collect();
    let docs: Vec<_> = q.docs.iter().map(|d| d.text.as_str()).collect();
    budget.reserve(docs.len())?;
    // A reservation counts all submitted pairs even if a batch returns an error.
    let results =
        catch_encoding(|| model.rerank(q.query.as_str(), docs, false, Some(RERANKER_BATCH_SIZE)))
            .map_err(|_| "inference_panicked")?
            .map_err(|_| "inference_failed")?;
    restore_order(&ids, &results)
}

fn ratio(n: usize, d: usize) -> Option<f64> {
    (d != 0).then(|| n as f64 / d as f64)
}

fn query_metrics(q: &Query, scores: &[f64]) -> Value {
    let mut rank: Vec<_> = (0..q.docs.len()).collect();
    rank.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then_with(|| a.cmp(&b)));
    let positive: Vec<_> = (0..q.docs.len())
        .filter(|&i| q.docs[i].label == "positive")
        .collect();
    let negative: Vec<_> = (0..q.docs.len())
        .filter(|&i| q.docs[i].label == "negative")
        .collect();
    let mut failures = Vec::new();
    let mut wins = 0;
    for &p in &positive {
        for &n in &negative {
            if scores[p] > scores[n] {
                wins += 1;
            } else {
                failures.push(json!({"positive_id":q.docs[p].id,"negative_id":q.docs[n].id,
                    "positive_score":scores[p],"negative_score":scores[n],"tie":scores[p] == scores[n]}));
            }
        }
    }
    let top = |k: usize| {
        rank.iter()
            .take(k)
            .filter(|&&i| q.docs[i].label == "positive")
            .count()
    };
    let mut covered = BTreeSet::new();
    for &i in rank.iter().take(2) {
        if q.docs[i].label == "positive" {
            covered.extend(q.docs[i].supports.iter());
        }
    }
    let all_facts = q.answerable && q.required_facts.iter().all(|f| covered.contains(f));
    let comparisons = positive.len() * negative.len();
    json!({"query_id":q.id,"split":q.split,"domain":q.domain,"language":q.language,
        "cross_language":q.docs.iter().any(|d| d.cross_language),"answerable":q.answerable,
        "ranked_doc_ids":rank.iter().map(|&i| &q.docs[i].id).collect::<Vec<_>>(),
        "positive_count":positive.len(),"relative_wins":wins,"relative_comparisons":comparisons,
        "relative_pair_accuracy":ratio(wins,comparisons),"fp_ordering_count":failures.len(),
        "top_positive_hits_at_1":top(1),"top_positive_hits_at_2":top(2),"top_positive_hits_at_4":top(4),
        "top_positive_recall_at_1":ratio(top(1),positive.len()),
        "top_positive_recall_at_2":ratio(top(2),positive.len()),
        "top_positive_recall_at_4":ratio(top(4),positive.len()),
        "mandatory_fact_coverage_at_2":if q.answerable {Some(all_facts)} else {None},
        "required_fact_ids":q.required_facts,"covered_fact_ids_at_2":covered,
        "ranking_gate":if !q.answerable {"not_applicable_no_positive"} else if failures.is_empty() && all_facts {"pass"} else {"fail"},
        "no_answer_status":if q.answerable {"not_applicable"} else {"uncalibrated_no_abstention_claim"},
        "failures":failures})
}

fn aggregate(metrics: &[Value]) -> Vec<Value> {
    let mut groups: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for m in metrics {
        let split = m["split"].as_str().unwrap();
        let language = m["language"].as_str().unwrap();
        let domain = m["domain"].as_str().unwrap();
        let cross = m["cross_language"].as_bool().unwrap();
        for key in [
            split.to_string(),
            format!("{split}/language/{language}"),
            format!("{split}/cross_language/{cross}"),
            format!("{split}/domain/{domain}"),
        ] {
            groups.entry(key).or_default().push(m);
        }
    }
    groups.into_iter().map(|(key, rows)| {
        let sum = |field: &str| rows.iter().map(|m| m[field].as_u64().unwrap_or(0) as usize).sum::<usize>();
        let positives = sum("positive_count");
        let answerable = rows.iter().filter(|m| m["answerable"] == true).count();
        let covered = rows.iter().filter(|m| m["mandatory_fact_coverage_at_2"] == true).count();
        json!({"group":key,"query_count":rows.len(),"answerable_queries":answerable,
            "no_answer_queries":rows.len()-answerable,"relative_wins":sum("relative_wins"),
            "relative_comparisons":sum("relative_comparisons"),
            "relative_pair_accuracy":ratio(sum("relative_wins"),sum("relative_comparisons")),
            "fp_ordering_count":sum("fp_ordering_count"),
            "top_positive_recall_at_1":ratio(sum("top_positive_hits_at_1"),positives),
            "top_positive_recall_at_2":ratio(sum("top_positive_hits_at_2"),positives),
            "top_positive_recall_at_4":ratio(sum("top_positive_hits_at_4"),positives),
            "all_mandatory_facts_coverage_at_2":ratio(covered,answerable),
            "failure_query_ids":rows.iter().filter(|m| m["ranking_gate"] == "fail").map(|m| &m["query_id"]).collect::<Vec<_>>()})
    }).collect()
}

fn emit(report: &Value) {
    let bytes = serde_json::to_string(report).expect("JSON serialization failed");
    assert!(bytes.len() <= MAX_OUTPUT_BYTES, "output_bytes_limit");
    println!("\n{bytes}");
}

#[test]
#[ignore = "offline optin: III_MEMORY_PAIR_EVAL=1, absolute model dir, HF_HUB_OFFLINE=1, coordinator CPU cap"]
fn offline_local_pair_benchmark() {
    let total = Instant::now();
    let c = match frozen_corpus() {
        Ok(c) => c,
        Err(code) => {
            emit(&json!({"schema_version":1,"outcome":"blocked","code":code,"submitted_pairs":0}));
            panic!("BLOCKED: corpus validation");
        }
    };
    let mut rows = initial_rows(&c);
    let mut budget = Budget::default();
    let mut query_results = Vec::new();
    let mut batches = Vec::new();
    let mut init_ms = None;
    let mut verification_ms = None;
    let mut runtime_info = None;
    let mut tokenizer_config = None;
    let repeat_query = c
        .queries
        .iter()
        .find(|q| q.id == c.repeat_query_id)
        .unwrap();
    let mut repetition: Vec<Value> = repeat_query
        .docs
        .iter()
        .map(|d| {
            json!({
                "query_id":repeat_query.id,"doc_id":d.id,"status":"not_run","score":null
            })
        })
        .collect();
    let mut deterministic = None;
    let mut completed_calls = 0;
    let measured = catch_encoding(|| -> EvalResult<()> {
        let verify_start = Instant::now();
        let model_path = preflight()?;
        verification_ms = Some(verify_start.elapsed().as_secs_f64() * 1000.0);
        let init_start = Instant::now();
        // This existing reranker loader has no embed or scoring probe.
        let mut model = load_verified_reranker(&model_path).map_err(|_| "loader_failed")?;
        init_ms = Some(init_start.elapsed().as_secs_f64() * 1000.0);
        runtime_info = Some(ort::info().to_owned());
        tokenizer_config = Some(
            json!({"padding":format!("{:?}", model.tokenizer.get_padding()),
            "truncation":format!("{:?}", model.tokenizer.get_truncation())}),
        );
        let mut offset = 0;
        for q in &c.queries {
            let token_counts = measure_tokens(&model, q)?;
            let start = Instant::now();
            let scores = score_query(&mut model, q, &mut budget)?;
            let batch_ms = start.elapsed().as_secs_f64() * 1000.0;
            completed_calls += 1;
            batches.push(json!({"query_id":q.id,"pairs":scores.len(),"batch_ms":batch_ms}));
            for (i, score) in scores.iter().enumerate() {
                let row = &mut rows[offset + i];
                if row.query_id != q.id || row.doc_id != q.docs[i].id {
                    return Err("input_order_mismatch");
                }
                row.score = Some(*score);
                row.status = "scored".into();
                let (raw, padded, padding, truncated) = token_counts[i];
                row.raw_tokens = Some(raw);
                row.input_tokens_with_padding = Some(padded);
                row.padding_tokens = Some(padding);
                row.truncated_tokens = Some(truncated);
            }
            offset += scores.len();
            query_results.push(query_metrics(q, &scores));
        }
        let repeat = c
            .queries
            .iter()
            .find(|q| q.id == c.repeat_query_id)
            .ok_or("repeat_missing")?;
        let start = Instant::now();
        let scores = score_query(&mut model, repeat, &mut budget)?;
        completed_calls += 1;
        batches.push(
            json!({"query_id":repeat.id,"repeat":true,"pairs":scores.len(),
            "batch_ms":start.elapsed().as_secs_f64()*1000.0}),
        );
        let original: Vec<_> = rows.iter().filter(|r| r.query_id == repeat.id).collect();
        let exact = original
            .iter()
            .zip(&scores)
            .all(|(r, s)| r.score.map(f64::to_bits) == Some(s.to_bits()));
        for (row, score) in repetition.iter_mut().zip(&scores) {
            row["status"] = json!("scored");
            row["score"] = json!(score);
        }
        deterministic = Some(exact);
        if !exact {
            return Err("repeat_score_bits_changed");
        }
        Ok(())
    })
    .unwrap_or(Err("evaluation_panicked"));
    if let Err(code) = measured {
        for row in rows.iter_mut().filter(|r| r.score.is_none()) {
            row.status = format!("blocked:{code}");
        }
        for row in repetition.iter_mut().filter(|r| r["score"].is_null()) {
            row["status"] = json!(format!("blocked:{code}"));
        }
    }
    let quality_failures: Vec<_> = query_results
        .iter()
        .filter(|m| m["ranking_gate"] == "fail")
        .map(|m| m["query_id"].clone())
        .collect();
    let technical_ok = measured.is_ok() && rows.iter().all(|r| r.score.is_some_and(f64::is_finite));
    let ranking_ok = technical_ok && quality_failures.is_empty();
    let input_order: Vec<_> = rows.iter().map(|r| (&r.query_id, &r.doc_id)).collect();
    let order_bytes = serde_json::to_vec(&input_order).unwrap();
    let report = json!({
        "schema_version":1,
        "correlation":{"run_id":"memory-quality-523100c4-r1","phase":"local-pair-benchmark-preparation","attempt":1},
        "outcome":if !technical_ok {"blocked"} else if ranking_ok {"ranking_pass_abstention_unmeasured"} else {"quality_failure"},
        "error_code":measured.err(),"quality_failure_query_ids":quality_failures,
        "corpus_sha256":CORPUS_SHA256,"fixed_now":c.fixed_now,"input_order_sha256":digest(&order_bytes),
        "counts":{"queries":c.queries.len(),"unique_pairs":rows.len(),"planned_scoring_pairs_including_repeat":rows.len()+repeat_query.docs.len(),
            "submitted_pair_upper_bound":budget.submitted,
            "actual_scoring_pairs_on_complete_success":if measured.is_ok() {Some(budget.submitted)} else {None},
            "successful_scoring_calls":completed_calls,
            "returned_finite_pair_scores":rows.iter().filter(|r| r.score.is_some()).count()
                +repetition.iter().filter(|r| r["score"].is_number()).count()},
        "limits":{"unique_pairs":200,"all_scoring_pairs":PAIR_LIMIT,"cpu_seconds":600,
            "cpu_cap_enforcement":"external coordinator ulimit -t 600; acknowledgement is not proof",
            "cpu_user_seconds":null,"cpu_system_seconds":null,"cpu_evidence":"external /usr/bin/time -l",
            "wall_watchdog_seconds":1200,"intra_threads":MINILM_INTRA_THREADS,"max_pair_tokens":RERANKER_MAX_LENGTH,
            "batch_size":RERANKER_BATCH_SIZE,"max_input_bytes":MAX_INPUT_BYTES,"max_output_bytes":MAX_OUTPUT_BYTES},
        "safety":{"offline_optin_required":true,"cache_access_policy":"read_only",
            "network_policy":"forbidden; no network call path in evaluator or local loader",
            "observed_network_calls":null,"api_cost":null,
            "api_cost_reason":"no API used","runtime_output_files":[],"temporary_files":[],
            "initial_scoring_probe_pairs":0,"no_engine":true,"threshold_sweeps":0},
        "model":{"repository":RERANKER_REPOSITORY,"revision":RERANKER_REVISION,
            "artifacts":RERANKER_FILES.iter().map(|(name,size,hash)| json!({"name":name,"bytes":size,"sha256":hash})).collect::<Vec<_>>(),
            "verification_status":if verification_ms.is_some() {"verified_before_loader"} else {"not_verified"}},
        "libraries":{"fastembed":"6.0.0","ort":"2.0.0-rc.13","onnx_api":ort::MINOR_VERSION,
            "runtime_build_info":runtime_info,"cargo_lock_sha256":digest(LOCK.as_bytes()),
            "target":env!("TARGET"),"worker_version":env!("CARGO_PKG_VERSION")},
        "tokenizer":tokenizer_config,
        "timing":{"verification_ms":verification_ms,"initialization_ms_including_loader_reverification":init_ms,
            "batches":batches,"total_clock_ms":total.elapsed().as_secs_f64()*1000.0,
            "total_clock_scope":"before JSON serialization; full process CPU/wall evidence supplied by coordinator"},
        "repeatability":{"query_id":c.repeat_query_id,"extra_scoring_pairs":repeat_query.docs.len(),"exact_score_bits_equal":deterministic,
            "scope":"one dev query repeated once; no full-corpus determinism claim","results":repetition},
        "policy":{"higher_logit_is_better":true,"ties_fail_relative_order":true,"top_k":[1,2,4],
            "mandatory_facts_gate_k":2,"ranking_gate":"all positive-negative orders strict and all mandatory facts covered at k=2",
            "no_answer":"no positives; ranking and abstention pass not defined without calibration",
            "calendar":"explicit fixed-time intervals only; no real-current-calendar claim",
            "interpretation":"ranking logits, not probabilities or guaranteed entailment; no integration authorization"},
        "aggregates":aggregate(&query_results),"queries":query_results,"results":rows
    });
    emit(&report);
    assert!(
        technical_ok,
        "BLOCKED: technical evidence incomplete; inspect JSON error_code"
    );
    assert!(
        ranking_ok,
        "QUALITY_FAILURE: inspect JSON failure IDs; do not suppress exit status"
    );
}

#[test]
fn pair_eval_corpus_is_frozen_and_valid() {
    let c = frozen_corpus().unwrap();
    assert_eq!(c.queries.len(), 24);
    let rows = initial_rows(&c);
    assert_eq!(rows.len(), 96);
    assert_eq!(rows.iter().filter(|r| r.label == "positive").count(), 24);
    assert_eq!(rows.iter().filter(|r| r.cross_language).count(), 20);
    for split in ["dev", "holdout"] {
        assert_eq!(c.queries.iter().filter(|q| q.split == split).count(), 12);
        assert_eq!(
            c.queries
                .iter()
                .filter(|q| q.split == split && !q.answerable)
                .count(),
            2
        );
    }
    assert!(LOCK.contains("name = \"fastembed\"\nversion = \"6.0.0\""));
    assert!(LOCK.contains("name = \"ort\"\nversion = \"2.0.0-rc.13\""));
}

#[test]
fn pair_eval_json_and_token_input_are_bounded_without_temp_files() {
    let c = frozen_corpus().unwrap();
    assert!(CORPUS.len() <= MAX_INPUT_BYTES);
    let mut rows = initial_rows(&c);
    for row in &mut rows {
        row.status = "scored".into();
        row.score = Some(-123.123456789);
        row.raw_tokens = Some(8192);
        row.input_tokens_with_padding = Some(512);
        row.padding_tokens = Some(512);
        row.truncated_tokens = Some(8192);
    }
    let metrics: Vec<_> = c
        .queries
        .iter()
        .map(|q| {
            let scores: Vec<_> = q
                .docs
                .iter()
                .map(|d| if d.label == "positive" { -1.0 } else { 1.0 })
                .collect();
            query_metrics(q, &scores)
        })
        .collect();
    let encoded = serde_json::to_vec(&json!({"results":rows,"queries":metrics,
        "aggregates":aggregate(&metrics)}))
    .unwrap();
    assert!(encoded.len() + 32768 < MAX_OUTPUT_BYTES);
    let decoded: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded["results"].as_array().unwrap().len(), 96);
}

#[test]
fn pair_eval_guards_fail_closed_without_model_access() {
    assert_eq!(guard_values("", "/abs", "1", "600"), Err("optin_required"));
    assert_eq!(
        guard_values("1", "/abs", "", "600"),
        Err("hf_hub_offline_required")
    );
    assert_eq!(
        guard_values("1", "/abs", "1", ""),
        Err("coordinator_cpu_limit_ack_required")
    );
    for invalid in ["", "relative", "/abs/../escape"] {
        assert_eq!(
            guard_values("1", invalid, "1", "600"),
            Err("absolute_model_dir_required")
        );
    }
    assert!(guard_values("1", "/abs", "1", "600").is_ok());
}

#[test]
fn pair_eval_schema_and_labels_reject_invalid_input() {
    assert_eq!(
        parse_corpus(&vec![b' '; MAX_INPUT_BYTES + 1]).unwrap_err(),
        "corpus_bytes_limit"
    );
    let mut unknown: Value = serde_json::from_str(CORPUS).unwrap();
    unknown["extra"] = json!(true);
    assert_eq!(
        parse_corpus(&serde_json::to_vec(&unknown).unwrap()).unwrap_err(),
        "corpus_schema"
    );
    let c = frozen_corpus().unwrap();
    let mut bad = c.clone();
    bad.queries[0].docs[1].id = bad.queries[0].docs[0].id.clone();
    assert_eq!(validate_corpus(&bad), Err("duplicate_or_invalid_id"));
    let mut bad = c.clone();
    bad.queries[0].docs[1].label = "negative".into();
    assert_eq!(validate_corpus(&bad), Err("label_support_conflict"));
    let mut bad = c.clone();
    bad.queries[0].docs[1].text = bad.queries[0].docs[0].text.clone();
    assert_eq!(validate_corpus(&bad), Err("document_label_conflict"));
    let mut bad = c.clone();
    bad.queries[0].docs.pop();
    assert_eq!(validate_corpus(&bad), Err("query_contract"));
    let mut bad = c.clone();
    bad.queries[0].required_facts.push("uncovered".into());
    assert_eq!(validate_corpus(&bad), Err("answerability_or_coverage"));
    let mut bad = c.clone();
    bad.queries[0].docs[0].cross_language = true;
    assert_eq!(validate_corpus(&bad), Err("document_contract"));
    let mut bad = c;
    bad.queries[12].domain = bad.queries[0].domain.clone();
    assert_eq!(validate_corpus(&bad), Err("domain_leakage"));
}

#[test]
fn pair_eval_counts_repetitions_and_rejects_over_budget() {
    let mut budget = Budget::default();
    for _ in 0..25 {
        budget.reserve(4).unwrap();
    }
    assert_eq!(budget.submitted, 100);
    budget.reserve(156).unwrap();
    assert_eq!(budget.reserve(1), Err("pair_budget"));
    assert_eq!(budget.submitted, 256);
    assert_eq!(budget.reserve(usize::MAX), Err("pair_budget"));
}

#[test]
fn pair_eval_output_mapping_proves_membership_order_and_finitude() {
    let ids = vec!["a".into(), "b".into()];
    let r = |index, score| RerankResult {
        index,
        score,
        document: None,
    };
    assert_eq!(
        restore_order(&ids, &[r(1, 9.0), r(0, -3.0)]).unwrap(),
        vec![-3.0, 9.0]
    );
    for invalid in [
        vec![r(0, 1.0)],
        vec![r(0, 1.0), r(0, 2.0)],
        vec![r(0, 1.0), r(2, 2.0)],
        vec![r(0, f32::NAN), r(1, 2.0)],
        vec![r(0, 1.0), r(1, f32::INFINITY)],
    ] {
        assert_eq!(
            restore_order(&ids, &invalid),
            Err("score_membership_or_finitude")
        );
    }
}

#[test]
fn pair_eval_metrics_require_all_complementary_facts_and_keep_no_answer_uncalibrated() {
    let c = frozen_corpus().unwrap();
    let q = c.queries.iter().find(|q| q.id == "d08").unwrap();
    let passed = query_metrics(q, &[0.0, 4.0, -1.0, 3.0]);
    assert_eq!(passed["ranking_gate"], "pass");
    assert_eq!(passed["top_positive_recall_at_1"], 0.5);
    assert_eq!(passed["mandatory_fact_coverage_at_2"], true);
    let failed = query_metrics(q, &[3.0, 4.0, -1.0, 0.0]);
    assert_eq!(failed["ranking_gate"], "fail");
    assert_eq!(failed["mandatory_fact_coverage_at_2"], false);
    let tied = query_metrics(q, &[3.0, 4.0, -1.0, 3.0]);
    assert_eq!(tied["ranking_gate"], "fail");
    let no_answer = query_metrics(&c.queries[10], &[100.0, 99.0, 98.0, 97.0]);
    assert_eq!(
        no_answer["no_answer_status"],
        "uncalibrated_no_abstention_claim"
    );
    assert!(no_answer["relative_pair_accuracy"].is_null());
    assert!(no_answer["mandatory_fact_coverage_at_2"].is_null());
    let grouped = aggregate(&[passed, failed, no_answer]);
    assert!(grouped
        .iter()
        .any(|g| g["group"] == "dev" && g["fp_ordering_count"] == 1));
}
