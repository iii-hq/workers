"""Tiny clef GGUF for judge-clef's tests: tiny/model.gguf, random weights at toy size.

Run once from judge-clef/ and commit the output (CI only reads it):
  uv run --no-project --with numpy \
    --with 'gguf @ https://github.com/ggml-org/llama.cpp/archive/1537a0a8b2f8711d840878b0a0677ab2213c882c.tar.gz#subdirectory=gguf-py' \
    python tests/fixtures/make_fixtures.py

The gguf-py is llama.cpp b11379 (1537a0a8), the release judge-clef builds: arch `clef` (backbone
`qwen35`, 3 linear-attention layers then 1 full-attention layer, plus Cloudflare's joint head)
is not in the PyPI gguf. Tensor names and shapes follow src/models/{qwen35,clef}.cpp there, the
keys follow conversion/clef.py. tiny/tokenizer.json (byte-level: token id = byte value, 256 =
"Ġt", specials 257-261) is the input and becomes the GGUF vocabulary; a local checkpoint is
<dir>/{model.gguf, tokenizer.json}. Every tensor is f32 and seeded.
"""
import json, math, pathlib
import numpy as np
import gguf

TINY = pathlib.Path(__file__).parent / "tiny"
EMBD, LAYERS, FF = 32, 4, 64  # backbone; every 4th layer is full attention
HEADS, HEADS_KV, HEAD_DIM, ROT = 2, 1, 16, 8  # full attention, the Q projection also outputs a gate
CONV, STATE, K_HEADS, V_HEADS = 4, 16, 2, 4  # gated delta net
WIDTH, HEAD_HEADS, HEAD_FF, ROUTING, JOINT = 16, 2, 24, 2, 4  # decision head

tok = json.loads((TINY / "tokenizer.json").read_text())
vocab = {i: t for t, i in tok["model"]["vocab"].items()}
types = dict.fromkeys(vocab, gguf.TokenType.NORMAL)
for t in tok["added_tokens"]:
    vocab[t["id"]] = t["content"]
    types[t["id"]] = gguf.TokenType.CONTROL if t["special"] else gguf.TokenType.USER_DEFINED
assert sorted(vocab) == list(range(len(vocab))), "the GGUF vocabulary must be dense"
special = {t: i for i, t in vocab.items() if types[i] != gguf.TokenType.NORMAL}

w = gguf.GGUFWriter(TINY / "model.gguf", "clef")
w.add_name("tiny-clef-test")
w.add_file_type(gguf.LlamaFileType.ALL_F32)
w.add_context_length(262144)
w.add_embedding_length(EMBD)
w.add_block_count(LAYERS)
w.add_feed_forward_length(FF)
w.add_head_count(HEADS)
w.add_head_count_kv(HEADS_KV)
w.add_key_length(HEAD_DIM)
w.add_value_length(HEAD_DIM)
w.add_rope_dimension_count(ROT)
w.add_rope_dimension_sections([2, 1, 1, 0])  # sums to ROT / 2, as [11, 11, 10, 0] in Clef-Flash
w.add_rope_freq_base(1e7)
w.add_layer_norm_rms_eps(1e-6)
w.add_ssm_conv_kernel(CONV)
w.add_ssm_state_size(STATE)
w.add_ssm_group_count(K_HEADS)
w.add_ssm_time_step_rank(V_HEADS)
w.add_ssm_inner_size(STATE * V_HEADS)
w.add_full_attention_interval(4)
w.add_recurrent_layers([(i + 1) % 4 != 0 for i in range(LAYERS)])
w.add_decision_type(gguf.DecisionType.CLEF)
w.add_decision_routing_block_count(ROUTING)
w.add_decision_block_count(JOINT)
w.add_decision_head_count(HEAD_HEADS)
w.add_layer_norm_eps(1e-5)  # the head's LayerNorm
w.add_tokenizer_model("gpt2")
w.add_tokenizer_pre("qwen35")
w.add_token_list([vocab[i] for i in range(len(vocab))])
w.add_token_types([types[i] for i in range(len(vocab))])
w.add_token_merges([" ".join(m) for m in tok["model"]["merges"]])
w.add_eos_token_id(special["<|im_end|>"])
w.add_pad_token_id(special["<|endoftext|>"])
w.add_add_bos_token(False)
w.add_add_eos_token(False)

rng = np.random.default_rng(0)


def put(name, *ne, value=None):
    # ne in ggml order (ne[0] = input width); numpy's shape is the reverse
    if value is None:
        value = rng.standard_normal(ne[::-1]) * ne[0] ** -0.5
    w.add_tensor(name, np.asarray(value, dtype=np.float32).reshape(ne[::-1]))


