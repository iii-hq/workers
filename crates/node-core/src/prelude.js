"use strict";
((globalThis) => {
  const handlers = new Map();
  // Observer hooks, keyed by function id: `{ observerAdded, observerRemoved }`
  // — the guest-side half of `iii.onObserverAdded` / `iii.onObserverRemoved`.
  // Looked up FRESH on every `invoke()` call, the same as `handlers` is for
  // functions, so setting a hook again under the same id (prelude-side
  // assignment, no new engine write — see `op_iii_set_observer_hooks`) is
  // picked up by the engine-side proxy already routing there.
  const observerHooks = new Map();
  // The only two method names `invoke()` may ever look up on a hooks
  // object. `hooks[method]` would otherwise be an unguarded property lookup
  // driven by an engine-supplied string — not reachable today (the proxy
  // only ever sends one of these two literals), but "constructor"/"valueOf"/
  // etc. would resolve through `Object.prototype` and pass the
  // `typeof fn === "function"` gate below.
  const HOOK_METHODS = new Set(["observerAdded", "observerRemoved"]);

  function formatError(e) {
    if (e instanceof Error) {
      return e.stack ? String(e.stack) : `${e.name}: ${e.message}`;
    }
    try {
      return String(e);
    } catch {
      return "unstringifiable thrown value";
    }
  }

  // `formatError` without the stack. A handler that fails at DEFINITION time
  // has a stack made entirely of prelude internals — `Array.map`, the
  // generated `[code-runner:eval]` wrapper, `toHandler` itself. None of it is
  // the caller's code, none of it is actionable, and it buries the one line
  // that is. Runtime errors keep their stack: that one points at tenant code.
  function formatCause(e) {
    if (e instanceof Error) {
      return `${e.name}: ${e.message}`;
    }
    try {
      return String(e);
    } catch {
      return "unstringifiable thrown value";
    }
  }

  // A definition error carries its message and nothing else, for the same
  // reason. Overwriting `stack` is what makes `settle`'s `formatError` — which
  // prefers `e.stack` — report just the message.
  function definitionError(message) {
    const e = new Error(message);
    e.stack = `Error: ${message}`;
    return e;
  }

  // The envelope builders, captured before any tenant code can run.
  //
  // `settle`'s return value IS the eval's result as Rust reads it, so anything
  // it resolves off the global at call time is a way to forge that envelope.
  // Overwriting `JSON.stringify` turned a thrown error into a reported
  // success; replacing `globalThis.Promise` — or `Promise.prototype.then` —
  // let `.then()` return a hand-written envelope string instead of a promise.
  // `__iii` being frozen does not help: these are the primitives `settle`
  // itself is built from.
  const stringify = JSON.stringify;
  const resolvePromise = Promise.resolve.bind(Promise);
  const thenPromise = Function.prototype.call.bind(Promise.prototype.then);

  // Every value leaving the isolate goes through here: one promise in, one
  // promise of `{"ok":...}` / `{"err":"..."}` JSON text out.
  //
  // Values JSON cannot represent come back as ok:null rather than turning a
  // successful run into an error. Two different failures produce that:
  // `stringify` THROWS on a cycle or a BigInt, but for a function, a symbol,
  // or an object whose `toJSON` returns undefined it returns `undefined`
  // instead — and stringifying `{ok: v}` in one step hides that, because a
  // dropped property yields `{}`, an envelope carrying neither key. So
  // stringify the VALUE alone, where unrepresentable is observable, and build
  // the envelope around the text.
  function settle(promise) {
    return thenPromise(
      resolvePromise(promise),
      (v) => {
        try {
          const encoded = v === undefined ? "null" : stringify(v);
          return encoded === undefined ? '{"ok":null}' : `{"ok":${encoded}}`;
        } catch {
          return '{"ok":null}';
        }
      },
      (e) => `{"err":${stringify(formatError(e))}}`,
    );
  }

  // `id` resolves against `handlers` for a plain function invocation, or
  // against `observerHooks` (picking one of its two hooks by name) when
  // `method` is given — the same envelope-producing path serves both
  // directions the engine calls INTO the isolate.
  //
  // Returns a promise/value for `wrap_invoke`'s OWN `settle(...)` to consume
  // — it must NOT call `settle` itself. `wrap_invoke` always generates
  // `globalThis.__iii.settle(globalThis.__iii.invoke(...))`; settling here
  // too would hand that outer `settle` an already-encoded envelope STRING to
  // re-stringify, corrupting every response (`{"ok":"{\"ok\":...}"}`).
  //
  // The missing-handler case is raised through a promise rejection, not a
  // synchronous throw, so a handler that itself throws synchronously and a
  // missing handler both fail identically for whichever caller is awaiting
  // this — `wrap_invoke`'s bare (non-async-IIFE) call site would otherwise
  // let a synchronous throw here escape `execute_script` before `settle` is
  // ever reached.
  //
  // Uses the pre-captured `thenPromise`, NOT `.then()`/`Promise.prototype.
  // then` directly: this is the ENGINE calling INTO the isolate through a
  // path `settle` cannot see, so a tenant that overwrote `Promise.prototype.
  // then` before this runs would otherwise have its replacement invoked
  // here, forging whatever this resolves to (e.g. turning a handler that
  // throws into a reported success) — the exact class of hijack `stringify`/
  // `resolvePromise`/`thenPromise` are captured above to prevent, and the
  // reason those three are named specifically in that block's comment.
  function invoke(id, payloadJson, method) {
    if (method) return invokeHook(id, payloadJson, method);
    const fn = handlers.get(id);
    if (typeof fn !== "function") {
      return thenPromise(resolvePromise(undefined), () => {
        throw definitionError(`no handler registered for ${id}`);
      });
    }
    return thenPromise(resolvePromise(JSON.parse(payloadJson)), (p) => fn(p));
  }

  // An observer hook, called with the observer. The engine-side proxy is set
  // for BOTH hooks whenever the guest set either one, so a callback with no
  // guest hook behind it is normal: it resolves the way iii does by default
  // — accept the observer, nothing to tear down.
  //
  // A hook that throws RESOLVES to `{ rejected: { code, message } }` instead
  // of rejecting. The thrown value's `code` is what the observer's creator
  // reads back from `ready()`, and only JavaScript can read it: an error
  // crosses `settle` as one formatted string. The host reads this shape with
  // `engine::hook_outcome`.
  function invokeHook(id, payloadJson, method) {
    if (!HOOK_METHODS.has(method)) {
      return thenPromise(resolvePromise(undefined), () => {
        throw definitionError(`no observer hook ${method}`);
      });
    }
    const hooks = observerHooks.get(id);
    const fn = hooks ? hooks[method] : undefined;
    if (typeof fn !== "function") return resolvePromise(null);
    return thenPromise(
      thenPromise(resolvePromise(JSON.parse(payloadJson)), (observer) => fn(observer)),
      () => null,
      (e) => ({ rejected: { code: rejectionCode(e), message: rejectionMessage(e) } }),
    );
  }

  // The `code` a hook's thrown value carries, or null. Read defensively: the
  // value is tenant-made, and a throwing `code` getter must not turn a
  // rejection into a hook failure with a different message.
  function rejectionCode(e) {
    try {
      const code = e !== null && (typeof e === "object" || typeof e === "function") ? e.code : undefined;
      return typeof code === "string" ? code : null;
    } catch {
      return null;
    }
  }

  function rejectionMessage(e) {
    try {
      return e instanceof Error ? String(e.message) : String(e);
    } catch {
      return "unstringifiable thrown value";
    }
  }

  const {
    op_iii_log,
    op_iii_call,
    op_iii_call_observer,
    op_iii_register,
    op_iii_unregister,
    op_iii_set_observer_hooks,
    op_iii_clear_observer_hooks,
    op_iii_observe,
    op_iii_dispose_observer,
    op_iii_fs_write,
    op_iii_fs_read,
    op_iii_fs_list,
    op_iii_fs_remove,
  } = Deno.core.ops;

  // V8's own UTF-8 codec, captured here for the same reason the ops are:
  // `Deno` is deleted before tenant code runs. Hand-rolling a UTF-8 encoder
  // in this file would be the flimsy-algorithm trap — surrogate pairs are
  // exactly where it goes wrong.
  const { encode: utf8Encode, decode: utf8Decode } = Deno.core;

  function formatArg(a) {
    if (typeof a === "string") return a;
    if (a instanceof Error) return a.stack ? String(a.stack) : String(a);
    try {
      const s = JSON.stringify(a);
      return s === undefined ? String(a) : s;
    } catch {
      return String(a);
    }
  }

  function emit(level) {
    return (...args) => op_iii_log(level, args.map(formatArg).join(" "));
  }

  // Replaces deno_core's console, which writes to the process stdout. Ours
  // routes into the eval's response instead.
  globalThis.console = {
    log: emit("log"),
    info: emit("log"),
    debug: emit("log"),
    warn: emit("warn"),
    error: emit("error"),
    trace: emit("warn"),
  };

  // Why `call` refuses a call through an observer that names metadata: its
  // target's metadata argument is the observer's registration metadata
  // (decision 53). The same message as every iii SDK.
  const OBSERVER_CALL_METADATA =
    "a call through an observer takes no metadata; its target receives the observer's metadata";

  // The SDK's request-object form: `iii.call({ function_id, payload })`, or
  // `iii.call({ observer, payload })` to run an observer's own target.
  // `action` routes the call (omit for request/response; `{ type: 'void' }`
  // is fire-and-forget); `timeout_ms` is a per-call millisecond budget
  // clamped by the host. `iii.trigger` is this same function.
  //
  // `metadata: null` is no metadata on every call (decision 54). A call
  // through an observer that names any other metadata is refused before
  // anything is sent, as the SDKs refuse it. A guest's call of a function
  // carries no metadata: the ops take none.
  async function call(request) {
    if (request === null || typeof request !== "object") {
      throw new TypeError("call(request): request must be an object");
    }
    const { function_id, observer, payload, action, timeout_ms, metadata } = request;
    if (observer !== undefined) {
      if (function_id !== undefined) {
        throw new TypeError("call(request): pass function_id or observer, not both");
      }
      if (observer === null || typeof observer !== "object" || typeof observer.id !== "string" || observer.id.length === 0) {
        throw new TypeError("call(request): observer must be an observer an onObserverAdded hook received");
      }
      if (metadata !== undefined && metadata !== null) {
        throw new Error(OBSERVER_CALL_METADATA);
      }
    } else if (typeof function_id !== "string" || function_id.length === 0) {
      throw new TypeError("call(request): function_id must be a non-empty string");
    }
    if (action !== undefined && (action === null || typeof action !== "object" || typeof action.type !== "string")) {
      throw new TypeError("call(request): action must be an object such as { type: 'void' } when given");
    }
    if (timeout_ms !== undefined && typeof timeout_ms !== "number") {
      throw new TypeError("call(request): timeout_ms must be a number when given");
    }
    // `stringify` is the pre-captured `JSON.stringify` — see the block above
    // where it is captured, before any tenant code can run. These values
    // cross into Rust, so the global (tenant-replaceable) one is never safe
    // to use here.
    const body = stringify(payload === undefined ? {} : payload);
    const routed = action === undefined ? "" : stringify(action);
    const timeout = timeout_ms === undefined ? 0 : timeout_ms;
    const text =
      observer !== undefined
        ? await op_iii_call_observer(observer.id, body, routed, timeout)
        : await op_iii_call(function_id, body, routed, timeout);
    return JSON.parse(text);
  }

  // SDK signature: (functionId, handler, options?). `options.description` is
  // what `engine::functions::info` shows a caller; omitted sends "", which
  // Rust reads as "not supplied" and replaces with a generic default.
  // `options.args_format` is the JSON Schema observers' `args` are validated
  // against, and `options.private` limits calls to this worker while anyone
  // may still observe the function.
  function registerFunction(functionId, handler, options) {
    if (typeof functionId !== "string" || functionId.length === 0) {
      throw new TypeError(
        "registerFunction(functionId, handler, options?): functionId must be a non-empty string",
      );
    }
    if (typeof handler !== "function") {
      throw new TypeError(
        "registerFunction(functionId, handler, options?): handler must be a function — an " +
          "HttpInvocationConfig object registers an engine-side HTTP binding with no isolate " +
          "involved, which code-runner does not publish; call engine::functions::register for that",
      );
    }
    if (options !== undefined && (options === null || typeof options !== "object")) {
      throw new TypeError(
        "registerFunction(functionId, handler, options?): options must be an object when given",
      );
    }
    const description = options === undefined ? undefined : options.description;
    if (description !== undefined && typeof description !== "string") {
      throw new TypeError(
        "registerFunction(functionId, handler, options?): options.description must be a string",
      );
    }
    const isPrivate = options === undefined ? undefined : options.private;
    if (isPrivate !== undefined && typeof isPrivate !== "boolean") {
      throw new TypeError(
        "registerFunction(functionId, handler, options?): options.private must be a boolean",
      );
    }
    // `request_format`/`response_format` are JSON Schemas for the catalog —
    // engine::functions::info shows them to the next caller in place of
    // "any" — and `args_format` is the schema the engine holds observers'
    // args to. Checked here as plain objects for a good error early; the op
    // re-validates either way (this check runs on tenant-replaceable
    // builtins, the op's does not).
    const formats = [];
    for (const name of ["request_format", "response_format", "args_format"]) {
      const value = options === undefined ? undefined : options[name];
      if (value !== undefined && (value === null || typeof value !== "object" || Array.isArray(value))) {
        throw new TypeError(
          `registerFunction(functionId, handler, options?): options.${name} must be a plain object holding a JSON Schema`,
        );
      }
      const encoded = value === undefined ? "" : stringify(value);
      if (typeof encoded !== "string") {
        throw new TypeError(
          `registerFunction(functionId, handler, options?): options.${name} did not survive JSON encoding`,
        );
      }
      formats.push(encoded);
    }
    op_iii_register(
      functionId,
      description === undefined ? "" : description,
      formats[0],
      formats[1],
      formats[2],
      isPrivate === true,
    );
    handlers.set(functionId, handler);
    return {
      unregister() {
        op_iii_unregister(functionId);
        handlers.delete(functionId);
      },
    };
  }

  // Set (or, with `null`, remove) one of a function's two observer hooks.
  // The ENGINE calls back into these: `onObserverAdded`'s hook runs before
  // an observer of `functionId` is accepted, and throwing rejects it (with
  // the thrown error's `code`, or OBSERVE_REJECTED); `onObserverRemoved`'s
  // runs once an accepted one leaves. The proxy built on the Rust side
  // (`op_iii_set_observer_hooks`) dispatches back in through `__iii.invoke`,
  // the same path a registered function's handler uses.
  //
  // The op runs before the map changes, so a refusal (another namespace, an
  // id another runtime holds) leaves this runtime's hooks as they were.
  function setObserverHook(name, method, functionId, hook) {
    if (typeof functionId !== "string" || functionId.length === 0) {
      throw new TypeError(`${name}(functionId, hook): functionId must be a non-empty string`);
    }
    if (hook !== null && typeof hook !== "function") {
      throw new TypeError(`${name}(functionId, hook): hook must be a function, or null to remove it`);
    }
    const hooks = observerHooks.get(functionId);
    if (hook === null) {
      if (hooks === undefined) return;
      const other = method === "observerAdded" ? "observerRemoved" : "observerAdded";
      if (hooks[other] === undefined) {
        op_iii_clear_observer_hooks(functionId);
        observerHooks.delete(functionId);
      } else {
        hooks[method] = undefined;
      }
      return;
    }
    if (hooks === undefined) {
      op_iii_set_observer_hooks(functionId);
      const fresh = { observerAdded: undefined, observerRemoved: undefined };
      fresh[method] = hook;
      observerHooks.set(functionId, fresh);
    } else {
      hooks[method] = hook;
    }
  }

  function onObserverAdded(functionId, hook) {
    setObserverHook("onObserverAdded", "observerAdded", functionId, hook);
  }

  function onObserverRemoved(functionId, hook) {
    setObserverHook("onObserverRemoved", "observerRemoved", functionId, hook);
  }

  // Observe `source` (a function id, or `{ function_id, namespace?, args?,
  // filters?, deliver? }`) with `target`, a registered function. Resolves to
  // a handle whose `dispose()` removes the observer; the observer itself
  // stays pending until its function is registered and accepts it.
  async function observe(source, target, options) {
    const normalized = typeof source === "string" ? { function_id: source } : source;
    if (normalized === null || typeof normalized !== "object" || Array.isArray(normalized)) {
      throw new TypeError(
        "observe(source, target, options?): source must be a function id or an object with function_id",
      );
    }
    if (typeof normalized.function_id !== "string" || normalized.function_id.length === 0) {
      throw new TypeError("observe(source, target, options?): source.function_id must be a non-empty string");
    }
    if (typeof target !== "string" || target.length === 0) {
      throw new TypeError("observe(source, target, options?): target must be a non-empty function id");
    }
    if (options !== undefined && (options === null || typeof options !== "object" || Array.isArray(options))) {
      throw new TypeError("observe(source, target, options?): options must be an object when given");
    }
    // Pre-captured `stringify`, for the same reason as `call`'s.
    const request = stringify({ source: normalized, target, options: options === undefined ? {} : options });
    if (typeof request !== "string") {
      throw new TypeError("observe(source, target, options?): the request did not survive JSON encoding");
    }
    const id = await op_iii_observe(request);
    return {
      id,
      dispose() {
        return op_iii_dispose_observer(id);
      },
    };
  }

  // A real property, not a missing one: the iii-sdk client this shim mirrors
  // owns the connection it closes, but here the connection belongs to the
  // WORKER, shared by every namespace runtime on it — severing it would be a
  // cross-tenant denial of service. Throwing names the actual way to dispose
  // a runtime, instead of leaving tenant code to puzzle out
  // "undefined is not a function".
  function shutdown() {
    throw new Error(
      "iii.shutdown() is not available in a code-runner runtime — the engine " +
        "connection belongs to the worker and is shared by every runtime. Use " +
        "code-runner::teardown to dispose this runtime.",
    );
  }

  // An agent's first move against an unknown global is printing it or
  // listing its keys; an opaque `{}` there previously cost a live session
  // (in the sibling sandbox-code-runner worker) six blind runs before anyone
  // worked out what the global even was. `String(iii)` and Node's inspect
  // both answer with this instead.
  const HINT =
    "[iii: code-runner host client. e.g. await iii.call({ function_id: 'worker::fn', " +
    "payload: {} }); registerFunction(id, handler, opts?); observe(source, target, opts?); " +
    "onObserverAdded(id, hook) / onObserverRemoved(id, hook); iii.namespace is the prefix " +
    "this runtime may register under. iii.files is a private scratch directory " +
    "(write/read/readText/list/remove) that lives as long as this runtime. " +
    "docs <https://iii.dev/docs/reference/sdk-node>]";

  // This runtime's private scratch directory. Flat — a name is one file, never
  // a path — and bounded by the operator's `scratch_mb`/`scratch_files`. The
  // directory dies with the runtime, so it is what makes a kept runtime's
  // files outlive a single eval and nothing more.
  //
  // Frozen HERE, separately from the `Object.freeze(globalThis.iii)` that
  // runtime.rs applies: that freeze is SHALLOW, so it pins the `files`
  // property but not the object behind it. Two freezes, two different jobs.
  //
  // Honest about the stakes, so nobody overclaims in review: unlike `settle`,
  // nothing the HOST reads comes back through `iii.files`, so a tenant that
  // forged a method here would only be lying to itself. This freeze is for
  // consistency with the rule, not because it stops an exploit.
  function requireName(name, fn) {
    if (typeof name !== "string") {
      throw new TypeError(`iii.files.${fn}: name must be a string`);
    }
    return name;
  }

  const files = Object.freeze({
    write(name, contents) {
      requireName(name, "write");
      let bytes;
      if (typeof contents === "string") bytes = utf8Encode(contents);
      else if (contents instanceof Uint8Array) bytes = contents;
      else throw new TypeError("iii.files.write: contents must be a string or Uint8Array");
      op_iii_fs_write(name, bytes);
    },
    read(name) {
      return op_iii_fs_read(requireName(name, "read"));
    },
    readText(name) {
      return utf8Decode(op_iii_fs_read(requireName(name, "readText")));
    },
    list() {
      return JSON.parse(op_iii_fs_list());
    },
    remove(name) {
      op_iii_fs_remove(requireName(name, "remove"));
    },
  });

  globalThis.iii = {
    call,
    trigger: call,
    registerFunction,
    onObserverAdded,
    onObserverRemoved,
    observe,
    shutdown,
    files,
  };

  // Non-enumerable: a `toString` STRING key set via an object literal or a
  // bare `defineProperty` call defaults to enumerable and would show up in
  // `Object.keys(iii)` — the pinned golden counts the SDK methods, `files`
  // and `namespace`, not this hint, as the guest-visible surface. The Symbol key
  // is never enumerable regardless (`Object.keys`/`for...in` only ever see
  // string keys), so this only matters for `toString`.
  Object.defineProperties(globalThis.iii, {
    toString: { value: () => HINT, enumerable: false },
    [Symbol.for("nodejs.util.inspect.custom")]: { value: () => HINT, enumerable: false },
  });

  // `namespace` (published by runtime.rs, see `[node-engine:namespace]`) and
  // `Object.freeze` both apply once this script returns — see that call site
  // for why freezing has to wait that long, and why waiting is still safe.

  // Resolve a caller-supplied handler string to a function.
  //
  // Expression first. The body fallback is gated on SyntaxError ALONE and
  // that is load-bearing: almost any string is a valid function body, so an
  // unconditional fallback can never fail — "payload.n * 2", the most natural
  // short handler anyone would write, would become a live function returning
  // undefined forever, which the envelope reports as a deliberate null.
  //
  // Returns `{ fn, form }`, not a bare function. `form` names which branch
  // won — only this code knows, and re-deriving it in Rust would need a
  // JavaScript parser. It was read by `node-engine::eject`, which wrote these
  // handlers to disk and had to emit the shape the caller actually got; that
  // function is gone, so `wrap_register` now just passes `form` back as part
  // of its eval result and nothing consumes it.
  function toHandler(id, src) {
    // Construct and invoke as SEPARATE steps. Only construction can raise the
    // SyntaxError that means "this is not an expression"; only invocation can
    // raise a tenant error. Splitting them keeps the not-a-function throw
    // below outside any catch, so it is never wrapped twice, and it keeps an
    // invocation error from ever reaching the body-form fallback.
    //
    // Every message goes through `formatCause`: a handler is free to
    // `throw null`, and reading `.message` off that would raise an unrelated
    // TypeError that loses the handler id entirely.
    let make;
    try {
      make = new Function("return (" + src + ")");
    } catch (exprErr) {
      if (!(exprErr instanceof SyntaxError)) {
        throw definitionError(`handler for ${id}: ${formatCause(exprErr)}`);
      }
      // Not an expression at all — try it as a function body.
      try {
        return { fn: new Function("payload", src), form: "body" };
      } catch (bodyErr) {
        // Both parser messages quote tokens from the `new Function`
        // wrappers, not from the caller's source, so neither is actionable on
        // its own. Lead with the shape that actually works: `toHandler`'s
        // only caller now is `wrap_register` (protocol.rs), so this is
        // reached with a genuine syntax error in `register_function`'s
        // `source` — the retired bare-expression/function-body forms this
        // message used to recommend are exactly what that contract no
        // longer accepts.
        throw definitionError(
          `handler for ${id} does not parse: source must define handler(payload) — write, ` +
            `for example, "export function handler(p) { ... }" or ` +
            `"const handler = (p) => { ... }". Note that "export" is only recognised as ` +
            `the first token of source: a statement ahead of it, a second export further ` +
            `down, or CommonJS "exports.handler = ..." all leave an export the evaluated ` +
            `source cannot parse — drop the keyword, or put the handler first. ` +
            `(as an expression: ${formatCause(exprErr)}; as a body: ${formatCause(bodyErr)})`,
        );
      }
    }

    let fn;
    try {
      fn = make();
    } catch (e) {
      // NOT a fallback to the body form: an expression that parses but throws
      // was meant as an expression, and "payload.n * 2" silently becoming a
      // null-returning handler is the exact failure this gate prevents.
      //
      // A ReferenceError here is almost always the same confusion: the caller
      // wrote a bare expression ("payload.n * 2") instead of DEFINING
      // handler, so evaluating it looked up `payload` before any payload
      // exists. Naming the contract is what keeps that from being a wall.
      //
      // `src` is deliberately NOT interpolated. This hint used to quote it,
      // from when callers passed their handler text straight in;
      // `toHandler`'s only caller is now `wrap_register` (protocol.rs), so
      // `src` is the GENERATED wrapper — the hint dumped ~1,400 characters of
      // machine-written code at the caller, and recommended the bare-
      // expression and function-body forms this contract retired. (It also
      // made an E2E assertion vacuous: the case checking this message names
      // "source must define handler(payload)" was matching that phrase
      // inside the dumped wrapper, not in any message.)
      const hint =
        e instanceof ReferenceError
          ? `. source must define handler(payload) — write, for example, ` +
            `"export function handler(p) { ... }" or "const handler = (p) => { ... }"`
          : "";
      throw definitionError(
        `handler for ${id} threw while being defined: ${formatCause(e)}${hint}`,
      );
    }
    if (typeof fn !== "function") {
      const kind = typeof fn;
      const article = "aeiou".includes(kind[0]) ? "an" : "a";
      throw definitionError(
        `handler for ${id} evaluated to ${article} ${kind}, not a function; ` +
          `write an expression such as "(p) => …", or a body containing "return"`,
      );
    }
    return { fn, form: "expression" };
  }

  // `handlers` is deliberately NOT exposed: `wrap_eval` and `wrap_invoke`
  // resolve `__iii` by name at call time, so tenant code that can reach it
  // could forge its own result envelope.
  //
  // `Object.freeze` alone is NOT enough — it protects the object, not the
  // binding, so `globalThis.__iii = {...}` would replace it wholesale.
  // `defineProperty` defaults to non-writable AND non-configurable, which
  // closes both the property-assignment and the rebinding forms.
  Object.defineProperty(globalThis, "__iii", {
    value: Object.freeze({ settle, invoke, toHandler }),
  });

  // Remove deno_core's builtin surface. This is the containment boundary:
  // `Deno.core.ops` carries `op_panic` — literally `panic!(...)`, which
  // unwinds out of a V8 callback and kills this worker thread, i.e. one
  // tenant can take down every other tenant's runtime — and `op_print`,
  // which writes straight to the worker's stdout. Neither is reachable
  // through the two `iii` ops that are supposed to be the entire escape
  // surface.
  //
  // deno_core supports this: 01_core.js hands Rust a captured copy of
  // `__bootstrap` specifically so extension scripts keep working "when the
  // live `__bootstrap` has already been deleted (i.e. after runtime
  // bootstrap)". Anything this prelude needs from `Deno.core` must be
  // destructured into a local ABOVE, before these deletes.
  delete globalThis.Deno;
  delete globalThis.__bootstrap;

  // WebAssembly is not part of what this worker offers — the README promises
  // "plain modern JavaScript plus the `iii` global" — and it is the one
  // remaining way to take memory the capped ArrayBuffer allocator cannot see:
  // V8 backs `WebAssembly.Memory` with its own reservation, not that
  // allocator, so a tenant could hold hundreds of megabytes under a runtime
  // configured for far less. Removing it also drops a JIT surface nothing
  // here needs.
  delete globalThis.WebAssembly;
})(globalThis);
