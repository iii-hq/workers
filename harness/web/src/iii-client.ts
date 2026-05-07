// Lazy-initialized singleton iii-browser-sdk client used by the harness UI.
//
// Bootstrap order:
//   1. Fetch `bridge::info` over HTTP via the existing `/bridge/trigger` route
//      to discover the engine WebSocket URL.
//   2. Open a WebSocket via `iii-browser-sdk::registerWorker(url)`.
//   3. Mint a stable `browser_id` for this page; per-browser handlers are
//      registered under `<functionId>::<browserId>` so the harness fanout can
//      target a specific browser when it pushes events.
//
// The `bridge::info` HTTP fetch is deliberately the ONLY HTTP round-trip in
// the production code path. Once `_client` is resolved, every other call
// (`call`, `on`, `dispose`) goes over the single WS connection.

import {
  registerWorker,
  type IIIConnectionState,
  type ISdk,
  type RemoteFunctionHandler,
} from "iii-browser-sdk";

export type { IIIConnectionState };

export interface BridgeInfoResponse {
  ws_path: string;
  protocol: "ws" | "wss";
  engine_url: string;
}

export interface IiiClient {
  browserId: string;
  call<T = unknown>(
    functionId: string,
    payload?: Record<string, unknown>,
  ): Promise<T>;
  on<P = unknown>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
  ): () => void;
  addConnectionStateListener(
    handler: (state: IIIConnectionState) => void,
  ): () => void;
  dispose(): Promise<void>;
}

interface Deps {
  fetchBridgeInfo: () => Promise<BridgeInfoResponse>;
  makeBrowserId: () => string;
  registerWorker: (url: string) => ISdk;
  composeWsUrl: (info: BridgeInfoResponse) => string;
}

let _clientPromise: Promise<IiiClient> | null = null;
let _deps: Deps = defaultDeps();

function defaultDeps(): Deps {
  return {
    fetchBridgeInfo,
    makeBrowserId,
    registerWorker: (url) => registerWorker(url),
    composeWsUrl,
  };
}

/**
 * Get (and lazily create) the shared iii-client for this page.
 */
export function getIiiClient(): Promise<IiiClient> {
  if (!_clientPromise) {
    _clientPromise = bootstrap(_deps);
  }
  return _clientPromise;
}

/**
 * Tear down the shared client. Safe to call multiple times.
 */
export async function disposeIiiClient(): Promise<void> {
  if (!_clientPromise) return;
  const client = await _clientPromise.catch(() => null);
  _clientPromise = null;
  if (client) await client.dispose();
}

/**
 * Test seam: replace the bootstrap dependencies and wipe the cached client.
 * Tests should call `__resetIiiClientForTests()` in afterEach to restore.
 */
export function __setIiiClientDepsForTests(overrides: Partial<Deps>): void {
  _deps = { ..._deps, ...overrides };
  _clientPromise = null;
}

export function __resetIiiClientForTests(): void {
  _deps = defaultDeps();
  _clientPromise = null;
}

async function bootstrap(deps: Deps): Promise<IiiClient> {
  const info = await deps.fetchBridgeInfo();
  const wsUrl = deps.composeWsUrl(info);
  const browserId = deps.makeBrowserId();
  const sdk = deps.registerWorker(wsUrl);

  return wrapSdk(sdk, browserId);
}

function wrapSdk(sdk: ISdk, browserId: string): IiiClient {
  // Track per-functionId unregister fns so dispose() can clean them all up
  // even if individual `on()` callers forgot.
  const handlerUnregisters = new Set<() => void>();

  function call<T>(
    functionId: string,
    payload: Record<string, unknown> = {},
  ): Promise<T> {
    return sdk.trigger<unknown, T>({
      function_id: functionId,
      payload,
    });
  }

  function on<P>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
  ): () => void {
    const id = `${functionId}::${browserId}`;
    // Wrap to satisfy the SDK's RemoteFunctionHandler signature (returns
    // Promise<unknown>). We never want to send a result back.
    const wrapped: RemoteFunctionHandler = async (data: unknown) => {
      await handler(data as P);
      return null;
    };
    const ref = sdk.registerFunction(id, wrapped);
    let active = true;
    const unregister = () => {
      if (!active) return;
      active = false;
      handlerUnregisters.delete(unregister);
      try {
        ref.unregister();
      } catch {
        // SDK already disposed; nothing to do.
      }
    };
    handlerUnregisters.add(unregister);
    return unregister;
  }

  function addConnectionStateListener(
    handler: (state: IIIConnectionState) => void,
  ): () => void {
    return sdk.addConnectionStateListener(handler);
  }

  async function dispose(): Promise<void> {
    for (const unregister of [...handlerUnregisters]) {
      unregister();
    }
    await sdk.shutdown();
  }

  return {
    browserId,
    call,
    on,
    addConnectionStateListener,
    dispose,
  };
}

function composeWsUrl(info: BridgeInfoResponse): string {
  // If the backend gave us a fully-qualified engine URL (single-tenant local
  // default), prefer it. Otherwise compose from `window.location` so
  // reverse-proxy / HTTPS deployments keep working without configuration.
  if (info.engine_url && info.engine_url.length > 0) {
    return info.engine_url;
  }
  const proto = info.protocol === "wss" ? "wss" : "ws";
  if (typeof window === "undefined") {
    throw new Error(
      "iii-client: cannot compose ws url without window.location and engine_url",
    );
  }
  return `${proto}://${window.location.host}${info.ws_path}`;
}

function makeBrowserId(): string {
  // crypto.randomUUID() is available in all evergreen browsers.
  if (
    typeof crypto !== "undefined" &&
    typeof crypto.randomUUID === "function"
  ) {
    return `harness-${crypto.randomUUID()}`;
  }
  // Fallback for environments without crypto.randomUUID (older WebViews).
  const rand = Math.random().toString(36).slice(2, 10);
  return `harness-${Date.now().toString(36)}-${rand}`;
}

async function fetchBridgeInfo(): Promise<BridgeInfoResponse> {
  const res = await fetch("/bridge/trigger", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ function_id: "bridge::info", payload: {} }),
  });
  if (!res.ok) {
    throw new Error(`bridge::info failed: ${res.status} ${res.statusText}`);
  }
  const data = (await res.json()) as Partial<BridgeInfoResponse>;
  if (
    typeof data.ws_path !== "string" ||
    (data.protocol !== "ws" && data.protocol !== "wss") ||
    typeof data.engine_url !== "string"
  ) {
    throw new Error("bridge::info returned malformed response");
  }
  return {
    ws_path: data.ws_path,
    protocol: data.protocol,
    engine_url: data.engine_url,
  };
}
