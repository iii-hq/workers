#!/usr/bin/env python3
"""Run llama.cpp's convert_hf_to_gguf.py from a checkout, accepting tokenizers
whose pre-tokenizer it does not recognize (mmBERT's): the worker never uses
llama.cpp's tokenizer, so `default` is fine.

  convert-gguf.py <llama.cpp checkout> <hf dir> <out.gguf>
"""
import runpy, sys

checkout, hf_dir, out = sys.argv[1:4]
sys.path.insert(0, f"{checkout}/gguf-py")
sys.path.insert(0, checkout)
from conversion import base  # noqa: E402

original = base.TextModel.get_vocab_base_pre
def lenient(self, tokenizer):
    try:
        return original(self, tokenizer)
    except NotImplementedError:
        return "default"
base.TextModel.get_vocab_base_pre = lenient
sys.argv = ["convert_hf_to_gguf.py", hf_dir, "--outfile", out, "--outtype", "f16"]
runpy.run_path(f"{checkout}/convert_hf_to_gguf.py", run_name="__main__")
