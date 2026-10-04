//! The context API on the CPU over two tiny random GGUFs of the other judge
//! providers: judge-decider's qwen3 (causal, byte vocabulary, logits) and
//! judge-laya's modern-bert encoder with its decision head (non-causal,
//! embeddings).
use iii_llama_native::{Batch, Context, ContextParams, Model, Pooling};
use std::path::Path;

fn load(fixture: &str) -> Model {
    let gguf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(fixture);
    Model::load(&gguf, Some(0), 2, false).expect("tiny GGUF loads")
}

fn qwen3() -> Model {
    load("judge-decider/tests/fixtures/tiny-qwen3.gguf")
}

fn encoder() -> Model {
    load("judge-laya/tests/fixtures/tiny.reference.gguf")
}

/// Two sequences sharing one KV pool, as crates/llama-runtime's scorer runs.
fn scorer_context(model: &Model) -> Context<'_> {
    model
        .new_context(ContextParams {
            n_ctx: 256,
            n_batch: 64,
            n_ubatch: 64,
            n_seq_max: 2,
            threads: 2,
            kv_unified: true,
            ..Default::default()
        })
        .unwrap()
}

/// The largest difference between two rows.
fn gap(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0., f32::max)
}

/// Decode `spans` (tokens, first position, sequence) in one batch; the logits
/// at each span's last token.
fn decode(ctx: &mut Context<'_>, spans: &[(&[i32], i32, i32)]) -> anyhow::Result<Vec<Vec<f32>>> {
    let mut batch = Batch::default();
    let mut last = Vec::new();
    let mut row = 0;
    for &(ids, start, seq) in spans {
        for (i, &id) in ids.iter().enumerate() {
            batch.add(id, start + i as i32, seq, i + 1 == ids.len());
            row += 1;
        }
        last.push(row - 1);
    }
    ctx.decode(&batch)?;
    Ok(last
        .into_iter()
        .map(|row| ctx.logits_ith(row).expect("an output row").to_vec())
        .collect())
}

#[test]
fn tokenizes_like_str_to_token() {
    let model = qwen3();
    assert_eq!(model.architecture().as_deref(), Some("qwen3"));
    assert_eq!(
        model.meta("general.name").as_deref(),
        Some("tiny-qwen3-test")
    );
    assert_eq!(model.meta("no.such.key"), None);
    // Bytes are their own ids, " t" is the one merge, special tokens parse.
    assert_eq!(
        model.tokenize("<|im_start|>A t", false).unwrap(),
        [258, 65, 256]
    );
    // More tokens than the first guess (half the bytes) takes a second call.
    assert_eq!(model.tokenize(&"A".repeat(100), false).unwrap(), [65; 100]);
    assert!(model.tokenize("a\0b", false).is_err());
    // qwen3's vocabulary adds no BOS; the encoder's adds its own around.
    assert_eq!(model.tokenize("A", true).unwrap(), [65]);
    let encoder = encoder();
    let plain = encoder.tokenize("the site is down", false).unwrap();
    let special = encoder.tokenize("the site is down", true).unwrap();
    assert!(special.len() > plain.len(), "{special:?} vs {plain:?}");
    assert!(special.windows(plain.len()).any(|w| w == plain));
}

#[test]
fn decodes_sequences_in_one_batch_deterministically() {
    let model = qwen3();
    let mut ctx = scorer_context(&model);
    assert!(ctx.n_ctx() >= 256);
    assert_eq!(ctx.n_batch(), 64);
    let a = model.tokenize("STATE: the site is down.", false).unwrap();
    let b = model.tokenize("OPTION: billing", false).unwrap();
    let both = decode(&mut ctx, &[(&a, 0, 0), (&b, 0, 1)]).unwrap();
    assert_eq!(both[0].len(), model.n_vocab());
    assert!(both.iter().flatten().all(|v| v.is_finite()));
    assert_ne!(both[0], both[1]);
    // Only the rows asked for are kept.
    assert!(ctx.logits_ith(0).is_none());
    ctx.clear_kv_cache();
    assert_eq!(decode(&mut ctx, &[(&a, 0, 0), (&b, 0, 1)]).unwrap(), both);
    // A sequence does not see the other one in its batch.
    ctx.clear_kv_cache();
    let alone = decode(&mut ctx, &[(&b, 0, 0)]).unwrap();
    assert!(gap(&alone[0], &both[1]) < 1e-4);
}

#[test]
fn restores_a_sequence_state_into_another_sequence() {
    let model = qwen3();
    let mut ctx = scorer_context(&model);
    let prefix = model.tokenize("STATE: the site is down.", false).unwrap();
    let suffix = model.tokenize("\nQUESTION: which team?", false).unwrap();
    let at = prefix.len() as i32;
    decode(&mut ctx, &[(&prefix, 0, 0)]).unwrap();
    let state = ctx.state_seq_get(0).unwrap();
    let whole = decode(&mut ctx, &[(&suffix, at, 0)]).unwrap();
    // Restored into another sequence, the same decode gives the same logits.
    ctx.clear_kv_cache();
    ctx.state_seq_set(&state, 1).unwrap();
    assert_eq!(decode(&mut ctx, &[(&suffix, at, 1)]).unwrap(), whole);
    // The scorer's move: restore into each sequence, decode the suffixes
    // together (a wider batch: equal up to rounding).
    ctx.clear_kv_cache();
    ctx.state_seq_set(&state, 0).unwrap();
    ctx.state_seq_set(&state, 1).unwrap();
    for logits in decode(&mut ctx, &[(&suffix, at, 0), (&suffix, at, 1)]).unwrap() {
        assert!(gap(&logits, &whole[0]) < 1e-4);
    }
    // Sequences outside the context are refused (llama.cpp would abort).
    assert!(ctx.state_seq_set(&state, 2).is_err());
    assert!(ctx.state_seq_get(-1).is_err());
}

