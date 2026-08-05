// sandbox-code-runner guest iii library — planted at runtime creation. Do not edit
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
    "[iii: lazy iii-sdk client — connects on first use. e.g. await iii.trigger({ function_id: 'worker::fn', payload: {} }); registerFunction(id, handler, opts?); docs: https://iii.dev/docs/reference/sdk-node]";

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
