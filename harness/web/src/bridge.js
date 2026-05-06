// One endpoint to reach the entire iii bus.
// The harness worker registers POST /bridge/trigger which forwards
// {function_id, payload} to iii.trigger and returns the result.
const BRIDGE_URL = "/bridge/trigger";
export class BridgeError extends Error {
    functionId;
    status;
    errorId;
    constructor(message, functionId, status, errorId) {
        super(message);
        this.functionId = functionId;
        this.status = status;
        this.errorId = errorId;
        this.name = "BridgeError";
    }
}
export async function bridge(functionId, payload = {}) {
    const res = await fetch(BRIDGE_URL, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ function_id: functionId, payload }),
    });
    if (!res.ok) {
        let message = `${res.status} ${res.statusText}`;
        let errorId;
        try {
            const err = (await res.json());
            if (err.error)
                message = err.error;
            errorId = err.error_id;
        }
        catch {
            // body not json
        }
        throw new BridgeError(message, functionId, res.status, errorId);
    }
    return (await res.json());
}
