//! The language table and the runner protocol — how sandbox-code-runner
//! talks to a process inside the guest.
//!
//! Every guest process (an eval and a registered-function call alike) gets a
//! global `iii`: a LAZY handle on the real iii-sdk client
//! (<https://iii.dev/docs/reference/sdk-node> /
//! <https://iii.dev/docs/reference/sdk-python>), connected to the engine
//! over the sandbox's network gateway (`III_URL`, set at runtime creation).
//! Nothing connects until the first use, so code that never touches `iii`
//! pays nothing. Node runtimes get the SDK planted as an embedded
//! single-file bundle under `/node_modules/iii-sdk`; Python runtimes
//! `pip install iii-sdk` at creation (its pydantic-core dependency is
//! compiled per-platform, so there is no plantable pure-Python form).
//!
//! Per registered-function call, the manager execs the runtime's runner with
//! `argv = [source_path]` and, on stdin, a JSON envelope
//! `{"sentinel": "<uuid>", "payload": <payload>}`. The runner reads and
//! parses that envelope BEFORE loading the handler's source, keeps the
//! sentinel in a variable local to its own entry-point function — never at
//! module scope, since Python always registers the running script as
//! `sys.modules['__main__']` and a module-level name would have been a
//! plain, guessably-named attribute on it — and calls `handler(payload)`
//! with only the payload. On completion it prints a line holding only the
//! sentinel followed by the JSON result.
//!
//! What the sentinel is FOR: framing the result in the runner's stdout so an
//! ordinary handler's own prints (its "logs") can never be mistaken for the
//! result. It is a fresh UUID minted per call, delivered out of band on
//! stdin, and consumed before any handler code runs. It is not reachable
//! through any AMBIENT channel a handler might touch for unrelated reasons —
//! argv, environment variables, a re-read of stdin, or a module-level
//! attribute — so an ordinary handler cannot produce or collide with it by
//! accident.
//!
//! What the sentinel is NOT: a security boundary, and no list of bypass
//! techniques would make it one. The handler runs inside the runner's own
//! process, so it can read anything that process can read and write
//! anything that process can write — this frame included, by reassigning
//! `process.stdout.write` / `sys.stdout.write` before its own code ever
//! runs. Nothing here defends against that, and nothing needs to: a handler
//! already determines its own return value — that is what a handler is —
//! so there is no boundary between "the handler" and "this call's result"
//! to defend. Isolation between runtimes, and between guest and host, is
//! the microVM's job, not the sentinel's.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Node,
    Python,
}

impl Lang {
    /// The iii-sandbox preset image name.
    pub fn image(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
        }
    }
    /// The interpreter binary inside that image.
    pub fn interpreter(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python3",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Self::Node => "mjs",
            Self::Python => "py",
        }
    }
    /// Where `create` plants this language's runner inside the guest.
    pub fn runner_path(self) -> &'static str {
        match self {
            Self::Node => "/opt/sandbox-code-runner/invoke.mjs",
            Self::Python => "/opt/sandbox-code-runner/invoke.py",
        }
    }
    pub fn runner_source(self) -> &'static str {
        match self {
            Self::Node => INVOKE_MJS,
            Self::Python => INVOKE_PY,
        }
    }
    /// Where `create` plants the guest `iii` library. The runner and the
    /// run wrapper both resolve it RELATIVE to their own file, so the
    /// trio only has to land in one directory together (which also lets
    /// tests run them from a scratch dir with no `/opt` at all). The
    /// Python file is deliberately NOT `iii.py`: `python3 <script>` puts
    /// the script's own directory at `sys.path[0]`, and a sibling
    /// `iii.py` would shadow the SDK's real `iii` package.
    pub fn iii_lib_path(self) -> &'static str {
        match self {
            Self::Node => "/opt/sandbox-code-runner/iii.mjs",
            Self::Python => "/opt/sandbox-code-runner/sandbox_code_runner_iii.py",
        }
    }
    pub fn iii_lib_source(self) -> &'static str {
        match self {
            Self::Node => III_MJS,
            Self::Python => III_PY,
        }
    }
    /// Where `create` plants the run wrapper — what `run` execs instead
    /// of the bare interpreter, so the code it runs gets the `iii` global.
    pub fn run_wrapper_path(self) -> &'static str {
        match self {
            Self::Node => "/opt/sandbox-code-runner/run.mjs",
            Self::Python => "/opt/sandbox-code-runner/run.py",
        }
    }
    pub fn run_wrapper_source(self) -> &'static str {
        match self {
            Self::Node => RUN_MJS,
            Self::Python => RUN_PY,
        }
    }
    /// Everything `create` plants into a fresh runtime of this language,
    /// as `(path, content)` pairs. Node additionally gets the embedded
    /// SDK bundle at root `/node_modules` — the ESM resolver walks UP
    /// from the importing file, so that one location serves run files in
    /// `/tmp/sandbox-code-runner`, handlers in `/opt/sandbox-code-runner/fns`,
    /// and any file a tenant writes anywhere else. Python's SDK comes from
    /// the `pip install` step in `create` instead — see the module doc.
    pub fn guest_files(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Node => &[
                ("/opt/sandbox-code-runner/invoke.mjs", INVOKE_MJS),
                ("/opt/sandbox-code-runner/iii.mjs", III_MJS),
                ("/opt/sandbox-code-runner/run.mjs", RUN_MJS),
                ("/node_modules/iii-sdk/package.json", SDK_PACKAGE_JSON),
                ("/node_modules/iii-sdk/dist/index.mjs", SDK_BUNDLE_MJS),
            ],
            Self::Python => &[
                ("/opt/sandbox-code-runner/invoke.py", INVOKE_PY),
                (
                    "/opt/sandbox-code-runner/sandbox_code_runner_iii.py",
                    III_PY,
                ),
                ("/opt/sandbox-code-runner/run.py", RUN_PY),
            ],
        }
    }
}

