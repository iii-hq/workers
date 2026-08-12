# python-engine's guest wrapper — the host writes this file into /run for
# every call; tenant code never chooses its name or content. Its integrity
# is ORDERING, not secrecy: the envelope is written after tenant code
# finishes, and the host never classifies infrastructure failures from
# anything written under /out (see manager.rs).
import builtins
import json
import sys
import traceback

MAX_RESULT_BYTES = 1048576
MAX_TRACEBACK_CHARS = 16384
# `str(e)` is tenant-controlled and unbounded — `raise RuntimeError("x" *
# 10_000_000)` would otherwise build an envelope the host refuses on read,
# losing the very error the tenant needs to see. Capped here rather than at
# each call site so no future caller can forget.
MAX_MESSAGE_CHARS = 4096
TENANT_FILENAME = "<code>"


def write_error(out_dir, kind, message, tb_text=None):
    if len(message) > MAX_MESSAGE_CHARS:
        message = message[:MAX_MESSAGE_CHARS] + "... message truncated"
    if tb_text is not None and len(tb_text) > MAX_TRACEBACK_CHARS:
        tb_text = tb_text[:MAX_TRACEBACK_CHARS] + "\n... traceback truncated"
    with open(out_dir + "/result.json", "w") as f:
        json.dump({"ok": False, "kind": kind, "message": message, "traceback": tb_text}, f)


def _install_iii(run_dir):
    """Install the `iii` global when the host offered a bridge.

    Returns `(persistent, sentinel)` — the same config file carries both,
    because the bridge and the persistent-turn loop share one channel.


    Absent config file means no bridge, and then there is no `iii` at all —
    a global that exists and always fails is worse than one that is missing,
    because a tenant cannot tell the two apart from inside.

    Installed as a BUILTIN, not a module import: `-I` implies `-P`, so
    `/run` is not on `sys.path` and there is nothing to import. As a builtin
    it also resolves in every module's name lookup, so no amount of tenant
    `sys.path` fiddling changes what `iii` means for code that does not
    rebind it. (A tenant rebinding its own `iii` is shadowing its own name,
    not crossing a boundary.)
    """
    try:
        with open(run_dir + "/iii.json", "rb") as f:
            cfg = json.load(f)
        sentinel = cfg["sentinel"]
    except (OSError, ValueError, KeyError):
        # The file is host-written, so truncation/garbage here is corruption
        # (a partial write during a crash), not tenant reach. A malformed
        # bridge config means NO bridge — not an exception that escapes main
        # before any envelope exists.
        return (False, None)

    namespace = cfg.get("namespace", "")
    stdout = sys.stdout

    def trigger(request):
        if not isinstance(request, dict):
            raise TypeError("iii.trigger: request must be a dict")
        fn = request.get("function_id")
        if not isinstance(fn, str) or not fn:
            raise TypeError("iii.trigger: function_id must be a non-empty string")
        # allow_nan=False: the bare `NaN`/`Infinity` tokens the default emits
        # are not JSON, and the host's strict parser would fail the FRAME —
        # a broken bridge — instead of this call. The ValueError this raises
        # is an ordinary tenant exception at the trigger call site.
        frame = json.dumps(
            {
                "function_id": fn,
                "payload": request.get("payload"),
                "timeout_ms": request.get("timeout_ms"),
            },
            allow_nan=False,
        )
        # The host scans stdout for "\n<sentinel>\n<json>\n". Flushed
        # immediately: the answer cannot come back until the whole frame has.
        stdout.write("\n" + sentinel + "\n" + frame + "\n")
        stdout.flush()

        # Length-prefixed on the way back, so the body needs no escaping.
        body = _read_frame()
        if body is None:
            raise OSError("iii.trigger: the host closed the bridge")
        answer = json.loads(body)
        if answer.get("ok"):
            return answer.get("value")
        raise RuntimeError("iii.trigger: " + str(answer.get("value")))

    class _Iii:
        """An agent's first move against an unknown global is printing it."""

        def __init__(self, ns):
            # NOT `namespace = namespace` in the class body: a class body does
            # not participate in the enclosing function's closure, so the
            # right-hand side would skip the local and raise NameError.
            self.namespace = ns

        def trigger(self, request):
            return trigger(request)

        def __repr__(self):
            return (
                "<iii: host client. e.g. iii.trigger({'function_id': 'worker::fn', "
                "'payload': {}}); iii.namespace is the prefix this runtime may "
                "register under>"
            )

    builtins.iii = _Iii(namespace)
    return (bool(cfg.get("persistent")), sentinel)


