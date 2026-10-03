"""Tiny joint head, tokenizer and head parity cases for judge-clef's tests.

Run once from judge-clef/ and commit the outputs (CI only reads them):
  uv run --no-project --index https://download.pytorch.org/whl/cpu --index-strategy unsafe-best-match \
    --with torch==2.11.0 --with safetensors --with numpy --with 'tokenizers==0.22.*' --with gguf \
    python tests/fixtures/make_fixtures.py

tiny/backbone.gguf (a copy of judge-decider's tiny qwen3) is the input: its vocabulary becomes
tiny/tokenizer.json and its output.weight the lexical rows. The head is Cloudflare's own
JointSchemaHead from a pinned revision, built at toy size with seeded weights rounded to bf16 (the
release dtype) and run in float64, so the error left is judge-clef's f32.
"""
import json, math, pathlib, sys, urllib.request
import numpy as np
import torch
from gguf import GGUFReader
from safetensors.torch import save_file
from tokenizers import AddedToken, Regex, Tokenizer, decoders, models, normalizers, pre_tokenizers

URL = "https://huggingface.co/Cloudflare/clef-flash/raw/17f0b0ad64efb65d273590632833508766b2aae6/joint_schema_model.py"
OUT = pathlib.Path(__file__).parent
TINY = OUT / "tiny"
CONFIG = {"hidden_size": 32, "width": 16, "routing_layers": 2, "layers": 4, "heads": 2, "feedforward": 24}
# Qwen2's pre-tokenizer split, as Cloudflare/clef-flash's tokenizer.json.
QWEN2 = r"""(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\r\n\p{L}\p{N}]?\p{L}+|\p{N}| ?[^\s\p{L}\p{N}]+[\r\n]*|\s*[\r\n]+|\s+(?!\S)|\s+"""

gguf = GGUFReader(TINY / "backbone.gguf")
field = gguf.fields
tokens = [bytes(field["tokenizer.ggml.tokens"].parts[i]).decode() for i in field["tokenizer.ggml.tokens"].data]
types = [int(field["tokenizer.ggml.token_type"].parts[i][0]) for i in field["tokenizer.ggml.token_type"].data]
merges = [tuple(bytes(field["tokenizer.ggml.merges"].parts[i]).decode().split(" ")) for i in field["tokenizer.ggml.merges"].data]
lm_head = torch.from_numpy(np.array(next(t for t in gguf.tensors if t.name == "output.weight").data)).double()
assert lm_head.shape == (len(tokens), CONFIG["hidden_size"]), lm_head.shape

# Normal tokens (1) are the BPE vocabulary; control (3) and user-defined (4) tokens are added in id order.
tok = Tokenizer(models.BPE(vocab={t: i for i, (t, k) in enumerate(zip(tokens, types)) if k == 1}, merges=merges))
tok.normalizer = normalizers.NFC()
tok.pre_tokenizer = pre_tokenizers.Sequence([pre_tokenizers.Split(Regex(QWEN2), behavior="isolated"),
                                             pre_tokenizers.ByteLevel(add_prefix_space=False, use_regex=False)])
tok.decoder = decoders.ByteLevel()
for t, k in zip(tokens, types):
    if k != 1:
        tok.add_tokens([AddedToken(t, normalized=False, special=k == 3)])
assert all(tok.token_to_id(t) == i for i, t in enumerate(tokens))
assert tok.encode(" t<|im_start|>é<think>", add_special_tokens=False).ids == [256, 258, 195, 169, 260]
tok.save(str(TINY / "tokenizer.json"))

jsm = type(sys)("jsm")
sys.modules["jsm"] = jsm  # dataclasses look the module up
exec(compile(urllib.request.urlopen(URL).read(), URL, "exec"), jsm.__dict__)

torch.manual_seed(0)
head = jsm.JointSchemaHead(**CONFIG).eval()
with torch.no_grad():
    for p in head.parameters():
        p.copy_(torch.randn_like(p) * (p.shape[-1] ** -0.5 if p.dim() == 2 else 0.2))
    for m in head.modules():
        if isinstance(m, torch.nn.LayerNorm):
            m.weight.add_(1.0)
    head.prior_logit_scale.fill_(math.log(10.0))  # the release value
    head.joint_logit_scale.fill_(5.0)  # above ln 100: exercises the clamp
    head.residual_gate.fill_(-2.0)  # the release value
    for p in head.parameters():
        p.copy_(p.bfloat16().float())
save_file({k: v.bfloat16().contiguous() for k, v in head.state_dict().items()}, TINY / "joint_head.safetensors")
(TINY / "joint_head_config.json").write_text(json.dumps(CONFIG, indent=2) + "\n")
head = head.double()


def case(name, length, fields, ids=None):
    # Multiples of 1/16 print short and read back exactly as f32 and f64.
    hidden = (torch.randn(length, CONFIG["hidden_size"]) * 32).round() / 16
    input_ids = torch.randint(0, len(tokens), (length,))
    for at, token in (ids or {}).items():
        input_ids[at] = token
    record = jsm.EncodedRecord(input_ids=tuple(input_ids.tolist()), record_id=name, questions=tuple(
        jsm.EncodedQuestion(str(i), kind, span, tuple(options), tuple(map(str, range(len(options)))))
        for i, (kind, span, options) in enumerate(fields)))
    with torch.inference_mode():
        logits = head(hidden[None].double(), input_ids[None], torch.ones(1, length, dtype=torch.long), [record], lm_head)[0]
    return {"name": name, "ids": input_ids.tolist(), "hidden": hidden.flatten().tolist(),
            "fields": [{"kind": k, "span": list(s), "options": [list(o) for o in opts]} for k, s, opts in fields],
            "logits": [x.tolist() for x in logits]}


cases = [
    # noul, choice and score together; option 22..25 repeats token 7, so its lexical mean counts it twice
    case("mixed", 40, [(0, (5, 9), [(10, 14), (15, 17)]),
                       (1, (18, 21), [(22, 25), (26, 27), (28, 31)]),
                       (2, (32, 33), [(33, 35), (35, 36), (36, 38), (38, 39)])], ids={22: 7, 23: 7, 24: 9}),
    # one question with one option; spans at both ends of the prompt
    case("single", 12, [(1, (0, 3), [(9, 12)])]),
    # 140 options: more query rows than one attention step takes
    case("many", 300, [(1, (1, 4), [(10 + 2 * j, 11 + 2 * j + j % 2) for j in range(140)]),
                       (0, (292, 296), [(296, 298), (298, 300)])]),
]
(OUT / "head_cases.json").write_text(json.dumps(cases) + "\n")
print("written", [c["name"] for c in cases])