/// The published `iii-sdk` npm package with its whole dependency graph
/// inlined into one node ESM file — built by `ui/build.mjs` from the
/// pinned dependency in `ui/package.json`, embedded here so a Node
/// runtime needs no registry and no `npm install` to have the real SDK.
pub const SDK_BUNDLE_MJS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/ui/dist/iii-sdk-guest.mjs"
));

/// The planted package's manifest, generated by the same build from the
/// resolved dependency (the SDK reads `../package.json` for its own
/// version at runtime, so this must exist and must not drift).
pub const SDK_PACKAGE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/ui/dist/iii-sdk-guest-package.json"
));

/// One stdout emit point at the very end of the happy/error path, so a
/// partial write can never leave a sentinel with no result behind it.
/// `JSON.stringify` of a non-serializable value (a function) yields
/// `undefined`, caught explicitly; a circular value throws, caught by the
/// catch. `process.exitCode` instead of `process.exit()` so stdout flushes
/// before the process ends. A malformed envelope is a separate, earlier
/// failure mode: there is no sentinel yet to frame a reply with, so it goes
/// to stderr instead and stdout is never touched.
pub const INVOKE_MJS: &str = r#"// sandbox-code-runner invoke runner — planted at runtime creation. Do not edit in place.
// Protocol: argv = [source_path]; stdin = JSON envelope
// {"sentinel": "<uuid>", "payload": <payload>}, consumed before the
// handler's source ever loads. Result = JSON printed after a line holding
// only the sentinel. Exit 0 = result, exit 1 = {"error": "..."}. A
// malformed/missing envelope has no sentinel to frame a reply with: it is
// reported on stderr and the process exits non-zero with no stdout at all.
// Handlers get the same `iii` global run code gets, built by the
// sibling iii.mjs (planted next to this file at runtime creation).
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { makeIii } from './iii.mjs';

async function main() {
  const [source] = process.argv.slice(2);
  const raw = readFileSync(0, 'utf8');
  let envelope = null;
  try {
    envelope = JSON.parse(raw);
  } catch {
    envelope = null;
  }
  if (envelope === null || typeof envelope !== 'object' || typeof envelope.sentinel !== 'string') {
    process.stderr.write(
      'sandbox-code-runner invoke runner: malformed envelope on stdin (expected {"sentinel": "...", "payload": ...})\n'
    );
    process.exitCode = 1;
    return;
  }
  const { sentinel, payload } = envelope;

  const { iii, client } = await makeIii();
  globalThis.iii = iii;

  let body;
  let code;
  try {
    const mod = await import(pathToFileURL(source).href);
    if (typeof mod.handler !== 'function') {
      throw new TypeError("source must export a function named 'handler(payload)'");
    }
    const out = await mod.handler(payload);
    body = JSON.stringify(out === undefined ? null : out);
    if (body === undefined) {
      throw new TypeError('handler result is not JSON-serializable');
    }
    code = 0;
  } catch (e) {
    body = JSON.stringify({ error: String((e && e.message) || e) });
    code = 1;
  }

  // Shut the iii client down BEFORE the frame, so any output it produces
  // lands in the logs region and never after the result line — and so the
  // process can exit without the exec timing out on an open socket. Capped
  // and unref'd for the same reason as the run wrapper's.
  const c = client();
  if (c) {
    await Promise.race([
      c.shutdown().catch(() => {}),
      new Promise((r) => setTimeout(r, 2000).unref()),
    ]);
  }

  process.stdout.write('\n' + sentinel + '\n' + body + '\n');
  process.exitCode = code;
}

