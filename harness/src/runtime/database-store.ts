/**
 * SQL-backed per-provider key-value store. Backs both `auth-credentials`
 * and `provider-config`: same shape (provider → JSON payload), same
 * lifecycle, same on-bus call pattern via `database::query` and
 * `database::execute`.
 *
 * Each store owns one table:
 *
 *   CREATE TABLE IF NOT EXISTS <name> (
 *     provider   TEXT    PRIMARY KEY,
 *     payload    TEXT    NOT NULL,    -- JSON
 *     updated_at INTEGER NOT NULL     -- ms epoch
 *   )
 *
 * Mutations use `INSERT ... ON CONFLICT(provider) DO UPDATE`, so a single
 * `set(provider, value)` is atomic at the SQL layer — no read-modify-write
 * race between concurrent callers.
 *
 * For callers that need merge-not-replace semantics (provider-config), the
 * merge happens at the JS layer inside an in-process write queue. Same
 * "single-process by default" guarantee the old FileStore had, with
 * read/write IO replaced by SQL round-trips.
 */

import { getSection } from './config.js';
import type { ISdk } from './iii.js';
import { logger } from './otel.js';
import { loadStorageConfig, resolveDatabaseName } from './storage-config.js';

/** Shape of `database::query` response we care about. */
interface QueryResp<TRow = Record<string, unknown>> {
  rows: TRow[];
  row_count: number;
}

/** Shape of `database::execute` response we care about. */
interface ExecuteResp {
  affected_rows: number;
  last_insert_id: string | null;
}

/** Per-call timeout for talking to the database worker. */
const DB_TIMEOUT_MS = 5_000;

export interface DbStoreOptions {
  /** Logical database name as configured in `iii-database` (matches the key in its `databases:` map). */
  databaseName: string;
  /** Table name for this store. Created on first `init()` call. */
  tableName: string;
}

export interface CreateDbStoreOptions {
  /** Top-level config section key for this worker (e.g. `auth_credentials`). When omitted, only `storage.database_name` applies. */
  workerSection?: string;
  /** Table name for this store. Created on first `init()` call. */
  tableName: string;
}

/**
 * Construct a `DbStore` from an already-loaded harness config. Resolves
 * `database_name` via the shared storage helper so new workers opt in
 * without re-plumbing config.
 */
export function createDbStore<T>(
  iii: ISdk,
  cfg: Record<string, unknown>,
  opts: CreateDbStoreOptions,
): DbStore<T> {
  const storage = loadStorageConfig(cfg);
  const workerSection = opts.workerSection ? getSection(cfg, opts.workerSection) : {};
  const databaseName = resolveDatabaseName(workerSection, storage);
  return new DbStore<T>(iii, { databaseName, tableName: opts.tableName });
}

/**
 * Generic per-provider JSON store, backed by `database::*`.
 *
 * `T` is the JSON payload type — anything that round-trips through
 * `JSON.stringify` / `JSON.parse`. The store never inspects the payload.
 */
export class DbStore<T> {
  private readonly iii: ISdk;
  private readonly db: string;
  private readonly table: string;
  private writeChain: Promise<void> = Promise.resolve();
  private initialized = false;

  constructor(iii: ISdk, opts: DbStoreOptions) {
    // Defense-in-depth: table name is interpolated into SQL (SQLite has no
    // parameter binding for identifiers). Callers today pass compile-time
    // constants; this guard ensures a future caller can't accidentally
    // open a SQL-injection surface.
    if (!/^[a-z_][a-z0-9_]*$/i.test(opts.tableName)) {
      throw new Error(`invalid table name: ${opts.tableName}`);
    }
    this.iii = iii;
    this.db = opts.databaseName;
    this.table = opts.tableName;
  }

  /** Create the table if it doesn't exist. Idempotent. */
  async init(): Promise<void> {
    if (this.initialized) return;
    await this.execute(
      `CREATE TABLE IF NOT EXISTS ${this.table} (
         provider   TEXT    PRIMARY KEY,
         payload    TEXT    NOT NULL,
         updated_at INTEGER NOT NULL
       )`,
    );
    this.initialized = true;
  }

