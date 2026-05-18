/**
 * Iii-backed state bus + an in-memory implementation for tests.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

export interface StateBus {
  set(scope: string, key: string, value: unknown): Promise<void>;
  get(scope: string, key: string): Promise<unknown | null>;
  listPrefix(scope: string, prefix: string): Promise<unknown[]>;
}

export class IiiStateBus implements StateBus {
  constructor(private readonly iii: ISdk) {}

  async set(scope: string, key: string, value: unknown): Promise<void> {
    await this.iii.trigger<unknown, unknown>({
      function_id: 'state::set',
      payload: { scope, key, value },
    });
  }

  async get(scope: string, key: string): Promise<unknown | null> {
    try {
      const v = await this.iii.trigger<unknown, unknown>({
        function_id: 'state::get',
        payload: { scope, key },
      });
      return v === null || v === undefined ? null : v;
    } catch (err) {
      logger.debug('state::get failed', { scope, key, err: String(err) });
      return null;
    }
  }

  async listPrefix(scope: string, prefix: string): Promise<unknown[]> {
    try {
      const resp = await this.iii.trigger<unknown, unknown>({
        function_id: 'state::list',
        payload: { scope, prefix },
      });
      // The engine's state::list returns the values directly as an array.
      // Earlier harness versions returned `{items: [...]}` so accept both.
      const items = Array.isArray(resp)
        ? resp
        : Array.isArray((resp as { items?: unknown[] } | null)?.items)
          ? ((resp as { items: unknown[] }).items as unknown[])
          : [];
      return items.map((entry) => {
        if (entry && typeof entry === 'object' && 'value' in (entry as Record<string, unknown>)) {
          return (entry as Record<string, unknown>).value;
        }
        return entry;
      });
    } catch (err) {
      logger.debug('state::list failed', { scope, prefix, err: String(err) });
      return [];
    }
  }
}

export class InMemoryStateBus implements StateBus {
  private readonly store = new Map<string, unknown>();
  async set(scope: string, key: string, value: unknown): Promise<void> {
    this.store.set(`${scope}/${key}`, value);
  }
  async get(scope: string, key: string): Promise<unknown | null> {
    return this.store.get(`${scope}/${key}`) ?? null;
  }
  async listPrefix(scope: string, prefix: string): Promise<unknown[]> {
    const out: unknown[] = [];
    const fullPrefix = `${scope}/${prefix}`;
    for (const [k, v] of this.store) {
      if (k.startsWith(fullPrefix)) out.push(v);
    }
    return out;
  }
}