await main();
"#;

/// Same single-emit shape as `INVOKE_MJS`, and now the same scoping shape too:
/// the envelope, `sentinel`, and `payload` all live inside `main()`, never
/// at module scope. Python always registers the running script as
/// `sys.modules['__main__']`, so a module-level `sentinel = ...` would have
/// been a plain attribute any handler could read off it by name —
/// `getattr(sys.modules['__main__'], 'sentinel', None)` — regardless of how
/// the handler itself was loaded. `main()` RETURNS its exit code instead of
/// calling `sys.exit()` itself, so `sys.exit(main())` at module scope is the
/// only exit call and it stays OUTSIDE every `try`: the malformed-envelope
/// path returns 1 before the result-framing `try` is ever entered, exactly
/// as `INVOKE_MJS`'s `return` does before its own inner `try`.
pub const INVOKE_PY: &str = r#"# sandbox-code-runner invoke runner — planted at runtime creation. Do not edit in place.
# Protocol: argv = [source_path]; stdin = JSON envelope
# {"sentinel": "<uuid>", "payload": <payload>}, consumed before the
# handler's source ever loads. Result = JSON printed after a line holding
# only the sentinel. Exit 0 = result, exit 1 = {"error": "..."}. A
# malformed/missing envelope has no sentinel to frame a reply with: it is
# reported on stderr and the process exits non-zero with no stdout at all.
# Handlers get the same `iii` global run code gets, built by the
# sibling sandbox_code_runner_iii.py (planted next to this file at runtime creation).
import builtins
import importlib.util
import inspect
import json
import os
import sys


def install_iii():
    lib = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sandbox_code_runner_iii.py")
    spec = importlib.util.spec_from_file_location("sandbox_code_runner_iii", lib)
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
            'sandbox-code-runner invoke runner: malformed envelope on stdin (expected {"sentinel": "...", "payload": ...})\n'
        )
        return 1

    sentinel = envelope["sentinel"]
    payload = envelope.get("payload")

    iii_obj = install_iii()

    def run():
        spec = importlib.util.spec_from_file_location("sandbox_code_runner_handler", source)
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
"#;

