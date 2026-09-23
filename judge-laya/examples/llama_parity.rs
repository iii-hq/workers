//! Spike: laya's encoder through llama.cpp (a GGUF converted from the
//! checkpoint's `encoder.*` tensors) with the decision head in candle, against
//! the all-candle path, on tests/fixtures/encode.json.
//! Usage: llama_parity <gguf> [n_gpu_layers] [threads]
use anyhow::{anyhow, Result};
use candle_core::{Device, Tensor};
use judge_laya::{download, model::LayaModel};
use llama_cpp_2::{
    context::{
        params::{LlamaContextParams, LlamaPoolingType},
        LlamaContext,
    },
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, LlamaModel},
    token::LlamaToken,
};
use serde::Deserialize;
use std::{num::NonZeroU32, time::Instant};

type Row = (Vec<u32>, Vec<usize>, u32);

#[derive(Deserialize)]
struct Case {
    name: String,
    #[serde(rename = "type")]
    qtype: String,
    ids: Vec<u32>,
    markers: Vec<usize>,
}

fn encode(ctx: &mut LlamaContext, rows: &[Row], d: usize) -> Result<Tensor> {
    let s = rows.iter().map(|r| r.0.len()).max().unwrap_or(1);
    let total: usize = rows.iter().map(|r| r.0.len()).sum();
    let mut batch = LlamaBatch::new(total, 1);
    for (seq, (ids, _, _)) in rows.iter().enumerate() {
        for (pos, &id) in ids.iter().enumerate() {
            batch.add(LlamaToken(id as i32), pos as i32, &[seq as i32], true)?;
        }
    }
    ctx.decode(&mut batch)?;
    let mut h = vec![0f32; rows.len() * s * d];
    let mut i = 0i32;
    for (seq, (ids, _, _)) in rows.iter().enumerate() {
        for pos in 0..ids.len() {
            let e = ctx.embeddings_ith(i)?;
            h[(seq * s + pos) * d..(seq * s + pos + 1) * d].copy_from_slice(e);
            i += 1;
        }
    }
    Ok(Tensor::from_vec(h, (rows.len(), s, d), &Device::Cpu)?)
}

fn probabilities(model: &LayaModel, row: &Row, z: &[f32]) -> Vec<f64> {
    let t = model.temperature(row.2, z.len());
    let scaled: Vec<f64> = z.iter().map(|&v| f64::from(v) / t).collect();
    let max = scaled.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exp: Vec<f64> = scaled.iter().map(|v| (v - max).exp()).collect();
    let sum: f64 = exp.iter().sum();
    exp.iter().map(|v| v / sum).collect()
}

fn argmax(p: &[f64]) -> usize {
    p.iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let gguf = args
        .next()
        .ok_or_else(|| anyhow!("usage: llama_parity <gguf> [n_gpu_layers] [threads]"))?;
    let gpu_layers: u32 = args.next().map(|a| a.parse()).transpose()?.unwrap_or(0);
    let threads: i32 = args.next().map(|a| a.parse()).transpose()?.unwrap_or(8);
    let cases: Vec<Case> = serde_json::from_str(include_str!("../tests/fixtures/encode.json"))?;
    let rows: Vec<Row> = cases
        .iter()
        .map(|c| {
            let qtype = ["choice", "score", "noul"]
                .iter()
                .position(|t| *t == c.qtype)
                .expect("known question type") as u32;
            (c.ids.clone(), c.markers.clone(), qtype)
        })
        .collect();

    let checkpoint = download::fetch("laya", None)?;
    let candle = LayaModel::load(&checkpoint, Device::Cpu)?;

    let backend = LlamaBackend::init()?;
    let model = LlamaModel::load_from_file(
        &backend,
        &gguf,
        &LlamaModelParams::default().with_n_gpu_layers(gpu_layers),
    )?;
    let window = 4096u32;
    let mut ctx = model.new_context(
        &backend,
        LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(window))
            .with_n_batch(window)
            .with_n_ubatch(window)
            .with_n_seq_max(rows.len() as u32)
            .with_embeddings(true)
            .with_pooling_type(LlamaPoolingType::None)
            .with_n_threads(threads)
            .with_n_threads_batch(threads),
    )?;
    let d = model.n_embd() as usize;
    eprintln!(
        "gguf={gguf} gpu_layers={gpu_layers} threads={threads} rows={} d={d}",
        rows.len()
    );

    // Parity on the whole fixture batch.
    let (ids, mask, mask_t) = candle.pad(&rows)?;
    let h_candle = candle.encoder_states(&ids, &mask_t)?;
    let h_llama = encode(&mut ctx, &rows, d)?;
    let z_candle = candle.logits_from_states(&h_candle, &mask, &rows)?;
    let z_llama = candle.logits_from_states(&h_llama, &mask, &rows)?;
    let diff = (&h_candle - &h_llama)?.abs()?;
    let s = h_candle.dim(1)?;
    println!(
        "{:<22} {:>9} {:>9} {:>8} {:>6}",
        "case", "max|Δh|", "max|Δp|", "argmax", "len"
    );
    for (i, (case, row)) in cases.iter().zip(&rows).enumerate() {
        let n = row.0.len();
        let valid = diff.get(i)?.narrow(0, 0, n)?;
        let dh = valid.flatten_all()?.max(0)?.to_scalar::<f32>()?;
        // Where the largest gap sits, and how it compares with the state's scale.
        let per_pos = valid.max(1)?.to_vec1::<f32>()?;
        let worst = per_pos
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(p, _)| p);
        let mean_dh = valid.mean_all()?.to_scalar::<f32>()?;
        let mean_h = h_candle
            .get(i)?
            .narrow(0, 0, n)?
            .abs()?
            .mean_all()?
            .to_scalar::<f32>()?;
        eprintln!(
            "  {:<20} mean|Δh| {:.4} mean|h| {:.3} worst position {}/{}",
            case.name, mean_dh, mean_h, worst, n
        );
        let (pc, pl) = (
            probabilities(&candle, row, &z_candle[i]),
            probabilities(&candle, row, &z_llama[i]),
        );
        let dp = pc
            .iter()
            .zip(&pl)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        println!(
            "{:<22} {:>9.4} {:>9.4} {:>8} {:>3}/{:<3}",
            case.name,
            dh,
            dp,
            if argmax(&pc) == argmax(&pl) {
                "same"
            } else {
                "DIFF"
            },
            n,
            s
        );
    }

    // Speed: one row at a time (the production shape), then the whole batch.
    let mut t_candle = Vec::new();
    let mut t_llama = Vec::new();
    for _ in 0..2 {
        candle.logits(&rows[..1])?;
        encode(&mut ctx, &rows[..1], d)?;
    }
    for row in &rows {
        let one = std::slice::from_ref(row);
        let t = Instant::now();
        candle.logits(one)?;
        t_candle.push(t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        let (_, mask, _) = candle.pad(one)?;
        let h = encode(&mut ctx, one, d)?;
        candle.logits_from_states(&h, &mask, one)?;
        t_llama.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let t = Instant::now();
    candle.logits(&rows)?;
    let batch_candle = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let (_, mask, _) = candle.pad(&rows)?;
    let h = encode(&mut ctx, &rows, d)?;
    candle.logits_from_states(&h, &mask, &rows)?;
    let batch_llama = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "per row (median of {}): candle {:.0} ms, llama+head {:.0} ms; batch of {}: candle {:.0} ms, llama+head {:.0} ms",
        rows.len(),
        median(t_candle),
        median(t_llama),
        rows.len(),
        batch_candle,
        batch_llama
    );
    Ok(())
}
