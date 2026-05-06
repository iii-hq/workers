import { useCallback, useEffect, useRef, useState } from "react";
import { bridge, BridgeError } from "./bridge";

export interface SkillsIndex {
  /** Markdown body of `iii://skills`, or null if the fetch hasn't resolved
   *  successfully yet (still loading, or last attempt failed). */
  index: string | null;
  /** Last fetch error, or null if the most recent attempt succeeded. */
  error: string | null;
  /** Re-run the fetch. Useful for a future skills::on-change subscription. */
  refresh: () => Promise<void>;
}

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
export function useSkillsIndex(): SkillsIndex {
  const [index, setIndex] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Guard against setState after unmount (StrictMode double-mounts the
  // effect during dev; the second mount can land after the first fetch).
  const mountedRef = useRef(true);

  const refresh = useCallback(async (): Promise<void> => {
    try {
      const md = await bridge<string>("skill::fetch", { uri: INDEX_URI });
      if (!mountedRef.current) return;
      setIndex(md);
      setError(null);
    } catch (e) {
      if (!mountedRef.current) return;
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