/// The guest `iii` global — Node. A LAZY handle on the real iii-sdk
/// client: `makeIii()` never connects by itself; the first property access
/// calls `registerWorker(III_URL)` and every later access reaches the same
/// live client. Resolved by the runner and the run wrapper as a SIBLING
/// import, so the same bytes work from `/opt/sandbox-code-runner` in a VM and
/// from a scratch dir in tests.
pub const III_MJS: &str = r#"// sandbox-code-runner guest iii library — planted at runtime creation. Do not edit
// in place. makeIii() returns the global every run and handler gets: a
// LAZY handle on the real iii-sdk client (planted at /node_modules/iii-sdk).
// Nothing connects until the first property access, so code that never
// touches `iii` pays nothing.
export async function makeIii() {
  let sdk = null;
  let importError = null;
  try {
    sdk = await import('iii-sdk');
  } catch (e) {
    importError = e;
  }

  let client = null;
  const resolve = () => {
    if (client) return client;
    if (!sdk) {
      throw new Error(
        `iii is unavailable: the iii-sdk package could not be loaded (${importError && importError.message})`
      );
    }
    const url = process.env.III_URL;
    if (!url) {
      throw new Error('iii is unavailable: III_URL is not set for this runtime');
    }
    // The SDK console.debug's its own lifecycle lines — "[OTel] ..." at
    // setup, "[iii] Worker registered with ID: ..." ASYNCHRONOUSLY on
    // connect — and console.debug is stdout, which for a run IS the
    // result surface. Filter exactly those prefixed debug lines,
    // permanently but only once code actually uses `iii` (this function
    // is lazy); every other console.debug still goes through untouched.
    const debug = console.debug;
    console.debug = (...args) => {
      if (
        typeof args[0] === 'string' &&
        (args[0].startsWith('[iii]') || args[0].startsWith('[OTel]'))
      ) {
        return;
      }
      debug(...args);
    };
    client = sdk.registerWorker(url, {
      workerName: process.env.III_WORKER_NAME || undefined,
      // Guest processes are momentary; worker gauges would only warn
      // (OTel is disabled in the VM) and report nothing useful.
      enableMetricsReporting: false,
    });
    return client;
  };

  // A lazy proxy, not the client itself: METHOD access is what triggers
  // the connection; INTROSPECTION never does. Printing the global, listing
  // its keys, or reading its prototype must answer something useful and
  // must never dial the engine — an agent's first move against an unknown
  // global is exactly that probing, and an opaque `{}` here cost a live
  // session six blind runs (console-a2795be8).
  const HINT =
    "[iii: lazy iii-sdk client — connects on first use. e.g. await iii.trigger({ function_id: 'worker::fn', payload: {} }); registerFunction(id, handler, opts?); docs <https://iii.dev/docs/reference/sdk-node>]";

  const lookup = (prop) => {
    const c = resolve();
    const value = c[prop];
    // Bind methods so `const t = iii.trigger; await t(...)` works; leave
    // `constructor` alone so introspection sees the real class, not
    // "bound III".
    return typeof value === 'function' && prop !== 'constructor' ? value.bind(c) : value;
  };

  const iii = new Proxy(Object.create(null), {
    get(_, prop) {
      if (client === null) {
        // Pre-connection: only the hint surfaces (inspect/string coercion);
        // everything non-string — and a bare `await iii` — stays inert.
        if (prop === Symbol.for('nodejs.util.inspect.custom') || prop === 'toString') {
          return () => HINT;
        }
        if (typeof prop !== 'string' || prop === 'then') {
          return undefined;
        }
        return lookup(prop);
      }
      if (typeof prop !== 'string') {
        return undefined;
      }
      return lookup(prop);
    },
    // Never null: `Object.getPrototypeOf(iii)` crashing a tenant's own
    // introspection (`getOwnPropertyNames(getPrototypeOf(iii))` did, live)
    // is exactly the confusion this proxy must not cause.
    getPrototypeOf() {
      return client ? Reflect.getPrototypeOf(client) : Object.prototype;
    },
    has(_, prop) {
      return client ? Reflect.has(client, prop) : false;
    },
    // Once connected, `Object.keys(iii)` answers "what can I call": the
    // client's own properties plus its prototype methods. Before that it
    // stays empty — listing keys must not connect.
    ownKeys() {
      if (!client) return [];
      const keys = new Set();
      let o = client;
      while (o && o !== Object.prototype) {
        for (const k of Reflect.ownKeys(o)) {
          if (typeof k === 'string' && k !== 'constructor') keys.add(k);
        }
        o = Reflect.getPrototypeOf(o);
      }
      return [...keys];
    },
    getOwnPropertyDescriptor(_, prop) {
      if (!client || typeof prop !== 'string') return undefined;
      return { configurable: true, enumerable: true, value: lookup(prop) };
    },
  });

  return { iii, client: () => client };
}
"#;

/// The guest `iii` global — Python. Same lazy contract as [`III_MJS`]; the
/// SDK itself comes from the `pip install iii-sdk` step at runtime
/// creation.
pub const III_PY: &str = r#"# sandbox-code-runner guest iii library — planted at runtime creation. Do not edit
# in place. make_iii() returns the global every run and handler gets: a
# LAZY handle on the real iii-sdk client (pip-installed at runtime
# creation). Nothing connects until the first attribute access, so code
# that never touches `iii` pays nothing.
#
# NOT named iii.py: `python3 <script>` puts the script's own directory at
# sys.path[0], and a sibling iii.py would shadow the SDK's real `iii`
# package for every run and handler.
import os


