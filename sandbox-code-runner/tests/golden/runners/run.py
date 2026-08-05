# code-runner runner — planted at runtime creation. Do not edit in place.
# Protocol: argv = [source_path]; stdin = JSON envelope
# {"sentinel": "<uuid>", "payload": <payload>}, consumed before the
# handler's source ever loads. Result = JSON printed after a line holding
# only the sentinel. Exit 0 = result, exit 1 = {"error": "..."}. A
# malformed/missing envelope has no sentinel to frame a reply with: it is
# reported on stderr and the process exits non-zero with no stdout at all.
# Handlers get the same `iii` global evaluated code gets, built by the
# sibling iii.py (planted next to this file at runtime creation).
import builtins
import importlib.util
import inspect
import json
import os
import sys


def install_iii():
    lib = os.path.join(os.path.dirname(os.path.abspath(__file__)), "code_runner_iii.py")
    spec = importlib.util.spec_from_file_location("code_runner_iii", lib)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    # A builtin, not a module-level name: builtins are the fallback of every
    # module's name lookup, so `iii` resolves inside the handler's own module
    # no matter how it was loaded.
    builtins.iii = mod.make_iii()
    return builtins.iii


def main():
    source = sys.argv[1]

    raw = sys.stdin.read()
    try:
        envelope = json.loads(raw)
    except json.JSONDecodeError:
        envelope = None

    if not isinstance(envelope, dict) or not isinstance(envelope.get("sentinel"), str):
        sys.stderr.write(
            'code-runner runner: malformed envelope on stdin (expected {"sentinel": "...", "payload": ...})\n'
        )
        return 1

    sentinel = envelope["sentinel"]
    payload = envelope.get("payload")

    iii_obj = install_iii()

    def run():
        spec = importlib.util.spec_from_file_location("code_runner_handler", source)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        handler = getattr(mod, "handler", None)
        if not callable(handler):
            raise TypeError("source must define a function named 'handler(payload)'")
        if inspect.iscoroutinefunction(handler):
            raise TypeError(
                "'async def handler' is not supported in v1; define a plain 'def handler(payload)'"
            )
        return handler(payload)

    try:
        body, code = json.dumps(run()), 0
    except BaseException as exc:
        body, code = json.dumps({"error": f"{type(exc).__name__}: {exc}"}), 1

    # Shut the iii client down BEFORE the frame, so any output it produces
    # lands in the logs region and never after the result line — and so the
    # process can exit without the exec timing out on a live connection.
    if iii_obj._client is not None:
        try:
            iii_obj._client.shutdown()
        except Exception:
            pass

    sys.stdout.write("\n" + sentinel + "\n" + body + "\n")
    return code


sys.exit(main())
