# code-runner eval wrapper — planted at runtime creation. Do not edit in
# place. Runs an eval file as `python3 <file>` would, with the `iii`
# builtin (a lazy handle on the real iii-sdk client — see
# code_runner_iii.py) installed first. If the eval used iii, the client is
# shut down after the run so the process can exit.
import builtins
import importlib.util
import os
import runpy
import sys

if len(sys.argv) < 2:
    sys.stderr.write("code-runner eval wrapper: missing target file argument\n")
    sys.exit(1)

_lib = os.path.join(os.path.dirname(os.path.abspath(__file__)), "code_runner_iii.py")
_spec = importlib.util.spec_from_file_location("code_runner_iii", _lib)
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)
# A builtin, not a module global: builtins are the fallback of every
# module's name lookup, so `iii` resolves inside the evaluated file.
_iii = _mod.make_iii()
builtins.iii = _iii

_target = sys.argv[1]
# [eval.py, file, ...] -> [file, ...]: the evaluated file sees the argv a
# direct run would have given it.
sys.argv = sys.argv[1:]
try:
    runpy.run_path(_target, run_name="__main__")
finally:
    if _iii._client is not None:
        try:
            _iii._client.shutdown()
        except Exception:
            pass
