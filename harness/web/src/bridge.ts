// One endpoint to reach the entire iii bus.
// The harness worker registers POST /bridge/trigger which forwards
// {function_id, payload} to iii.trigger and returns the result.

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