class _LazyIii:
    def __init__(self):
        self._client = None

    def _resolve(self):
        if self._client is not None:
            return self._client
        url = os.environ.get("III_URL")
        if not url:
            raise RuntimeError("iii is unavailable: III_URL is not set for this runtime")
        try:
            from iii import InitOptions, register_worker
        except ModuleNotFoundError as exc:
            raise RuntimeError(
                "iii is unavailable: the iii-sdk package is not installed in this "
                "runtime (its pip install at runtime creation may have failed): "
                f"{exc}"
            ) from exc
        name = os.environ.get("III_WORKER_NAME")
        options = InitOptions(worker_name=name) if name else None
        self._client = register_worker(url, options)
        return self._client

    def __getattr__(self, name):
        return getattr(self._resolve(), name)

    # Introspection never connects: printing the global or dir()-ing it is
    # how an unknown API gets explored, and it must answer usefully with no
    # side effects.
    def __repr__(self):
        if self._client is None:
            return (
                "<iii: lazy iii-sdk client (connects on first use); e.g. "
                "iii.trigger({'function_id': 'worker::fn', 'payload': {}}); "
                "docs: https://iii.dev/docs/reference/sdk-python>"
            )
        return repr(self._client)

    def __dir__(self):
        return [] if self._client is None else dir(self._client)


def make_iii():
    return _LazyIii()
"#;

/// What `run` execs instead of the bare interpreter — Node. Installs the
/// lazy `iii` global, rebases argv so the target file sees itself at
/// `argv[1]`, and otherwise behaves exactly like `node <file>`: same
/// stdout/stderr, and an uncaught error (a rejected top-level `await
/// import`) prints its stack and exits non-zero on its own — the
/// `finally` shutdown runs first, capped and unref'd so it can neither
/// hang the exec nor keep an idle loop alive.
pub const RUN_MJS: &str = r#"// sandbox-code-runner run wrapper — planted at runtime creation. Do not edit in
// place. Runs a file as `node <file>` would, with the `iii` global (a
// lazy handle on the real iii-sdk client — see iii.mjs) installed first. If
// the run used iii, the client is shut down after the run so the process
// can exit.
import { pathToFileURL } from 'node:url';
import { makeIii } from './iii.mjs';

const target = process.argv[2];
if (!target) {
  process.stderr.write('sandbox-code-runner run wrapper: missing target file argument\n');
  process.exit(1);
}
const { iii, client } = await makeIii();
globalThis.iii = iii;
// [node, run.mjs, file] -> [node, file]: the target file sees the argv
// a direct run would have given it.
process.argv.splice(1, 2, target);
try {
  await import(pathToFileURL(target).href);
} finally {
  const c = client();
  if (c) {
    await Promise.race([
      c.shutdown().catch(() => {}),
      new Promise((r) => setTimeout(r, 3000).unref()),
    ]);
  }
}
"#;

/// What `run` execs instead of the bare interpreter — Python. Same
/// contract as [`RUN_MJS`]: a raised exception propagates out of
/// `runpy.run_path` (after the `finally` shutdown), prints its traceback,
/// and exits non-zero, exactly as `python3 <file>` does.
pub const RUN_PY: &str = r#"# sandbox-code-runner run wrapper — planted at runtime creation. Do not edit in
# place. Runs a file as `python3 <file>` would, with the `iii`
# builtin (a lazy handle on the real iii-sdk client — see
# sandbox_code_runner_iii.py) installed first. If the run used iii, the client is
# shut down after the run so the process can exit.
import builtins
import importlib.util
import os
import runpy
import sys

if len(sys.argv) < 2:
    sys.stderr.write("sandbox-code-runner run wrapper: missing target file argument\n")
    sys.exit(1)

_lib = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sandbox_code_runner_iii.py")
_spec = importlib.util.spec_from_file_location("sandbox_code_runner_iii", _lib)
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)
# A builtin, not a module global: builtins are the fallback of every
# module's name lookup, so `iii` resolves inside the target file.
_iii = _mod.make_iii()
builtins.iii = _iii

_target = sys.argv[1]
# [run.py, file, ...] -> [file, ...]: the target file sees the argv a
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
"#;

pub struct RunnerOutput {
    /// Everything the handler printed before the sentinel — returned to the
    /// caller as logs, never parsed.
    pub logs: String,
    /// The single line of JSON text immediately after the sentinel line.
    /// `None` when the sentinel never appeared in stdout at all — which
    /// covers two distinct causes indistinguishably: the interpreter
    /// crashed (OOM-killed, bad shebang, segfault, …) before it could write
    /// the frame, OR the handler itself called `process.exit()` /
    /// `os._exit()` and the runner never reached its own final write. Either
    /// way, there is no result to report.
    pub result: Option<String>,
}

