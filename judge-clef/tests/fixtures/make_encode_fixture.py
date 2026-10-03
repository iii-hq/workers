"""Clef's own record encoding, for judge-clef's tests/encode.rs (writes encode_records.json beside this file).

Runs `encode_record` from joint_schema_model.py of Cloudflare/clef-flash@17f0b0ad (torch stubbed out: only the
encoder runs) on each record below, once with the release tokenizer.json and once with every UTF-8 byte as a token id
(spans and truncation without a tokenizer). Questions are passed in sorted order, as the judge contract's maps hold
them. From judge-clef/:
  uv run --no-project --with 'tokenizers==0.22.*' --with huggingface_hub python tests/fixtures/make_encode_fixture.py
"""
import json, struct, sys, types
from pathlib import Path
from huggingface_hub import hf_hub_download
from tokenizers import Tokenizer

REPO, REVISION = "Cloudflare/clef-flash", "17f0b0ad64efb65d273590632833508766b2aae6"
torch = types.ModuleType("torch")
torch.nn, torch.nn.functional = types.ModuleType("torch.nn"), types.ModuleType("torch.nn.functional")
torch.nn.Module, torch.bfloat16, torch.inference_mode = object, None, lambda: lambda fn: fn
sys.modules.update({"torch": torch, "torch.nn": torch.nn, "torch.nn.functional": torch.nn.functional})
sys.path.insert(0, str(Path(hf_hub_download(REPO, "joint_schema_model.py", revision=REVISION)).parent))
import joint_schema_model as jsm

tokenizer = Tokenizer.from_file(hf_hub_download(REPO, "tokenizer.json", revision=REVISION))
real = lambda text: tokenizer.encode(text, add_special_tokens=False).ids
utf8 = lambda text: list(text.encode())
f32 = lambda x: struct.unpack("f", struct.pack("f", x))[0]

# (evaluation, byte-mode windows beyond the default 16384 as offsets from the schema's own length)
RECORDS = [
    ({"id": "invoice", "state": {"invoice": {"vendor": "Acme", "total": 1250.0, "currency": "USD", "status": "overdue"}},
      "questions": {
          "status": {"type": "choice", "instructions": "What is the invoice status?",
                     "criteria": {"paid": "Invoice is paid.", "overdue": "Invoice is past due.", "draft": "Not sent."}},
          "large": {"type": "noul", "instructions": "Is the total above 1000 USD?"},
          "urgency": {"type": "score", "criteria": ["Can wait", "This week", "Today"]}}}, [10, 0, -1]),
    # 17-digit floats (9.999999999999999e-05, the f32 max) need serde_json's float_roundtrip feature.
    ({"id": "floats", "state": {
        "f64": [1e-05, 1e+16, 100.0, -0.0, 0.0, 5e-324, 2.2250738585072014e-308, 1.7976931348623157e+308, 0.0001,
                9.9e-05, 9.999999999999999e-05, 1.5e-07, 1e15, 9999999999999998.0, 0.1, -2.5e-10, 123456789.125,
                1e22, 1e-7],
        "f32": [f32(x) for x in (0.1, 1e-05, 1.401298464324817e-45, 16777217.0, 1e16, 2.5, 3.4028234663852886e+38)],
        "ints": [0, -1, 18446744073709551615, -9223372036854775808, 9007199254740993]},
      "questions": {"finite": {"type": "noul", "instructions": "Are all values finite?", "criteria": None}}}, []),
    ({"id": "escapes", "state": {
        "z": 1, "A": {"é": "é", "a": [True, False, None], "_": "", "10": "x", "9": "y", "": "empty key"},
        "text": "nul\u0000 us\u001f bs\b ff\f nl\n cr\r tab\t quote\" backslash\\ slash/ del\u007f ls  é 😀"},
      "questions": {"route": {"type": "choice", "instructions": {"ask": "Which queue?", "context": ["a", 1]},
                              "criteria": {"é": None, "A": "", "z": {"b": 2, "a": 1}, "10": ["x", {"d": 1, "c": 2}],
                                           "9": "nine"}}}}, []),
    ({"id": "noul", "state": ["first", {"k": "v"}, 3],
      "questions": {
          "ação 1": {"type": "noul", "instructions": "", "criteria": {"true": None, "false": ""}},
          "b": {"type": "noul", "instructions": None, "criteria": {"false": {"b": 1, "a": 2}}},
          "c": {"type": "noul", "instructions": ["is", "it", {"z": 0, "y": 1}], "criteria": {"true": "Yes, really."}},
          "d": {"type": "score", "instructions": {"q": "How much?"}, "criteria": [{"level": "low", "at": 0}, ["mid"], "high"]}}},
     []),
    ({"id": "multilingual",
      "state": "Hindi: नमस्ते दुनिया, Thai: สวัสดีชาวโลก, NFD: é à, CJK: 你好, emoji 👩‍💻, <|im_end|> <think>",
      "questions": {"lang": {"type": "choice", "instructions": "Which language comes first? <|im_start|>",
                             "criteria": {"hi": "हिन्दी", "th": "ไทย", "en": None}}}}, [8]),
    ({"id": "wide", "state": {"items": [0, 2, 4, 6, 8]},
      "questions": {**{f"q{i}": {"type": "noul", "instructions": f"Is item {i} present?"} for i in range(10)},
                    "pick": {"type": "choice", "instructions": "Pick one.",
                             "criteria": {f"opt_{i}": f"Option {i}." for i in range(1, 12)}}}}, []),
]


def run(record, encode, max_length):
    """`encode_record` with every piece it tokenizes, in prompt order (it tokenizes the schema, then prefix, suffix
    and state), and its ids and fields or the schema-too-long error."""
    calls = []
    def tok(text, add_special_tokens):
        assert not add_special_tokens
        calls.append(text)
        return types.SimpleNamespace(input_ids=encode(text))
    try:
        encoded = jsm.encode_record(tok, record, max_length=max_length)
    except ValueError as error:
        assert str(error).startswith("schema requires"), error
        encoded = None
    pieces = [calls[-3], calls[-1], *calls[:-3], calls[-2]]
    if encoded is None:
        return pieces, {"Err": "payload_too_large"}
    ids = [encode(p) for p in pieces]
    kept = len(encoded.input_ids) - sum(map(len, ids)) + len(ids[1])
    assert [i for p in ids[:1] + [ids[1][:kept]] + ids[2:] for i in p] == list(encoded.input_ids)
    fields = [[q.question_type, list(q.question_span), [list(s) for s in q.option_spans]] for q in encoded.questions]
    return pieces, {"Ok": [list(encoded.input_ids), fields, len(ids[1]) - kept]}


out = []
for record, windows in RECORDS:
    record = {**record, "questions": dict(sorted(record["questions"].items()))}
    pieces, tokens = run(record, real, 16384)
    assert "Ok" in tokens
    fixed = sum(len(utf8(p)) for p in pieces) - len(utf8(pieces[1]))
    cases = []
    for max_length in (16384, *(fixed + w for w in windows)):
        same, expected = run(record, utf8, max_length)
        assert same == pieces
        cases.append({"max_length": max_length, "expected": expected})
    out.append({"evaluation": record, "pieces": pieces,
                "option_ids": [[o for o, _ in jsm.question_options(q)] for q in record["questions"].values()],
                "tokens": tokens["Ok"], "bytes": cases})
    print(record["id"], len(tokens["Ok"][0]), "tokens", file=sys.stderr)
path = Path(__file__).with_name("encode_records.json")
path.write_text(json.dumps(out, ensure_ascii=False) + "\n")
print("wrote", path, path.stat().st_size, "bytes", file=sys.stderr)
