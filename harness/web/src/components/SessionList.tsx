import { bridge } from "../bridge";
import type { SessionRow } from "../types";

export async function fetchSessions(): Promise<SessionRow[]> {
  // state::list returns values without keys. The turn_state value is the
  // only one in scope=agent that carries both session_id and state, so we
  // filter for it.
  const values = await bridge<unknown[]>("state::list", {
    scope: "agent",
    prefix: "session/",
  });
  if (!Array.isArray(values)) return [];

  const rows: SessionRow[] = [];
  for (const v of values) {
    if (
      v &&
      typeof v === "object" &&
      "session_id" in v &&
      "state" in v &&
      typeof (v as Record<string, unknown>).session_id === "string"
    ) {
      const o = v as Record<string, unknown>;
      rows.push({
        session_id: String(o.session_id),
        state: String(o.state ?? "unknown"),
        turn_count: Number(o.turn_count ?? 0),
        updated_at_ms: Number(o.updated_at_ms ?? 0),
      });
    }
  }
  rows.sort((a, b) => b.updated_at_ms - a.updated_at_ms);
  return rows;
}

interface Props {
  sessions: SessionRow[];
  active: string | null;
  onPick: (id: string) => void;
  onNew: () => void;
}

export function SessionList({ sessions, active, onPick, onNew }: Props) {
  return (
    <aside className="rail">
      <header className="rail-head">
        <span className="rail-title">sessions</span>
        <button className="btn-ghost" onClick={onNew} type="button">
          + new
        </button>
      </header>
      {sessions.length === 0 ? (
        <p className="rail-empty">
          The bus is quiet. Address the agent at right and the first turn will
          appear here.
        </p>
      ) : (
        <ul className="session-list">
          {sessions.map((s) => (
            <li key={s.session_id}>
              <button
                type="button"
                className="session-row"
                data-active={s.session_id === active}
                onClick={() => onPick(s.session_id)}
              >
                <span className="session-id">{s.session_id}</span>
                <span className="session-meta">
                  <span data-state={s.state}>{s.state}</span>
                  <span>·</span>
                  <span>{s.turn_count} turn{s.turn_count === 1 ? "" : "s"}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}
