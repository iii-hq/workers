import { bridge } from "../bridge";
import { groupByDate, truncatePath } from "../sessions";
import type { SessionRow } from "../types";

// Wire shape for `session-tree::list` rows. Mirrors `SessionListRow` in
// session-tree/src/lib.rs.
interface SessionTreeRow {
  session_id: string;
  created_at: number;
  updated_at: number;
  entry_count: number;
  display_name?: string;
  cwd?: string;
  last_message_summary?: string;
}

interface SessionTreeListResponse {
  sessions: SessionTreeRow[];
  total: number;
}

function fromSessionTreeRow(row: SessionTreeRow): SessionRow {
  return {
    session_id: row.session_id,
    state: "stopped",
    turn_count: row.entry_count,
    updated_at_ms: row.updated_at,
    cwd: row.cwd ?? null,
    last_message_summary: row.last_message_summary ?? null,
  };
}

// Drift fallback: pre-Phase-A sessions only live in the state store. If
// session-tree returns nothing (total === 0), surface those rows so the
// user can /repair them. This is the same parsing the previous fetcher used.
async function fetchFromStateFallback(): Promise<SessionRow[]> {
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
        cwd: null,
        last_message_summary: null,
      });
    }
  }
  rows.sort((a, b) => b.updated_at_ms - a.updated_at_ms);
  return rows;
}

export async function fetchSessions(): Promise<SessionRow[]> {
  // Prefer session-tree::list — it's the new source of truth post-Phase-A.
  try {
    const result = await bridge<SessionTreeListResponse>("session-tree::list", {
      order: "desc",
    });
    if (result?.sessions?.length) {
      return result.sessions.map(fromSessionTreeRow);
    }
    if (result?.total === 0) {
      // session-tree is genuinely empty — but state::list may have legacy
      // rows that haven't been migrated. Surface those so the user can
      // /repair them.
      try {
        return await fetchFromStateFallback();
      } catch {
        return [];
      }
    }
    return [];
  } catch {
    // session-tree unreachable — try the legacy reader anyway.
    try {
      return await fetchFromStateFallback();
    } catch {
      return [];
    }
  }
}

interface Props {
  sessions: SessionRow[];
  active: string | null;
  onPick: (id: string) => void;
  onNew: () => void;
}

const SUBTITLE_MAX = 32;

function renderRow(s: SessionRow, active: string | null, onPick: (id: string) => void) {
  const subtitle = s.cwd ? truncatePath(s.cwd, SUBTITLE_MAX) : null;
  return (
    <li key={s.session_id}>
      <button
        type="button"
        className="session-row"
        data-active={s.session_id === active}
        onClick={() => onPick(s.session_id)}
      >
        <span className="session-id">{s.session_id}</span>
        {subtitle ? (
          <span className="session-subtitle" title={s.cwd ?? undefined}>
            {subtitle}
          </span>
        ) : null}
        <span className="session-meta">
          <span data-state={s.state}>{s.state}</span>
          <span>·</span>
          <span>
            {s.turn_count} turn{s.turn_count === 1 ? "" : "s"}
          </span>
        </span>
      </button>
    </li>
  );
}

export function SessionList({ sessions, active, onPick, onNew }: Props) {
  const groups = groupByDate(sessions);
  const sections: Array<{ label: string; rows: SessionRow[] }> = [
    { label: "today", rows: groups.today },
    { label: "yesterday", rows: groups.yesterday },
    { label: "earlier", rows: groups.earlier },
  ].filter((s) => s.rows.length > 0);

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
        <div className="session-groups">
          {sections.map((section) => (
            <div key={section.label} className="session-group">
              <h3 className="session-group-h">{section.label}</h3>
              <ul className="session-list">
                {section.rows.map((s) => renderRow(s, active, onPick))}
              </ul>
            </div>
          ))}
        </div>
      )}
    </aside>
  );
}
