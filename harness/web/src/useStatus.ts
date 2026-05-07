// Aggregate live status for the harness header + status tab.
//
// Subscribes (once per page) to the four all-sessions push topics minted by
// the harness fanout (`harness/src/fanout.rs`):
//
//   ui::approval::requested  →  add to pendingApprovals + push event
//   ui::approval::resolved   →  remove from pendingApprovals + push event
//   ui::cost::tick           →  replace cost snapshot + push event
//   ui::workers::changed     →  replace workers snapshot + push event
//
// Owns a rolling 200-entry events buffer that StatusTab renders. Filters,
// pause/resume live in the consumer — this hook is unfiltered + always-on.
//
// Failure mode is documented in App.tsx: if the WS is dropped, useConnection
// surfaces the disconnected state and the UI gates re-hydration on reconnect.
// This hook does NOT attempt to reconnect itself; it just stops receiving
// pushes until the SDK re-establishes the connection.

import { useCallback, useEffect, useRef, useState } from "react";
import { getIiiClient } from "./iii-client";

/** Cost summary pushed by the fanout — `summarize_budgets` in fanout.rs. */
export interface CostSnapshot {
  usd_today: number;
  by_provider: Record<string, number>;
  by_period?: Record<string, number>;
  budgets?: number;
}

/** One worker row from `engine::workers::list`, normalized by the fanout. */
export interface WorkerSnapshot {
  name: string;
  status: string;
}

/** Workers payload pushed by the fanout — `diff_workers` in fanout.rs. */
export interface WorkersSnapshot {
  up: number;
  down: number;
  total: number;
  workers: WorkerSnapshot[];
}

/** Pending approval as pushed by the fanout's approval poll. */
export interface PendingApprovalSummary {
  tool_call_id: string;
  tool_name?: string;
  args?: unknown;
  expires_at?: number;
  session_id?: string;
}

/** A single rolling-buffer entry. Compact on purpose — the StatusTab feed
 *  formats each line, so we keep the raw payload for filter chips to inspect. */
export interface StatusEvent {
  /** Local-monotonic id for stable React keys. */
  id: number;
  /** Origin topic — drives filter chips. */
  kind: "approval" | "cost" | "workers";
  /** ms timestamp of arrival. Used for relative formatting. */
  at: number;
  /** Raw payload from the WS push, for downstream display. */
  payload: unknown;
}

const EVENT_BUFFER_CAP = 200;

const INITIAL_COST: CostSnapshot = {
  usd_today: 0,
  by_provider: {},
};

const INITIAL_WORKERS: WorkersSnapshot = {
  up: 0,
  down: 0,
  total: 0,
  workers: [],
};

export interface UseStatusValue {
  pendingApprovals: PendingApprovalSummary[];
  cost: CostSnapshot;
  workers: WorkersSnapshot;
  events: StatusEvent[];
  /** True once the hook has subscribed and seen at least one push of any kind. */
  hydrated: boolean;
  /** Drop the rolling buffer (StatusTab "clear" button). */
  clearEvents: () => void;
}

interface ResolvedPayload {
  tool_call_id: string;
  decision?: "allow" | "deny";
}

export function useStatus(): UseStatusValue {
  const [pendingApprovals, setPendingApprovals] = useState<
    PendingApprovalSummary[]
  >([]);
  const [cost, setCost] = useState<CostSnapshot>(INITIAL_COST);
  const [workers, setWorkers] = useState<WorkersSnapshot>(INITIAL_WORKERS);
  const [events, setEvents] = useState<StatusEvent[]>([]);
  const [hydrated, setHydrated] = useState(false);

  // Monotonic event id. Refs survive re-renders; we never need to read it
  // during render so using useRef is sound.
  const idRef = useRef(0);
  const nextId = useCallback(() => {
    idRef.current += 1;
    return idRef.current;
  }, []);

  const pushEvent = useCallback(
    (kind: StatusEvent["kind"], payload: unknown) => {
      setEvents((prev) => {
        const next = prev.concat({
          id: nextId(),
          kind,
          at: Date.now(),
          payload,
        });
        if (next.length <= EVENT_BUFFER_CAP) return next;
        return next.slice(next.length - EVENT_BUFFER_CAP);
      });
    },
    [nextId],
  );

  const clearEvents = useCallback(() => setEvents([]), []);

  useEffect(() => {
    let cancelled = false;
    const offs: Array<() => void> = [];
    let subscribed = false;
    let browserId: string | null = null;

    void (async () => {
      try {
        const client = await getIiiClient();
        if (cancelled) return;
        browserId = client.browserId;

        offs.push(
          client.on<PendingApprovalSummary>(
            "ui::approval::requested",
            (payload) => {
              if (!payload?.tool_call_id) return;
              setPendingApprovals((prev) => {
                if (prev.some((p) => p.tool_call_id === payload.tool_call_id)) {
                  return prev;
                }
                return prev.concat(payload);
              });
              pushEvent("approval", payload);
              setHydrated(true);
            },
          ),
        );

        offs.push(
          client.on<ResolvedPayload>("ui::approval::resolved", (payload) => {
            if (!payload?.tool_call_id) return;
            setPendingApprovals((prev) =>
              prev.filter((p) => p.tool_call_id !== payload.tool_call_id),
            );
            pushEvent("approval", payload);
            setHydrated(true);
          }),
        );

        offs.push(
          client.on<CostSnapshot>("ui::cost::tick", (payload) => {
            if (!payload || typeof payload.usd_today !== "number") return;
            setCost({
              usd_today: payload.usd_today,
              by_provider: payload.by_provider ?? {},
              by_period: payload.by_period,
              budgets: payload.budgets,
            });
            pushEvent("cost", payload);
            setHydrated(true);
          }),
        );

        offs.push(
          client.on<WorkersSnapshot>("ui::workers::changed", (payload) => {
            if (!payload || !Array.isArray(payload.workers)) return;
            setWorkers(payload);
            pushEvent("workers", payload);
            setHydrated(true);
          }),
        );

        await client.call("ui::subscribe", {
          browser_id: browserId,
          session_id: null,
        });
        subscribed = true;
      } catch (err) {
        // Bootstrap failed — the StatusPill already surfaces it. Don't
        // throw from a hook; the tree must keep rendering.
        // eslint-disable-next-line no-console
        console.warn("[useStatus] subscribe failed", err);
      }
    })();

    return () => {
      cancelled = true;
      for (const off of offs) {
        try {
          off();
        } catch {
          // SDK already disposed
        }
      }
      if (subscribed && browserId) {
        void getIiiClient().then((client) =>
          client
            .call("ui::unsubscribe", {
              browser_id: browserId,
              session_id: null,
            })
            .catch(() => {}),
        );
      }
    };
  }, [pushEvent]);

  return {
    pendingApprovals,
    cost,
    workers,
    events,
    hydrated,
    clearEvents,
  };
}
