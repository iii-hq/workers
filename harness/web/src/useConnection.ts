// Hook that exposes the iii-browser-sdk connection state to the UI.
// The status pill (and any other ambient indicators) read from this so the
// transport layer is observable without each consumer threading the SDK.

import { useEffect, useState } from "react";
import type { IIIConnectionState } from "iii-browser-sdk";
import { getIiiClient } from "./iii-client";

export interface ConnectionInfo {
  status: IIIConnectionState;
  /** ms timestamp of the last transition. Useful for "reconnecting since…" UI. */
  since: number;
}

const INITIAL: ConnectionInfo = {
  status: "disconnected",
  since: 0,
};

export function useConnection(): ConnectionInfo {
  const [info, setInfo] = useState<ConnectionInfo>(INITIAL);

  useEffect(() => {
    let cancelled = false;
    let unsub: (() => void) | undefined;
    void (async () => {
      try {
        const client = await getIiiClient();
        if (cancelled) return;
        unsub = client.addConnectionStateListener((state) => {
          setInfo({ status: state, since: Date.now() });
        });
      } catch {
        // Bootstrap failure leaves status='disconnected'; the StatusPill
        // surfaces the same condition. Don't crash the tree.
      }
    })();
    return () => {
      cancelled = true;
      unsub?.();
    };
  }, []);

  return info;
}
