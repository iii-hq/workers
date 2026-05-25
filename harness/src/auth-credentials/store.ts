/**
 * Credential store backends.
 *
 * Two implementations:
 *
 * 1. `InMemoryStore` -- used in tests and for local-only sessions.
 * 2. `DbCredentialStore` -- SQL-backed. Wraps the shared `DbStore<Credential>`
 *    (see `../runtime/database-store.ts`) which routes through the
 *    `database::*` worker.
 *
 * Multi-process posture is unchanged: a single `auth-credentials` worker
 * owns the `auth_credentials` table; concurrent writes from inside the
 * harness composite are atomic at the SQL UPSERT level.
 */

import { DbStore, type DbStoreOptions } from '../runtime/database-store.js';
import type { ISdk } from '../runtime/iii.js';
import type { Credential } from './types.js';

/** Table that backs `DbCredentialStore`. */
export const CREDENTIALS_TABLE = 'auth_credentials';

export interface CredentialStore {
  get(provider: string): Promise<Credential | null>;
  set(provider: string, credential: Credential): Promise<void>;
  clear(provider: string): Promise<void>;
  list(): Promise<Array<readonly [string, Credential]>>;
}

export class InMemoryStore implements CredentialStore {
  private readonly store = new Map<string, Credential>();
  async get(provider: string): Promise<Credential | null> {
    return this.store.get(provider) ?? null;
  }
  async set(provider: string, credential: Credential): Promise<void> {
    this.store.set(provider, credential);
  }
  async clear(provider: string): Promise<void> {
    this.store.delete(provider);
  }
  async list(): Promise<Array<readonly [string, Credential]>> {
    return [...this.store.entries()];
  }
}

/**
 * SQL-backed credential store. Construct, then `await init()` before
 * serving any bus traffic so the underlying table exists.
 */
export class DbCredentialStore implements CredentialStore {
  private readonly inner: DbStore<Credential>;

  constructor(iii: ISdk, opts: Pick<DbStoreOptions, 'databaseName'>) {
    this.inner = new DbStore<Credential>(iii, {
      databaseName: opts.databaseName,
      tableName: CREDENTIALS_TABLE,
    });
  }

  /** Idempotent -- safe to call on every worker startup. */
  async init(): Promise<void> {
    await this.inner.init();
  }

  /** Expose the underlying DbStore so the JSON-import migration can target it. */
  unwrap(): DbStore<Credential> {
    return this.inner;
  }

  get(provider: string): Promise<Credential | null> {
    return this.inner.get(provider);
  }
  set(provider: string, credential: Credential): Promise<void> {
    return this.inner.set(provider, credential);
  }
  clear(provider: string): Promise<void> {
    return this.inner.clear(provider);
  }
  list(): Promise<Array<readonly [string, Credential]>> {
    return this.inner.list();
  }
}
