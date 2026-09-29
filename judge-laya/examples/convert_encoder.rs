//! Convert a laya checkpoint directory's encoder to GGUF ahead of time (the
//! worker does it on first load): `convert_encoder <checkpoint dir> <tokenizer.json> <out.gguf>`.
fn main() -> anyhow::Result<()> {
    let args: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    let [dir, tokenizer, out] = args.as_slice() else {
        anyhow::bail!("usage: convert_encoder <checkpoint dir> <tokenizer.json> <out.gguf>");
    };
    let started = std::time::Instant::now();
    judge_laya::gguf::convert(
        &dir.join("model.safetensors"),
        &dir.join("encoder/config.json"),
        tokenizer,
        out,
    )?;
    eprintln!("converted in {:.1}s", started.elapsed().as_secs_f32());
    Ok(())
}