def norm(name, n, bias=False):
    put(name + ".weight", n, value=1 + 0.1 * rng.standard_normal(n))
    if bias:
        put(name + ".bias", n, value=0.1 * rng.standard_normal(n))


put("token_embd.weight", EMBD, len(vocab))
put("output.weight", EMBD, len(vocab))  # untied: the head reads its rows
norm("output_norm", EMBD)
key, value = STATE * K_HEADS, STATE * V_HEADS
for i in range(LAYERS):
    b = f"blk.{i}"
    norm(b + ".attn_norm", EMBD)
    norm(b + ".post_attention_norm", EMBD)
    if (i + 1) % 4:
        put(b + ".attn_qkv.weight", EMBD, 2 * key + value)
        put(b + ".attn_gate.weight", EMBD, value)
        put(b + ".ssm_conv1d.weight", CONV, 2 * key + value)
        put(b + ".ssm_dt.bias", V_HEADS, value=rng.uniform(0.5, 1.5, V_HEADS))
        put(b + ".ssm_a", V_HEADS, value=-rng.uniform(1, 16, V_HEADS))  # -exp(A_log)
        put(b + ".ssm_beta.weight", EMBD, V_HEADS)
        put(b + ".ssm_alpha.weight", EMBD, V_HEADS)
        norm(b + ".ssm_norm", STATE)
        put(b + ".ssm_out.weight", value, EMBD)
    else:
        put(b + ".attn_q.weight", EMBD, 2 * HEADS * HEAD_DIM)
        put(b + ".attn_k.weight", EMBD, HEADS_KV * HEAD_DIM)
        put(b + ".attn_v.weight", EMBD, HEADS_KV * HEAD_DIM)
        put(b + ".attn_output.weight", HEADS * HEAD_DIM, EMBD)
        norm(b + ".attn_q_norm", HEAD_DIM)
        norm(b + ".attn_k_norm", HEAD_DIM)
    put(b + ".ffn_gate.weight", EMBD, FF)
    put(b + ".ffn_up.weight", EMBD, FF)
    put(b + ".ffn_down.weight", FF, EMBD)


def attn(prefix):
    for x in "qkvo":
        put(f"{prefix}_{x}.weight", WIDTH, WIDTH)
        put(f"{prefix}_{x}.bias", WIDTH, value=0.1 * rng.standard_normal(WIDTH))


# routing blocks (the options read the prompt) come first, then the joint blocks
for i in range(ROUTING + JOINT):
    b = f"dec.blk.{i}"
    if i < ROUTING:
        norm(b + ".cross_attn_norm_kv", WIDTH, bias=True)
    else:
        norm(b + ".attn_norm", WIDTH, bias=True)
        attn(b + ".attn")
    norm(b + ".cross_attn_norm", WIDTH, bias=True)
    attn(b + ".cross_attn")
    norm(b + ".ffn_norm", WIDTH, bias=True)
    put(b + ".ffn_up.weight", WIDTH, HEAD_FF)
    put(b + ".ffn_up.bias", HEAD_FF, value=0.1 * rng.standard_normal(HEAD_FF))
    put(b + ".ffn_down.weight", HEAD_FF, WIDTH)
    put(b + ".ffn_down.bias", WIDTH, value=0.1 * rng.standard_normal(WIDTH))

norm("decision.hidden_norm", EMBD, bias=True)
for n in ("option_summary_norm", "field_norm", "option_norm"):
    norm("decision." + n, WIDTH, bias=True)
for n in ("memory", "question", "option_question", "global", "option_context", "option_lexical"):
    put(f"decision.proj_{n}.weight", EMBD, WIDTH)
# prior scale, joint scale, residual gate: about Clef-Flash's [9.94, 9.94, sigmoid(-2)]
put("decision.scales", 3, value=[10.0, 10.0, 1 / (1 + math.exp(2.0))])
put("token_types.weight", WIDTH, 3)
put("decision.scorer.weight", 4 * WIDTH, WIDTH)
put("decision.scorer.bias", WIDTH, value=0.1 * rng.standard_normal(WIDTH))
put("decision.scorer_out.weight", WIDTH, 1)
put("decision.scorer_out.bias", 1, value=[0.0])

w.write_header_to_file()
w.write_kv_data_to_file()
w.write_tensors_to_file()
w.close()
print("written", TINY / "model.gguf", (TINY / "model.gguf").stat().st_size, "bytes")
