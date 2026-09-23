#!/usr/bin/env python3
"""Repack a laya checkpoint's `encoder.*` tensors as an HF ModernBERT folder for
llama.cpp's converter (the decision head stays in the worker).

  convert-encoder.py <laya snapshot dir> <tokenizer dir> <out dir>
  python llama.cpp/convert_hf_to_gguf.py <out dir> --outfile laya-encoder-f16.gguf --outtype f16

Only f16 keeps laya's calibrated probabilities (Q8_0 moved them by up to 0.04
and flipped one fixture's answer at 512 tokens).
"""
import json, os, shutil, struct, sys

snap, tok, out = sys.argv[1:4]
os.makedirs(out, exist_ok=True)
src = os.path.realpath(os.path.join(snap, "model.safetensors"))
with open(src, "rb") as f:
    n = struct.unpack("<Q", f.read(8))[0]
    header = json.loads(f.read(n))
    base = 8 + n
    header.pop("__metadata__", None)
    enc = {k: v for k, v in header.items() if k.startswith("encoder.")}
    new_header, blobs, off = {}, [], 0
    for k in sorted(enc):
        v = enc[k]
        a, b = v["data_offsets"]
        f.seek(base + a)
        blobs.append(f.read(b - a))
        new_header["model." + k[len("encoder."):]] = {"dtype": v["dtype"], "shape": v["shape"], "data_offsets": [off, off + b - a]}
        off += b - a
hb = json.dumps(new_header).encode()
hb += b" " * (-len(hb) % 8)
with open(os.path.join(out, "model.safetensors"), "wb") as f:
    f.write(struct.pack("<Q", len(hb)))
    f.write(hb)
    for blob in blobs:
        f.write(blob)
cfg = json.load(open(os.path.join(snap, "encoder/config.json")))
cfg["architectures"] = ["ModernBertForMaskedLM"]
json.dump(cfg, open(os.path.join(out, "config.json"), "w"), indent=1)
for name in ("tokenizer.json", "tokenizer_config.json", "special_tokens_map.json"):
    shutil.copy(os.path.join(tok, name), out)
print(f"{len(new_header)} encoder tensors, {off} bytes -> {out}")
