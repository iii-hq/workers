// Header status strip — currently a single Approvals chip. Placed right of
// the app mark, left of the connection pill. Click jumps to the status tab
// (the parent owns tab state, so we accept an `onJumpToStatus` callback).

import type { PendingApprovalSummary } from "../useStatus";

interface Props {
  pendingApprovals: PendingApprovalSummary[];
  /** Connection state from useConnection. Drives the "reconnecting" dot. */
  connected: boolean;
  /** Click handler — should switch the App tab to "status". */
  onJumpToStatus: () => void;
}

export function StatusStrip({
  pendingApprovals,
  connected,
  onJumpToStatus,
}: Props) {
  const count = pendingApprovals.length;
  const hasPending = count > 0;

  return (
    <div className="status-strip" role="group" aria-label="status chips">
      <button
        type="button"
        className="status-chip"
        data-tone={hasPending ? "alert" : "muted"}
        data-pulse={hasPending ? "true" : "false"}
        data-disconnected={connected ? "false" : "true"}
        onClick={onJumpToStatus}
        aria-label={
          hasPending
            ? `${count} approvals pending — click to view`
            : "no approvals pending"
        }
      >
        <span className="status-chip-label">approvals</span>
        <span className="status-chip-value">{count}</span>
        {!connected ? (
          <span
            className="status-chip-dot"
            aria-label="reconnecting"
            title="reconnecting"
          />
        ) : null}
      </button>
    </div>
  );
}
