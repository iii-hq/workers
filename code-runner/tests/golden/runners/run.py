# code-runner runner — planted at runtime creation. Do not edit in place.
# Protocol: argv = [source_path]; stdin = JSON envelope
# {"sentinel": "<uuid>", "payload": <payload>}, consumed before the
# handler's source ever loads. Result = JSON printed after a line holding
# only the sentinel. Exit 0 = result, exit 1 = {"error": "..."}. A
# malformed/missing envelope has no sentinel to frame a reply with: it is
# reported on stderr and the process exits non-zero with no stdout at all.
import importlib.util
import inspect
import json
import sys


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

    sys.stdout.write("\n" + sentinel + "\n" + body + "\n")
    return code


sys.exit(main())
