// Foot chips — daily cost USD + workers `N/M up`. Compact, mono.
//
// The chips are read-only: the StatusTab is where users drill in. The
// "reconnecting" dot mirrors StatusStrip and turns the chips gray after
// the connection drops.

import type { CostSnapshot, WorkersSnapshot } from "../useStatus";

interface Props {
  cost: CostSnapshot;
  workers: WorkersSnapshot;
  connected: boolean;
}

function fmtUsd(n: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 2,
  }).format(n);
}

export function FootStatus({ cost, workers, connected }: Props) {
  const total = workers.total;
  const up = workers.up;
  const allUp = total > 0 && up === total;
  const anyDown = total > 0 && up < total;

  return (
    <>
      <span
        className="foot-chip"
        data-tone="muted"
        data-disconnected={connected ? "false" : "true"}
        title="cost spent today (sum of daily budgets)"
      >
        <span className="foot-chip-label">cost</span>
        <span className="foot-chip-value">{fmtUsd(cost.usd_today)}</span>
        {!connected ? <span className="foot-chip-dot" aria-hidden /> : null}
      </span>
      <span
        className="foot-chip"
        data-tone={allUp ? "ok" : anyDown ? "warn" : "muted"}
        data-disconnected={connected ? "false" : "true"}
        title={
          total === 0
            ? "no worker status yet"
            : `${up} of ${total} workers up`
        }
      >
        <span className="foot-chip-label">workers</span>
        <span className="foot-chip-value">
          {total === 0 ? "—" : `${up}/${total} up`}
        </span>
        {!connected ? <span className="foot-chip-dot" aria-hidden /> : null}
      </span>
    </>
  );
}