  async get(provider: string): Promise<T | null> {
    const { rows } = await this.query<{ payload: string }>(
      `SELECT payload FROM ${this.table} WHERE provider = ? LIMIT 1`,
      [provider],
    );
    if (rows.length === 0) return null;
    return parseJsonOrNull<T>(rows[0]?.payload, { table: this.table, provider });
  }

  async list(): Promise<Array<readonly [string, T]>> {
    const { rows } = await this.query<{ provider: string; payload: string }>(
      `SELECT provider, payload FROM ${this.table} ORDER BY provider`,
    );
    const out: Array<readonly [string, T]> = [];
    for (const row of rows) {
      const parsed = parseJsonOrNull<T>(row.payload, {
        table: this.table,
        provider: row.provider,
      });
      if (parsed !== null) out.push([row.provider, parsed] as const);
    }
    return out;
  }

  /**
   * Replace the entire payload for a provider. Atomic at the SQL layer.
   * For merge-not-replace semantics, use `update()`.
   */
  async set(provider: string, value: T): Promise<void> {
    const now = Date.now();
    await this.execute(
      `INSERT INTO ${this.table} (provider, payload, updated_at) VALUES (?, ?, ?)
       ON CONFLICT(provider) DO UPDATE SET
         payload = excluded.payload,
         updated_at = excluded.updated_at`,
      [provider, JSON.stringify(value), now],
    );
  }

  /**
   * Shallow-merge `patch` into the existing payload for `provider`. Reads
   * the current row, merges in JS, then writes the result back. Serialized
   * through an in-process write queue so concurrent updates within this
   * Node process don't clobber each other. Multi-process safety relies on
   * "one harness composite owns the table" — same posture as the old
   * FileStore.
   */
  async update(provider: string, patch: Partial<T>): Promise<void> {
    await this.queue(async () => {
      const existing = (await this.get(provider)) ?? ({} as T);
      const merged = { ...existing, ...patch } as T;
      await this.set(provider, merged);
    });
  }

  async clear(provider: string): Promise<void> {
    await this.execute(`DELETE FROM ${this.table} WHERE provider = ?`, [provider]);
  }

  // ---------------------------------------------------------------------------
  // Internals
  // ---------------------------------------------------------------------------

  private queue<R>(op: () => Promise<R>): Promise<R> {
    const next = this.writeChain.then(() => op());
    this.writeChain = next.then(
      () => undefined,
      () => undefined,
    );
    return next;
  }

  private async query<TRow>(sql: string, params: unknown[] = []): Promise<QueryResp<TRow>> {
    const result = await this.iii.trigger<unknown, QueryResp<TRow>>({
      function_id: 'database::query',
      payload: { db: this.db, sql, params },
      timeoutMs: DB_TIMEOUT_MS,
    });
    if (!result || typeof result !== 'object' || !Array.isArray((result as QueryResp).rows)) {
      throw new Error(`database::query returned unexpected shape for table=${this.table}`);
    }
    return result;
  }

  private async execute(sql: string, params: unknown[] = []): Promise<ExecuteResp> {
    const result = await this.iii.trigger<unknown, ExecuteResp>({
      function_id: 'database::execute',
      payload: { db: this.db, sql, params },
      timeoutMs: DB_TIMEOUT_MS,
    });
    if (!result || typeof result !== 'object') {
      throw new Error(`database::execute returned unexpected shape for table=${this.table}`);
    }
    return result;
  }
}

function parseJsonOrNull<T>(
  text: string | undefined | null,
  ctx: { table: string; provider?: string },
): T | null {
  if (typeof text !== 'string') return null;
  try {
    const parsed = JSON.parse(text);
    if (parsed && typeof parsed === 'object') return parsed as T;
    logger.warn('DbStore: non-object payload, treating as missing', {
      ...ctx,
      kind: parsed === null ? 'null' : typeof parsed,
    });
    return null;
  } catch (err) {
    logger.warn('DbStore: corrupt JSON payload, treating as missing', {
      ...ctx,
      err: String(err),
    });
    return null;
  }
}
