/**
 * Provider overrides store backends.
 *
 * Two implementations:
 *
 * 1. `InMemoryStore` -- used in tests and for local-only sessions.
 * 2. `DbOverridesStore` -- SQL-backed via the shared `DbStore<ProviderOverrides>`
 *    (see `../runtime/database-store.ts`).
 *
 * `set()` preserves the legacy merge semantic -- partial overrides patch
 * existing overrides for the same provider rather than replacing them. The
 * merge happens in-process inside the underlying `DbStore.update`, which
 * serializes through a queue and does a single SQL UPSERT per call.
 */

import { DbStore, type DbStoreOptions } from '../runtime/database-store.js';
import type { ISdk } from '../runtime/iii.js';
import type { ProviderOverrides } from './types.js';

/** Table that backs `DbOverridesStore`. */
export const PROVIDER_CONFIG_TABLE = 'provider_config';

export interface OverridesStore {
  get(provider: string): Promise<ProviderOverrides | null>;
  set(provider: string, overrides: ProviderOverrides): Promise<void>;
  clear(provider: string): Promise<void>;
  list(): Promise<Array<readonly [string, ProviderOverrides]>>;
}

export class InMemoryStore implements OverridesStore {
  private readonly store = new Map<string, ProviderOverrides>();
  async get(provider: string): Promise<ProviderOverrides | null> {
    return this.store.get(provider) ?? null;
  }
  async set(provider: string, overrides: ProviderOverrides): Promise<void> {
    const existing = this.store.get(provider) ?? {};
    this.store.set(provider, { ...existing, ...overrides });
  }
  async clear(provider: string): Promise<void> {
    this.store.delete(provider);
  }
  async list(): Promise<Array<readonly [string, ProviderOverrides]>> {
    return [...this.store.entries()];
  }
}

/**
 * SQL-backed overrides store. Construct, then `await init()` before
 * serving any bus traffic so the underlying table exists.
 */
export class DbOverridesStore implements OverridesStore {
  private readonly inner: DbStore<ProviderOverrides>;

  constructor(iii: ISdk, opts: Pick<DbStoreOptions, 'databaseName'>) {
    this.inner = new DbStore<ProviderOverrides>(iii, {
      databaseName: opts.databaseName,
      tableName: PROVIDER_CONFIG_TABLE,
    });
  }

  /** Idempotent -- safe to call on every worker startup. */
  async init(): Promise<void> {
    await this.inner.init();
  }

  /** Expose the underlying DbStore so the JSON-import migration can target it. */
  unwrap(): DbStore<ProviderOverrides> {
    return this.inner;
  }

  get(provider: string): Promise<ProviderOverrides | null> {
    return this.inner.get(provider);
  }

  /**
   * Shallow-merge overrides into the existing row. Mirrors the legacy
   * FileStore which did `{ ...existing, ...overrides }` so callers can
   * patch one field (e.g. only `default_api_url`) without clobbering the
   * rest.
   */
  set(provider: string, overrides: ProviderOverrides): Promise<void> {
    return this.inner.update(provider, overrides);
  }

  clear(provider: string): Promise<void> {
    return this.inner.clear(provider);
  }

  list(): Promise<Array<readonly [string, ProviderOverrides]>> {
    return this.inner.list();
  }
}