def run_once(source, payload, out_dir, ns):
    """Execute one tenant program in namespace `ns` and write its envelope.

    Factored out of `main` so a persistent runtime can call it repeatedly with
    the SAME `ns` — that dict is what makes a global bound in one turn still
    bound in the next. Every exit path still writes exactly one envelope, and
    still writes it LAST — the ordering the host's classifier depends on.
    """
    try:
        code = compile(source, TENANT_FILENAME, "exec")
    except SyntaxError as e:
        write_error(out_dir, "syntax_error", "%s (line %s)" % (e.msg, e.lineno))
        return

    ns["payload"] = payload
    # `result` is this turn's output slot, not carried-over state. Without the
    # pop, a turn that sets no result would silently return the previous
    # turn's — the caller reading a stale answer as their own.
    ns.pop("result", None)
    try:
        exec(code, ns)
    except SystemExit:
        # sys.exit() inside tenant code ends the run; whatever `result`
        # held at that point stands. Documented behavior.
        pass
    except BaseException as e:
        tb = e.__traceback__
        while tb is not None and tb.tb_frame.f_code.co_filename != TENANT_FILENAME:
            tb = tb.tb_next
        text = "".join(traceback.format_exception(type(e), e, tb))
        write_error(out_dir, "python_exception", "%s: %s" % (type(e).__name__, e), text)
        return

    value = ns.get("result")
    try:
        # allow_nan=False so float("nan")/float("inf") raise ValueError and
        # take the unrepresentable path below; the default emits bare `NaN` /
        # `Infinity` tokens that are not JSON, and the host's envelope parse
        # would report the whole run as a bare exit.
        serialized = json.dumps(value, allow_nan=False)
    except (TypeError, ValueError):
        serialized = "null"  # unrepresentable results come back as null (node-engine parity)
    # Build the envelope FIRST, then measure it. Checking `serialized` alone
    # under-counts by the 24 bytes of `{"ok": true, "result": ` + `}` that the
    # write adds, so every result in the top 24 bytes of the range passed here
    # and was then refused by the host's own MAX_RESULT_BYTES check on read —
    # the caller getting neither the success nor `result_too_large`. Measuring
    # what is actually written removes the window and the magic number with it.
    #
    # With json.dumps' default ensure_ascii=True all non-ASCII is escaped as
    # \uXXXX, so MAX_RESULT_BYTES measures the *escaped* size: a result heavy
    # in non-ASCII is charged ~6 bytes per character. Deliberate, and
    # documented in README.md. The write is binary so it cannot fail on the
    # sandbox's locale encoding, which we do not control under -I.
    envelope = b'{"ok": true, "result": ' + serialized.encode("utf-8", errors="replace") + b"}"
    if len(envelope) > MAX_RESULT_BYTES:
        write_error(
            out_dir,
            "result_too_large",
            "serialized result envelope is %d bytes; max %d" % (len(envelope), MAX_RESULT_BYTES),
        )
        return
    with open(out_dir + "/result.json", "wb") as f:
        f.write(envelope)


def _read_frame():
    """Read one `<byte-length>\\n<body>` frame from the host, or None at EOF.

    Reads the BINARY stream, never `sys.stdin`. The host's prefix counts
    BYTES, while text-mode `read(n)` counts CHARACTERS and `len()` counts
    them too — so a single non-ASCII character in the body made the loop ask
    for bytes the host had already finished sending, and the guest blocked
    until its deadline. Reading bytes and decoding once makes the two ends
    measure the same thing.
    """
    raw = sys.stdin.buffer
    header = b""
    while True:
        ch = raw.read(1)
        if not ch:
            return None
        if ch == b"\n":
            break
        header += ch
    want = int(header)
    body = b""
    while len(body) < want:
        chunk = raw.read(want - len(body))
        if not chunk:
            return None
        body += chunk
    return body.decode("utf-8")


def main():
    run_dir, out_dir = sys.argv[1], sys.argv[2]
    persistent, sentinel = _install_iii(run_dir)

    if not persistent:
        # One-shot: the program and its payload are files the host wrote
        # before starting us, and we exit when it is done.
        with open(run_dir + "/code.py", "rb") as f:
            source = f.read().decode("utf-8", errors="replace")
        try:
            with open(run_dir + "/payload.json", "rb") as f:
                payload = json.load(f)
        except OSError:
            payload = None
        run_once(source, payload, out_dir, {"__name__": "__main__"})
        return

    # Persistent: the interpreter outlives the call. Each turn arrives on
    # stdin, and the frame we emit afterwards is what tells the host the turn
    # is over — it is not a trust boundary, just a boundary. A tenant that
    # forges one early only makes the host read an envelope it has not
    # written yet, which is reported as its own failure.
    ns = {"__name__": "__main__"}
    while True:
        frame = _read_frame()
        if frame is None:
            return
        turn = json.loads(frame)
        run_once(turn.get("code", ""), turn.get("payload"), out_dir, ns)
        sys.stdout.write("\n" + sentinel + "\n" + json.dumps({"kind": "turn_done"}) + "\n")
        sys.stdout.flush()


main()
