// One endpoint to reach the entire iii bus. Production callers use the iii
// WebSocket transport (the browser is itself a worker). `bridgeHttp` is kept
// as a backstop for direct curl-style debugging and for the very first
// `harness::info` round-trip in iii-client.ts; nothing else should reach
// for it.

import { getIiiClient } from "./iii-client";

const BRIDGE_URL = "/harness/call";

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
    throw toBridgeError(e, functionId);
  }
}

/**
 * Normalize anything thrown by the iii-sdk's `trigger()` into a `BridgeError`
 * with a human-readable `message`.
 *
 * The iii-sdk delivers engine errors via `Promise.reject(error)` where
 * `error` is the raw JSON value off the wire — usually a plain object like
 * `{ error_code, message, error_id }` or `{ error }`, NOT an `Error`
 * instance.  Previously this path collapsed to `String(e)` which produced
 * the infamous `"[object Object]"` in the UI (see harness AuthPanel error
 * pill).  Now we walk common engine-error shapes and fall back to
 * `JSON.stringify` so the actual server message reaches the user.
 */
export function toBridgeError(e: unknown, functionId: string): BridgeError {
  if (e instanceof Error) {
    return new BridgeError(e.message, functionId, 0);
  }
  if (e !== null && typeof e === "object") {
    const obj = e as Record<string, unknown>;
    const message =
      pickString(obj.message) ??
      pickString(obj.error) ??
      pickString(obj.error_message) ??
      pickString(obj.description) ??
      safeJson(obj);
    const errorId = pickString(obj.error_id);
    return new BridgeError(message, functionId, 0, errorId);
  }
  return new BridgeError(String(e), functionId, 0);
}

function pickString(v: unknown): string | undefined {
  return typeof v === "string" && v.length > 0 ? v : undefined;
}

function safeJson(v: unknown): string {
  try {
    return JSON.stringify(v);
  } catch {
    return "unknown error (unserializable)";
  }
}

/**
 * HTTP backstop for the bridge. Use only for direct debugging or for the
 * single `harness::info` round-trip needed to bootstrap the WebSocket
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
