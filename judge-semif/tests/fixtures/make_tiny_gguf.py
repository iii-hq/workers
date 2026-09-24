"""Tiny random qwen3 GGUF with a byte-level vocabulary for judge-semif tests.

Run: uv run --no-project --with gguf --with numpy python tests/fixtures/make_tiny_gguf.py
"""
import numpy as np
from gguf import GGUFWriter, TokenType

def byte_units():
    # GPT-2 byte-to-unicode table: byte-level BPE vocabularies spell bytes this way.
    bs = list(range(ord("!"), ord("~") + 1)) + list(range(ord("¡"), ord("¬") + 1)) + list(range(ord("®"), ord("ÿ") + 1))
    cs = bs[:]
    n = 0
    for b in range(256):
        if b not in bs:
            bs.append(b); cs.append(256 + n); n += 1
    return [chr(c) for _, c in sorted(zip(bs, cs))]

rng = np.random.default_rng(7)
# llama.cpp refuses a BPE vocabulary without merges: add one harmless merge.
tokens = byte_units() + ["Ġt", "<|endoftext|>", "<|im_start|>", "<|im_end|>", "<think>", "</think>"]
types = [TokenType.NORMAL] * 257 + [TokenType.CONTROL, TokenType.CONTROL, TokenType.CONTROL, TokenType.USER_DEFINED, TokenType.USER_DEFINED]
vocab, d, ff, heads, kv, layers, hd = len(tokens), 32, 64, 4, 2, 2, 8

w = GGUFWriter("tests/fixtures/tiny-qwen3.gguf", "qwen3")
w.add_name("tiny-qwen3-test")
w.add_context_length(4096)
w.add_embedding_length(d)
w.add_block_count(layers)
w.add_feed_forward_length(ff)
w.add_head_count(heads)
w.add_head_count_kv(kv)
w.add_key_length(hd)
w.add_value_length(hd)
w.add_rope_freq_base(10000.0)
w.add_layer_norm_rms_eps(1e-6)
w.add_file_type(0)
w.add_tokenizer_model("gpt2")
w.add_tokenizer_pre("qwen2")
w.add_token_list(tokens)
w.add_token_types(types)
w.add_token_merges(["Ġ t"])
w.add_bos_token_id(257)
w.add_eos_token_id(259)
w.add_pad_token_id(257)
w.add_add_bos_token(False)

def t(name, *shape):
    w.add_tensor(name, (rng.standard_normal(shape) * 0.2).astype(np.float32))

t("token_embd.weight", vocab, d)
w.add_tensor("output_norm.weight", np.ones(d, np.float32))
t("output.weight", vocab, d)
for i in range(layers):
    p = f"blk.{i}."
    w.add_tensor(p + "attn_norm.weight", np.ones(d, np.float32))
    t(p + "attn_q.weight", heads * hd, d)
    t(p + "attn_k.weight", kv * hd, d)
    t(p + "attn_v.weight", kv * hd, d)
    t(p + "attn_output.weight", d, heads * hd)
    w.add_tensor(p + "attn_q_norm.weight", np.ones(hd, np.float32))
    w.add_tensor(p + "attn_k_norm.weight", np.ones(hd, np.float32))
    w.add_tensor(p + "ffn_norm.weight", np.ones(d, np.float32))
    t(p + "ffn_gate.weight", ff, d)
    t(p + "ffn_up.weight", ff, d)
    t(p + "ffn_down.weight", d, ff)
w.write_header_to_file(); w.write_kv_data_to_file(); w.write_tensors_to_file(); w.close()
print("written", vocab, "tokens")
