// Status tab — workers table + rolling events feed + budget breakdown.
//
// Renders only when the parent App's `tab === "status"`, so the budget
// snapshot fetch on mount is bounded and easy to reason about. Live updates
// flow in via useStatus's pushed snapshots — this file does not subscribe.
//
// Rules of thumb encoded here:
//   - Workers table: text status (a11y), color secondary.
//   - Events feed: rolling 200, filter chips (agent/state/tool/error), pause.
//   - Budget breakdown: hydrated once from budget::list + budget::usage,
//     patched in-place from ui::cost::tick.
//   - Disconnected >5s → "live updates paused" banner; on reconnect, refetch.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bridge, BridgeError } from "../bridge";
import type { Budget, BudgetUsage } from "../types";
import type {
  CostSnapshot,
  StatusEvent,
  WorkersSnapshot,
} from "../useStatus";

interface Props {
  cost: CostSnapshot;
  workers: WorkersSnapshot;
  events: StatusEvent[];
  hydrated: boolean;
  /** Connection from useConnection. We render the disconnected banner here. */
  connected: boolean;
  /** ms since last connection transition. Used to gate the "paused" banner. */
  connectionSince: number;
  /** StatusTab "clear feed" button → useStatus.clearEvents(). */
  onClearEvents: () => void;
}

type FilterKind = "all" | "approval" | "cost" | "workers";

const DISCONNECT_PAUSED_AFTER_MS = 5_000;

function fmtUsd(n: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 2,
  }).format(n);
}

function fmtTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Compact label for a single feed entry. Stable + JSON-pretty payload. */
function summarizeEvent(ev: StatusEvent): string {
  const p = ev.payload as Record<string, unknown> | null;
  if (!p) return ev.kind;
  switch (ev.kind) {
    case "approval": {
      const id = (p.tool_call_id as string | undefined) ?? "?";
      const name = (p.tool_name as string | undefined) ?? "";
      const decision = p.decision as string | undefined;
      return decision
        ? `approval ${decision} · ${id}`
        : `approval requested · ${name} · ${id}`;
    }
    case "cost": {
      const usd = (p.usd_today as number | undefined) ?? 0;
      return `cost · ${fmtUsd(usd)} today`;
    }
    case "workers": {
      const up = (p.up as number | undefined) ?? 0;
      const total = (p.total as number | undefined) ?? 0;
      return `workers · ${up}/${total} up`;
    }
    default:
      return ev.kind;
  }
}