/// Split an exec's stdout at the sentinel LINE, taking only the FIRST LINE
/// after it as the result. The runner always writes exactly
/// `"\n" + sentinel + "\n" + body + "\n"`, where `body` comes from
/// `JSON.stringify` / `json.dumps` — both escape embedded newlines, so a
/// serialized result is always exactly one line. Anything on a LATER line
/// (a dangling `setTimeout` firing after the frame, a live non-daemon
/// thread that outlives the runner's own exit) is therefore, by
/// construction, not part of the result: it is dropped rather than
/// appended, which would otherwise corrupt the parse. The first occurrence
/// of the needle is the runner's own frame for ordinary handler output —
/// the sentinel is a per-call UUID delivered out of band and never handed
/// to the handler, so it can't collide by accident. A handler that
/// deliberately intercepts the runner's own write and emits a forged frame
/// first is a different matter this function has no way to detect; see the
/// module doc for why that isn't something the sentinel defends against.
pub fn split_sentinel(stdout: &str, sentinel: &str) -> RunnerOutput {
    let needle = format!("\n{sentinel}\n");
    match stdout.find(&needle) {
        Some(i) => {
            let after = &stdout[i + needle.len()..];
            let result = after.split('\n').next().unwrap_or("");
            RunnerOutput {
                logs: stdout[..i].to_string(),
                result: Some(result.to_string()),
            }
        }
        None => RunnerOutput {
            logs: stdout.to_string(),
            result: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_table_is_exact() {
        assert_eq!(Lang::Node.image(), "node");
        assert_eq!(Lang::Python.image(), "python");
        assert_eq!(Lang::Node.interpreter(), "node");
        assert_eq!(Lang::Python.interpreter(), "python3");
        assert_eq!(Lang::Node.ext(), "mjs");
        assert_eq!(Lang::Python.ext(), "py");
        assert_eq!(
            Lang::Node.runner_path(),
            "/opt/sandbox-code-runner/invoke.mjs"
        );
        assert_eq!(
            Lang::Python.runner_path(),
            "/opt/sandbox-code-runner/invoke.py"
        );
        assert_eq!(
            Lang::Node.iii_lib_path(),
            "/opt/sandbox-code-runner/iii.mjs"
        );
        assert_eq!(
            Lang::Python.iii_lib_path(),
            "/opt/sandbox-code-runner/sandbox_code_runner_iii.py",
            "must never be iii.py — sys.path[0] would shadow the SDK package"
        );
        assert_eq!(
            Lang::Node.run_wrapper_path(),
            "/opt/sandbox-code-runner/run.mjs"
        );
        assert_eq!(
            Lang::Python.run_wrapper_path(),
            "/opt/sandbox-code-runner/run.py"
        );
    }

    /// The plant table is what `create` writes; pin the entries whose
    /// LOCATION is load-bearing: the SDK must sit at root `/node_modules`
    /// (the ESM upward walk from any tenant file ends there), and no
    /// Python entry may be named `iii.py` (sys.path[0] shadowing).
    #[test]
    fn guest_file_tables_are_exact() {
        let node_paths: Vec<&str> = Lang::Node.guest_files().iter().map(|(p, _)| *p).collect();
        assert_eq!(
            node_paths,
            vec![
                "/opt/sandbox-code-runner/invoke.mjs",
                "/opt/sandbox-code-runner/iii.mjs",
                "/opt/sandbox-code-runner/run.mjs",
                "/node_modules/iii-sdk/package.json",
                "/node_modules/iii-sdk/dist/index.mjs",
            ]
        );
        let py_paths: Vec<&str> = Lang::Python.guest_files().iter().map(|(p, _)| *p).collect();
        assert_eq!(
            py_paths,
            vec![
                "/opt/sandbox-code-runner/invoke.py",
                "/opt/sandbox-code-runner/sandbox_code_runner_iii.py",
                "/opt/sandbox-code-runner/run.py",
            ]
        );
        assert!(
            !py_paths.iter().any(|p| p.ends_with("/iii.py")),
            "a planted iii.py would shadow the SDK's `iii` package"
        );
    }

    /// The embedded bundle really is the SDK: it must export
    /// `registerWorker`, and the planted manifest must route bare
    /// `import 'iii-sdk'` at it.
    #[test]
    fn sdk_bundle_and_manifest_are_sane() {
        assert!(
            SDK_BUNDLE_MJS.contains("registerWorker"),
            "the guest SDK bundle does not look like iii-sdk"
        );
        assert!(SDK_BUNDLE_MJS.len() > 100_000, "suspiciously small bundle");
        let manifest: serde_json::Value =
            serde_json::from_str(SDK_PACKAGE_JSON).expect("manifest parses");
        assert_eq!(manifest["name"], "iii-sdk");
        assert_eq!(manifest["exports"]["."], "./dist/index.mjs");
        assert!(
            manifest["version"].as_str().is_some_and(|v| !v.is_empty()),
            "the SDK reads its own version from this manifest"
        );
    }

    /// The runner and the run wrapper resolve the iii library RELATIVE to
    /// their own file (`./iii.mjs`, `os.path.dirname(__file__)`), so all
    /// three of a language's files must be planted into ONE directory —
    /// this pins that the path table actually puts them there.
    #[test]
    fn guest_files_of_a_language_share_a_directory() {
        for lang in [Lang::Node, Lang::Python] {
            let dir_of = |p: &str| p.rsplit_once('/').unwrap().0.to_string();
            assert_eq!(dir_of(lang.runner_path()), dir_of(lang.iii_lib_path()));
            assert_eq!(dir_of(lang.runner_path()), dir_of(lang.run_wrapper_path()));
        }
    }

    #[test]
    fn lang_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Lang::Node).unwrap(), r#""node""#);
        assert_eq!(
            serde_json::from_str::<Lang>(r#""python""#).unwrap(),
            Lang::Python
        );
        assert!(serde_json::from_str::<Lang>(r#""ruby""#).is_err());
    }

    #[test]
    fn split_finds_the_result_after_the_sentinel_line() {
        let out = split_sentinel("noise\n\nSENT-1\n{\"a\":1}\n", "SENT-1");
        assert_eq!(out.logs, "noise\n");
        assert_eq!(out.result.as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn split_with_no_prior_output_has_empty_logs() {
        let out = split_sentinel("\nSENT-1\nnull\n", "SENT-1");
        assert_eq!(out.logs, "");
        assert_eq!(out.result.as_deref(), Some("null"));
    }

    /// A crashed interpreter (OOM-killed, bad shebang, …) produces no
    /// sentinel at all; everything is logs and there is no result.
    #[test]
    fn split_without_sentinel_returns_no_result() {
        let out = split_sentinel("Segmentation fault\n", "SENT-1");
        assert_eq!(out.logs, "Segmentation fault\n");
        assert_eq!(out.result, None);
    }

    /// A print that merely CONTAINS the sentinel text mid-line must not
    /// match: the runner emits it as its own line, and that framing is what
    /// the split keys on.
    #[test]
    fn split_requires_the_sentinel_on_its_own_line() {
        let out = split_sentinel("prefix SENT-1 suffix\n\nSENT-1\n42\n", "SENT-1");
        assert_eq!(out.logs, "prefix SENT-1 suffix\n");
        assert_eq!(out.result.as_deref(), Some("42"));
    }

    /// A handler that leaves dangling async work (an uncleared timer, a
    /// live thread) can keep the process alive past the runner's final
    /// write; that late output lands on lines AFTER the result and must not
    /// be folded into it.
    #[test]
    fn split_takes_only_the_first_line_after_the_sentinel() {
        let out = split_sentinel(
            "\nSENT-1\n{\"a\":1}\nlate output from a dangling timer\n",
            "SENT-1",
        );
        assert_eq!(out.result.as_deref(), Some("{\"a\":1}"));
    }

    /// A handler that calls `process.exit(0)` / `os._exit(0)` exits cleanly
    /// but skips the runner's own final write — from `split_sentinel`'s
    /// point of view this is the same "no sentinel found" shape as a crash,
    /// down to the most literal case: no output at all.
    #[test]
    fn split_after_a_clean_self_exit_also_returns_no_result() {
        let out = split_sentinel("", "SENT-1");
        assert_eq!(out.logs, "");
        assert_eq!(out.result, None);
    }
}
