// One endpoint to reach the entire iii bus. Production callers use the iii
// WebSocket transport (the browser is itself a worker). `bridgeHttp` is kept
// as a backstop for direct curl-style debugging and for the very first
// `bridge::info` round-trip in iii-client.ts; nothing else should reach
// for it.

import { getIiiClient } from "./iii-client";

const BRIDGE_URL = "/bridge/trigger";

export class BridgeError extends Error {
  constructor(
    message: string,
    public readonly functionId: string,
    public readonly status: number,
    public readonly errorId?: string,
  ) {
    super(message);
    this.name = "BridgeError";
  }
}

export async function bridge<T = unknown>(
  functionId: string,
  payload: Record<string, unknown> = {},
): Promise<T> {
  try {
    const client = await getIiiClient();
    return await client.call<T>(functionId, payload);
  } catch (e) {
    if (e instanceof BridgeError) throw e;
    const msg = e instanceof Error ? e.message : String(e);
    throw new BridgeError(msg, functionId, 0);
  }
}

/**
 * HTTP backstop for the bridge. Use only for direct debugging or for the
 * single `bridge::info` round-trip needed to bootstrap the WebSocket
 * connection; production callers should go through {@link bridge}.
 */
export async function bridgeHttp<T = unknown>(
  functionId: string,
  payload: Record<string, unknown> = {},
): Promise<T> {
  const res = await fetch(BRIDGE_URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ function_id: functionId, payload }),
  });

  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    let errorId: string | undefined;
    try {
      const err = (await res.json()) as { error?: string; error_id?: string };
      if (err.error) message = err.error;
      errorId = err.error_id;
    } catch {
      // body not json
    }
    throw new BridgeError(message, functionId, res.status, errorId);
  }

  return (await res.json()) as T;
}
