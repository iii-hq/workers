//! The language table and the runner protocol — how code-runner talks to a
//! process inside the guest.
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
            Self::Node => "/opt/code-runner/run.mjs",
            Self::Python => "/opt/code-runner/run.py",
        }
    }
    pub fn runner_source(self) -> &'static str {
        match self {
            Self::Node => RUN_MJS,
            Self::Python => RUN_PY,
        }
    }
}

/// One stdout emit point at the very end of the happy/error path, so a
/// partial write can never leave a sentinel with no result behind it.
/// `JSON.stringify` of a non-serializable value (a function) yields
/// `undefined`, caught explicitly; a circular value throws, caught by the
/// catch. `process.exitCode` instead of `process.exit()` so stdout flushes
/// before the process ends. A malformed envelope is a separate, earlier
/// failure mode: there is no sentinel yet to frame a reply with, so it goes
/// to stderr instead and stdout is never touched.
pub const RUN_MJS: &str = r#"// code-runner runner — planted at runtime creation. Do not edit in place.
// Protocol: argv = [source_path]; stdin = JSON envelope
// {"sentinel": "<uuid>", "payload": <payload>}, consumed before the
// handler's source ever loads. Result = JSON printed after a line holding
// only the sentinel. Exit 0 = result, exit 1 = {"error": "..."}. A
// malformed/missing envelope has no sentinel to frame a reply with: it is
// reported on stderr and the process exits non-zero with no stdout at all.
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

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
      'code-runner runner: malformed envelope on stdin (expected {"sentinel": "...", "payload": ...})\n'
    );
    process.exitCode = 1;
    return;
  }
  const { sentinel, payload } = envelope;

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
  process.stdout.write('\n' + sentinel + '\n' + body + '\n');
  process.exitCode = code;
}

await main();
"#;

/// Same single-emit shape as `RUN_MJS`, and now the same scoping shape too:
/// the envelope, `sentinel`, and `payload` all live inside `main()`, never
/// at module scope. Python always registers the running script as
/// `sys.modules['__main__']`, so a module-level `sentinel = ...` would have
/// been a plain attribute any handler could read off it by name —
/// `getattr(sys.modules['__main__'], 'sentinel', None)` — regardless of how
/// the handler itself was loaded. `main()` RETURNS its exit code instead of
/// calling `sys.exit()` itself, so `sys.exit(main())` at module scope is the
/// only exit call and it stays OUTSIDE every `try`: the malformed-envelope
/// path returns 1 before the result-framing `try` is ever entered, exactly
/// as `RUN_MJS`'s `return` does before its own inner `try`.
pub const RUN_PY: &str = r#"# code-runner runner — planted at runtime creation. Do not edit in place.
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
        assert_eq!(Lang::Node.runner_path(), "/opt/code-runner/run.mjs");
        assert_eq!(Lang::Python.runner_path(), "/opt/code-runner/run.py");
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