/** 80×16 inline SVG sparkline of the cost-tick history. */
function CostSparkline({ points }: { points: number[] }) {
  if (points.length < 2) {
    return <svg className="cost-sparkline" width={80} height={16} aria-hidden />;
  }
  const max = Math.max(...points);
  const min = Math.min(...points);
  const span = max - min || 1;
  const w = 80;
  const h = 16;
  const dx = w / (points.length - 1);
  const path = points
    .map((p, i) => {
      const x = i * dx;
      const y = h - ((p - min) / span) * h;
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg
      className="cost-sparkline"
      width={w}
      height={h}
      viewBox={`0 0 ${w} ${h}`}
      role="img"
      aria-label="cost trend"
    >
      <path
        d={path}
        fill="none"
        stroke="var(--ink-faint)"
        strokeWidth="1"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

export function StatusTab({
  cost,
  workers,
  events,
  hydrated,
  connected,
  connectionSince,
  onClearEvents,
}: Props) {
  const [filter, setFilter] = useState<FilterKind>("all");
  const [paused, setPaused] = useState(false);
  const [budgets, setBudgets] = useState<Budget[]>([]);
  const [budgetUsage, setBudgetUsage] = useState<Record<string, BudgetUsage>>(
    {},
  );
  const [budgetError, setBudgetError] = useState<string | null>(null);

  // Snapshot the events list when paused so the user can read without
  // watching it scroll. We re-snapshot on each unpause + every render.
  const liveEventsRef = useRef<StatusEvent[]>(events);
  if (!paused) liveEventsRef.current = events;
  const visibleEvents = paused ? liveEventsRef.current : events;

  // Cost sparkline: derive a buffer from the last ~50 cost ticks. Capped
  // so the SVG doesn't grow unbounded in long sessions.
  const costPoints = useMemo(
    () =>
      events
        .filter((e) => e.kind === "cost")
        .slice(-50)
        .map((e) => {
          const p = e.payload as { usd_today?: number };
          return p?.usd_today ?? 0;
        }),
    [events],
  );

  const hydrateBudgets = useCallback(async () => {
    setBudgetError(null);
    try {
      const resp = await bridge<{ budgets: Budget[] } | Budget[]>(
        "budget::list",
        {},
      );
      const list = Array.isArray(resp)
        ? resp
        : Array.isArray(resp?.budgets)
          ? resp.budgets
          : [];
      setBudgets(list);
      // Best-effort usage hydration. Failures are surfaced inline per row.
      const entries = await Promise.all(
        list.map(async (b) => {
          try {
            const u = await bridge<BudgetUsage>("budget::usage", {
              budget_id: b.id,
              window: "all",
            });
            return [b.id, u] as const;
          } catch {
            return [b.id, null] as const;
          }
        }),
      );
      const usage: Record<string, BudgetUsage> = {};
      for (const [id, u] of entries) {
        if (u) usage[id] = u;
      }
      setBudgetUsage(usage);
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setBudgetError(msg);
      setBudgets([]);
      setBudgetUsage({});
    }
  }, []);

  // Hydrate budgets on mount.
  useEffect(() => {
    void hydrateBudgets();
  }, [hydrateBudgets]);

  // Re-hydrate when the connection comes back after being down.
  const wasConnectedRef = useRef(connected);
  useEffect(() => {
    if (!wasConnectedRef.current && connected) {
      void hydrateBudgets();
    }
    wasConnectedRef.current = connected;
  }, [connected, hydrateBudgets]);

  // "live updates paused" banner — only after the disconnect has lasted
  // long enough that the user might think things are frozen.
  const showPausedBanner =
    !connected &&
    connectionSince > 0 &&
    Date.now() - connectionSince > DISCONNECT_PAUSED_AFTER_MS;

  const filteredEvents = useMemo(() => {
    if (filter === "all") return visibleEvents;
    return visibleEvents.filter((e) => e.kind === filter);
  }, [visibleEvents, filter]);

  return (
    <section className="status-tab">
      {showPausedBanner ? (
        <p className="status-banner" role="status">
          live updates paused — engine unreachable. UI will resync on reconnect.
        </p>
      ) : null}

      <div className="status-grid">
        <section className="status-card">
          <header className="panel-head">
            <span className="panel-title">workers</span>
            <span className="panel-sub">
              {workers.total === 0
                ? "no status yet"
                : `${workers.up}/${workers.total} up`}
            </span>
          </header>
          {workers.workers.length === 0 ? (
            <p className="panel-empty">
              awaiting first ui::workers::changed push
              {hydrated ? "" : " (≤5s)"}
            </p>
          ) : (
            <table className="workers-table">
              <thead>
                <tr>
                  <th scope="col">worker</th>
                  <th scope="col">status</th>
                </tr>
              </thead>
              <tbody>
                {workers.workers.map((w) => (
                  <tr key={w.name}>
                    <td className="worker-name">{w.name}</td>
                    <td className="worker-status" data-status={w.status}>
                      {w.status}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>

        <section className="status-card">
          <header className="panel-head">
            <span className="panel-title">cost</span>
            <span className="panel-sub">{fmtUsd(cost.usd_today)} today</span>
            <CostSparkline points={costPoints} />
          </header>
          {budgetError ? (
            <p className="panel-error" role="alert">
              {budgetError}
            </p>
          ) : null}
          {budgets.length === 0 && !budgetError ? (
            <p className="panel-empty">
              no budgets configured. create one via{" "}
              <code>budget::create</code> on the bus.
            </p>
          ) : null}
          {budgets.length > 0 ? (
            <ul className="budget-list compact">
              {budgets.map((b) => {
                const usage = budgetUsage[b.id];
                const pct =
                  b.ceiling_usd > 0
                    ? `${((b.spent_usd / b.ceiling_usd) * 100).toFixed(1)}%`
                    : "—";
                return (
                  <li key={b.id} className="budget-row">
                    <header className="budget-row-head">
                      <span className="budget-id">{b.name ?? b.id}</span>
                      <span className="budget-period">{b.period}</span>
                    </header>
                    <dl className="budget-grid">
                      <div>
                        <dt>spent</dt>
                        <dd>
                          {fmtUsd(b.spent_usd)}{" "}
                          <span>/ {fmtUsd(b.ceiling_usd)}</span>
                        </dd>
                      </div>
                      <div>
                        <dt>utilization</dt>
                        <dd>{pct}</dd>
                      </div>
                      {usage ? (
                        <div>
                          <dt>records</dt>
                          <dd>{usage.records_count}</dd>
                        </div>
                      ) : null}
                    </dl>
                  </li>
                );
              })}
            </ul>
          ) : null}
        </section>

        <section className="status-card events-card">
          <header className="panel-head">
            <span className="panel-title">events</span>
            <span className="panel-sub">
              {filteredEvents.length} of {events.length} (rolling 200)
            </span>
            <div className="events-controls">
              {(["all", "approval", "cost", "workers"] as FilterKind[]).map(
                (f) => (
                  <button
                    key={f}
                    type="button"
                    className="events-filter"
                    data-active={filter === f}
                    onClick={() => setFilter(f)}
                  >
                    {f}
                  </button>
                ),
              )}
              <button
                type="button"
                className="events-control"
                onClick={() => setPaused((p) => !p)}
              >
                {paused ? "resume" : "pause"}
              </button>
              <button
                type="button"
                className="events-control"
                onClick={onClearEvents}
              >
                clear
              </button>
            </div>
          </header>
          {filteredEvents.length === 0 ? (
            <p className="panel-empty">no events yet</p>
          ) : (
            <ol className="events-feed" aria-live={paused ? "off" : "polite"}>
              {filteredEvents
                .slice()
                .reverse()
                .map((ev) => (
                  <li
                    key={ev.id}
                    className="events-row"
                    data-kind={ev.kind}
                  >
                    <span className="events-time">{fmtTime(ev.at)}</span>
                    <span className="events-kind">{ev.kind}</span>
                    <span className="events-summary">{summarizeEvent(ev)}</span>
                  </li>
                ))}
            </ol>
          )}
        </section>
      </div>
    </section>
  );
}
