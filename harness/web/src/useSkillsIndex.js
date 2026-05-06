import { useCallback, useEffect, useRef, useState } from "react";
import { bridge, BridgeError } from "./bridge";
const INDEX_URI = "iii://skills";
/**
 * Fetch the auto-rendered `iii://skills` index once at mount and cache it
 * in component state. Bodies of individual worker skills are loaded by the
 * agent on demand via `skill::fetch`; this hook only owns the directory.
 *
 * Failure is strictly non-blocking — `error` is exposed for diagnostics but
 * the consumer must NOT gate chat rendering on it. The agent's fallback path
 * (calling `skill::fetch` with `iii://skills` itself) recovers transparently.
 */
export function useSkillsIndex() {
    const [index, setIndex] = useState(null);
    const [error, setError] = useState(null);
    // Guard against setState after unmount (StrictMode double-mounts the
    // effect during dev; the second mount can land after the first fetch).
    const mountedRef = useRef(true);
    const refresh = useCallback(async () => {
        try {
            const md = await bridge("skill::fetch", { uri: INDEX_URI });
            if (!mountedRef.current)
                return;
            setIndex(md);
            setError(null);
        }
        catch (e) {
            if (!mountedRef.current)
                return;
            const msg = e instanceof BridgeError ? e.message : String(e);
            setError(msg);
        }
    }, []);
    useEffect(() => {
        mountedRef.current = true;
        void refresh();
        return () => {
            mountedRef.current = false;
        };
    }, [refresh]);
    return { index, error, refresh };
}
