/**
 * In-memory mock for the subset of `database::*` that `DbStore` actually
 * uses: CREATE TABLE, SELECT (by-provider, list-all), UPSERT, DELETE.
 *
 * Tests that exercise the SQL-backed stores construct a `FakeDatabaseSdk`,
 * pass it into `new DbStore({ databaseName: 'test', ... }, fake)`, and get
 * deterministic round-trip behaviour without spawning a real iii-database
 * worker. The SQL parser is intentionally narrow -- it matches the exact
 * statements `database-store.ts` issues. If you change the statements,
 * update this helper.
 */

import type { ISdk, TriggerRequest } from 'iii-sdk';

type Row = { provider: string; payload: string; updated_at: number };

type DbTable = Map<string, Row>;

interface QueryResp {
  rows: Array<Record<string, unknown>>;
  row_count: number;
}

interface ExecuteResp {
  affected_rows: number;
  last_insert_id: string | null;
}

const RX_CREATE = /CREATE TABLE IF NOT EXISTS\s+(\w+)/i;
const RX_SELECT_ONE = /SELECT payload FROM\s+(\w+)\s+WHERE provider = \?/i;
const RX_SELECT_ALL = /SELECT provider, payload FROM\s+(\w+)/i;
const RX_INSERT_UPSERT = /INSERT INTO\s+(\w+)\s+\(provider, payload, updated_at\)/i;
const RX_DELETE = /DELETE FROM\s+(\w+)\s+WHERE provider = \?/i;

export class FakeDatabaseSdk {
  private readonly tables = new Map<string, DbTable>();
  /** Records every call routed through `trigger` for assertion in tests. */
  readonly calls: Array<{ function_id: string; payload: unknown }> = [];

  /** Mount as the `ISdk` a worker would use. */
  asSdk(): ISdk {
    // The full ISdk surface is large; tests only exercise `trigger`. Cast
    // through unknown so type-checking still catches signature drift on the
    // single method we implement.
    return { trigger: this.trigger } as unknown as ISdk;
  }

  /** Reset all tables (between tests). */
  reset(): void {
    this.tables.clear();
    this.calls.length = 0;
  }

  /** Inspect a table directly. */
  table(name: string): DbTable | undefined {
    return this.tables.get(name);
  }

  // Arrow so `asSdk().trigger(...)` keeps `this`.
  private trigger = async <_TPayload, TResp>(req: TriggerRequest<_TPayload>): Promise<TResp> => {
    this.calls.push({ function_id: req.function_id, payload: req.payload });
    if (req.function_id === 'database::query') {
      return this.handleQuery(req.payload) as TResp;
    }
    if (req.function_id === 'database::execute') {
      return this.handleExecute(req.payload) as TResp;
    }
    throw new Error(`FakeDatabaseSdk: unsupported function ${req.function_id}`);
  };

  private handleQuery(payload: unknown): QueryResp {
    const { sql, params } = parseSqlPayload(payload);
    let match = sql.match(RX_SELECT_ONE);
    if (match) {
      const tbl = this.tableOf(match[1] ?? '');
      const provider = String(params[0] ?? '');
      const row = tbl.get(provider);
      const rows = row ? [{ payload: row.payload }] : [];
      return { rows, row_count: rows.length };
    }
    match = sql.match(RX_SELECT_ALL);
    if (match) {
      const tbl = this.tableOf(match[1] ?? '');
      const rows = [...tbl.values()]
        .sort((a, b) => a.provider.localeCompare(b.provider))
        .map(({ provider, payload: p }) => ({ provider, payload: p }));
      return { rows, row_count: rows.length };
    }
    throw new Error(`FakeDatabaseSdk: unrecognised query SQL\n${sql}`);
  }

  private handleExecute(payload: unknown): ExecuteResp {
    const { sql, params } = parseSqlPayload(payload);
    let match = sql.match(RX_CREATE);
    if (match) {
      const name = match[1] ?? '';
      if (!this.tables.has(name)) this.tables.set(name, new Map());
      return { affected_rows: 0, last_insert_id: null };
    }
    match = sql.match(RX_INSERT_UPSERT);
    if (match) {
      const tbl = this.tableOf(match[1] ?? '');
      const provider = String(params[0] ?? '');
      const payloadStr = String(params[1] ?? '');
      const updatedAt = Number(params[2] ?? 0);
      tbl.set(provider, { provider, payload: payloadStr, updated_at: updatedAt });
      return { affected_rows: 1, last_insert_id: null };
    }
    match = sql.match(RX_DELETE);
    if (match) {
      const tbl = this.tableOf(match[1] ?? '');
      const provider = String(params[0] ?? '');
      const existed = tbl.delete(provider);
      return { affected_rows: existed ? 1 : 0, last_insert_id: null };
    }
    throw new Error(`FakeDatabaseSdk: unrecognised execute SQL\n${sql}`);
  }

  private tableOf(name: string): DbTable {
    let tbl = this.tables.get(name);
    if (!tbl) {
      tbl = new Map();
      this.tables.set(name, tbl);
    }
    return tbl;
  }
}

function parseSqlPayload(payload: unknown): { db: string; sql: string; params: unknown[] } {
  const p = (payload ?? {}) as { db?: unknown; sql?: unknown; params?: unknown };
  return {
    db: String(p.db ?? ''),
    sql: String(p.sql ?? ''),
    params: Array.isArray(p.params) ? p.params : [],
  };
}
