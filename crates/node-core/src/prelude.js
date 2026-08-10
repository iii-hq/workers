"use strict";
((globalThis) => {
  const handlers = new Map();
  // Trigger-type handlers, keyed by the type id — the guest-side half of
  // `iii.registerTriggerType`. Looked up FRESH on every `invoke()` call, the
  // same as `handlers` is for functions, so a re-registration under the same
  // id (prelude-side `.set()`, no new bus write — see `op_iii_register_trigger_type`)
  // is picked up by whichever engine-side proxy is already routing there.
  const triggerTypes = new Map();
  // The only two method names `invoke()` may ever look up on a trigger-type
  // handler object. `target[method]` would otherwise be an unguarded
  // property lookup on tenant-shaped data driven by an engine-supplied
  // string — not reachable today (the adapter only ever sends one of these
  // two literals), but "constructor"/"valueOf"/etc. would resolve through
  // `Object.prototype` and pass the `typeof fn === "function"` gate below.
  const TRIGGER_TYPE_METHODS = new Set(["registerTrigger", "unregisterTrigger"]);

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
  // against `triggerTypes` (picking one of its two methods by name) when
  // `method` is given — the same envelope-producing path now serves both
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
    const target = method ? triggerTypes.get(id) : handlers.get(id);
    const fn = !method ? target : TRIGGER_TYPE_METHODS.has(method) ? target && target[method] : undefined;
    if (typeof fn !== "function") {
      return thenPromise(resolvePromise(undefined), () => {
        throw definitionError(`no handler registered for ${id}${method ? `.${method}` : ""}`);
      });
    }
    return thenPromise(resolvePromise(JSON.parse(payloadJson)), (p) => fn(p));
  }

  const {
    op_iii_log,
    op_iii_call,
    op_iii_register,
    op_iii_unregister,
    op_iii_register_trigger,
    op_iii_unregister_trigger,
    op_iii_register_trigger_type,
    op_iii_unregister_trigger_type,
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

  // The SDK's request-object form: `iii.trigger({function_id, payload})`.
  // `action` routes the call (omit for request/response); `timeout` is a
  // per-call millisecond budget clamped by the host.
  async function trigger(request) {
    if (request === null || typeof request !== "object") {
      throw new TypeError("trigger(request): request must be an object");
    }
    const { function_id, payload, action, timeout } = request;
    if (typeof function_id !== "string" || function_id.length === 0) {
      throw new TypeError("trigger(request): function_id must be a non-empty string");
    }
    if (action !== undefined && typeof action !== "string") {
      throw new TypeError("trigger(request): action must be a string when given");
    }
    if (timeout !== undefined && typeof timeout !== "number") {
      throw new TypeError("trigger(request): timeout must be a number when given");
    }
    // `stringify` is the pre-captured `JSON.stringify` — see the block above
    // where it is captured, before any tenant code can run. This value
    // crosses into Rust, so the global (tenant-replaceable) one is never
    // safe to use here.
    const text = await op_iii_call(
      function_id,
      stringify(payload === undefined ? {} : payload),
      action === undefined ? "" : action,
      timeout === undefined ? 0 : timeout,
    );
    return JSON.parse(text);
  }

  // SDK signature: (functionId, handler, options?). `options.description` is
  // what `engine::functions::info` shows a caller; omitted sends "", which
  // Rust reads as "not supplied" and replaces with a generic default.
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
    op_iii_register(functionId, description === undefined ? "" : description);
    handlers.set(functionId, handler);
    return {
      unregister() {
        op_iii_unregister(functionId);
        handlers.delete(functionId);
      },
    };
  }

  // Bind a trigger to a registered function. Returns a ref whose
  // `unregister()` removes it — the SDK shape, mirroring `registerFunction`.
  async function registerTrigger(input) {
    if (input === null || typeof input !== "object") {
      throw new TypeError("registerTrigger(input): input must be an object");
    }
    if (typeof input.type !== "string" || input.type.length === 0) {
      throw new TypeError("registerTrigger(input): input.type must be a non-empty string");
    }
    if (typeof input.function_id !== "string" || input.function_id.length === 0) {
      throw new TypeError("registerTrigger(input): input.function_id must be a non-empty string");
    }
    // Pre-captured `stringify` — see the block above where it, `resolvePromise`,
    // and `thenPromise` are captured before any tenant code can run. This
    // value crosses into Rust, so the global (tenant-replaceable) one is
    // never safe to use here.
    const triggerId = await op_iii_register_trigger(stringify(input));
    return {
      unregister() {
        op_iii_unregister_trigger(triggerId);
      },
    };
  }

  // Publish a trigger TYPE. `handler` is an object the ENGINE calls back
  // into: `handler.registerTrigger(config)` / `handler.unregisterTrigger(config)`
  // run whenever a trigger of this type is registered or removed anywhere on
  // the bus. This is the one direction that isn't isolate -> host: the proxy
  // built on the Rust side (`op_iii_register_trigger_type`) dispatches back
  // in through `__iii.invoke`, the same path a registered function's handler
  // already uses.
  function registerTriggerType(triggerType, handler) {
    if (triggerType === null || typeof triggerType !== "object") {
      throw new TypeError("registerTriggerType(triggerType, handler): triggerType must be an object");
    }
    if (typeof triggerType.id !== "string" || triggerType.id.length === 0) {
      throw new TypeError("registerTriggerType(triggerType, handler): triggerType.id must be a non-empty string");
    }
    if (handler === null || typeof handler !== "object") {
      throw new TypeError("registerTriggerType(triggerType, handler): handler must be an object");
    }
    if (typeof handler.registerTrigger !== "function" || typeof handler.unregisterTrigger !== "function") {
      throw new TypeError(
        "registerTriggerType(triggerType, handler): handler must have registerTrigger and unregisterTrigger methods",
      );
    }
    op_iii_register_trigger_type(
      triggerType.id,
      typeof triggerType.description === "string" ? triggerType.description : "",
    );
    triggerTypes.set(triggerType.id, handler);
    return {
      unregister() {
        op_iii_unregister_trigger_type(triggerType.id);
        triggerTypes.delete(triggerType.id);
      },
    };
  }

  function unregisterTriggerType(triggerType) {
    const id = typeof triggerType === "string" ? triggerType : triggerType && triggerType.id;
    if (typeof id !== "string" || id.length === 0) {
      throw new TypeError("unregisterTriggerType(triggerType): pass the type id or the type object");
    }
    op_iii_unregister_trigger_type(id);
    triggerTypes.delete(id);
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
    "[iii: code-runner host client. e.g. await iii.trigger({ function_id: 'worker::fn', " +
    "payload: {} }); registerFunction(id, handler, opts?); iii.namespace is the prefix this " +
    "runtime may register under. iii.files is a private scratch directory " +
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

  globalThis.iii = { trigger, registerFunction, registerTrigger, registerTriggerType, unregisterTriggerType, shutdown, files };

  // Non-enumerable: a `toString` STRING key set via an object literal or a
  // bare `defineProperty` call defaults to enumerable and would show up in
  // `Object.keys(iii)` — the pinned golden counts six SDK methods plus
  // `namespace`, not this hint, as the guest-visible surface. The Symbol key
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