#[test]
fn clear_empties_memory_and_bad_batches_err() {
    let model = qwen3();
    let mut ctx = scorer_context(&model);
    let ids = model.tokenize("the site is down", false).unwrap();
    decode(&mut ctx, &[(&ids, 0, 0)]).unwrap();
    // A sequence continues where it stopped: position 0 again is refused...
    assert!(decode(&mut ctx, &[(&ids, 0, 0)]).is_err());
    // ...until its memory is cleared.
    ctx.clear_kv_cache();
    decode(&mut ctx, &[(&ids, 0, 0)]).unwrap();

    ctx.clear_kv_cache();
    let one = |token, pos, seq| {
        let mut batch = Batch::default();
        batch.add(token, pos, seq, true);
        batch
    };
    assert!(ctx.decode(&Batch::default()).is_err());
    assert!(
        ctx.decode(&one(65, 0, 2)).is_err(),
        "sequence past n_seq_max"
    );
    assert!(ctx.decode(&one(65, -1, 0)).is_err(), "negative position");
    let vocab = model.n_vocab() as i32;
    assert!(
        ctx.decode(&one(vocab, 0, 0)).is_err(),
        "token past the vocabulary"
    );
    let mut long = Batch::default();
    for pos in 0..65 {
        long.add(65, pos, 0, false);
    }
    assert!(ctx.decode(&long).is_err(), "more than n_batch");
    // An encode is one micro-batch: the same 65 tokens past n_ubatch.
    assert!(ctx.encode(&long).is_err());
    assert!(
        ctx.encode(&one(65, 0, 0)).is_err(),
        "an encode needs a context without memory"
    );
    // The context is still usable.
    decode(&mut ctx, &[(&ids, 0, 0)]).unwrap();
    assert!(model
        .new_context(ContextParams {
            threads: 0,
            ..Default::default()
        })
        .is_err());
    assert!(model
        .new_context(ContextParams {
            n_ctx: 32,
            n_seq_max: 64,
            ..Default::default()
        })
        .is_err());
}

#[test]
fn encodes_rows_to_token_embeddings() {
    let model = encoder();
    assert_eq!(model.architecture().as_deref(), Some("modern-bert"));
    // Rows are laya's decision head: one score per question type.
    let width = model.n_embd_out();
    assert_eq!(width, 3);
    let mut ctx = model
        .new_context(ContextParams {
            n_ctx: 64,
            n_batch: 64,
            n_ubatch: 64,
            n_seq_max: 2,
            threads: 2,
            embeddings: true,
            pooling: Pooling::None,
            ..Default::default()
        })
        .unwrap();
    let a = model.tokenize("the site is down", true).unwrap();
    let b = model.tokenize("billing", true).unwrap();
    let rows = |ctx: &Context<'_>, n: usize| -> Vec<Vec<f32>> {
        (0..n as i32)
            .map(|i| {
                ctx.embeddings_ith(i)
                    .expect("every row is an output")
                    .to_vec()
            })
            .collect()
    };
    let mut batch = Batch::default();
    for (seq, ids) in [&a, &b].into_iter().enumerate() {
        for (pos, &id) in ids.iter().enumerate() {
            batch.add(id, pos as i32, seq as i32, true);
        }
    }
    ctx.encode(&batch).unwrap();
    let encoded = rows(&ctx, a.len() + b.len());
    assert!(encoded.iter().all(|row| row.len() == width));
    assert!(encoded.iter().flatten().all(|v| v.is_finite()));
    // An embeddings context keeps no logits.
    assert!(ctx.logits_ith(0).is_none());
    // Without memory, a decode is the same encode.
    ctx.decode(&batch).unwrap();
    assert_eq!(rows(&ctx, a.len() + b.len()), encoded);
    // A row's states do not depend on the other row in the batch.
    let mut alone = Batch::default();
    for (pos, &id) in b.iter().enumerate() {
        alone.add(id, pos as i32, 0, true);
    }
    ctx.encode(&alone).unwrap();
    let together: Vec<f32> = encoded[a.len()..].concat();
    assert!(gap(&rows(&ctx, b.len()).concat(), &together) < 1e-4);
    // Past n_ubatch an encode is refused (llama.cpp would abort).
    let mut long = Batch::default();
    for pos in 0..65 {
        long.add(a[0], pos, 0, true);
    }
    assert!(ctx.encode(&long).is_err());
    assert!(ctx.decode(&long).is_err());
}

#[test]
fn zero_gpu_layers_can_keep_the_gpu_devices() {
    let gguf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../judge-decider/tests/fixtures/tiny-qwen3.gguf");
    let model = Model::load(&gguf, Some(0), 2, true).unwrap();
    assert_eq!(model.device, "CPU");
    let ids = model.tokenize("the site is down", false).unwrap();
    let mut ctx = scorer_context(&model);
    assert!(decode(&mut ctx, &[(&ids, 0, 0)]).unwrap()[0]
        .iter()
        .all(|v| v.is_finite()));
}
