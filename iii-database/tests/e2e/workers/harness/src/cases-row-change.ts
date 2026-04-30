import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

/**
 * row-change trigger validation. The streaming decoder is stubbed in v1.0
 * (worker rejects `iii-database::row-change` registration with `UNSUPPORTED`),
 * so we can't exercise the dispatch path end-to-end yet. What we CAN validate
 * is the slot/publication name derivation contract that the worker pins in
 * its README — distinct caller-supplied `trigger_id`s must produce distinct
 * Postgres replication-slot names so two registrations don't silently share
 * one slot once the streaming runtime ships.
 *
 * Pre-fix: `derive_names` lowercased and replaced non-alnum with `_`, so
 * `orders-v1` and `orders.v1` both became `orders_v1` and the second
 * registration would silently reuse the first slot. Post-fix: an FNV-1a-32
 * hash of the original trigger_id is appended, guaranteeing uniqueness.
 *
 * This file mirrors the Rust `derive_names` algorithm in TS so we can
 * compute the same names the worker would and assert Postgres treats them
 * as distinct identifiers via `pg_create_logical_replication_slot`.
 */

/** FNV-1a-32 over UTF-8 bytes. Mirrors `triggers/row_change.rs::fnv1a_32`. */
function fnv1a32(s: string): string {
  let hash = 0x811c9dc5 >>> 0;
  const bytes = Buffer.from(s, 'utf8');
  for (const b of bytes) {
    hash = (hash ^ b) >>> 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, '0');
}

/**
 * TS port of `triggers/row_change.rs::derive_names`. Lowercases ASCII
 * alphanumerics, replaces every other char with `_`, truncates the
 * sanitized prefix at 40 chars to fit Postgres' 63-byte slot_name limit,
 * and appends an 8-hex-char FNV-1a-32 hash of the *original* trigger_id.
 */
function deriveSlotName(triggerId: string): string {
  const sanitized = Array.from(triggerId)
    .map((c) => (/[a-zA-Z0-9]/.test(c) ? c.toLowerCase() : '_'))
    .slice(0, 40)
    .join('');
  return `iii_slot_${sanitized}_${fnv1a32(triggerId)}`;
}

async function dropSlotIfExists(
  call: (id: string, payload: unknown) => Promise<any>,
  driver: string,
  slot: string,
): Promise<void> {
  // pg_drop_replication_slot errors if the slot is missing; pre-check then drop.
  // Quote-escape the slot name as a SQL literal: replace any `'` with `''`
  // (slot names from derive_names are `[a-z0-9_]` only, so this is defensive).
  const lit = slot.replace(/'/g, "''");
  const exists = await call('iii-database::query', {
    db: driver,
    sql: `SELECT 1 FROM pg_replication_slots WHERE slot_name = '${lit}'`,
  });
  if (exists.row_count > 0) {
    await call('iii-database::execute', {
      db: driver,
      sql: `SELECT pg_drop_replication_slot('${lit}')`,
    });
  }
}

export const ROW_CHANGE_CASES: TestCase[] = [
  {
    name: 'row-change derive_names: collision-prone trigger_ids produce distinct postgres slots',
    applies: ['pg_db'],
    async run({ driver, call }) {
      // These three inputs all sanitized to `orders_v1` in the pre-fix code
      // (lowercase + replace non-alnum with `_`). Post-fix, the appended hash
      // makes them distinct. We use 3 (not the full 5) because the docker
      // postgres image is configured with `max_replication_slots=4`, leaving
      // headroom for the long-trigger-id test that runs immediately after.
      const ids = ['Orders.v1', 'orders-v1', 'orders v1'];
      const slots = ids.map(deriveSlotName);

      // Sanity: TS-derived names must all be distinct.
      const unique = new Set(slots);
      expectEqual(unique.size, ids.length, 'TS-derived slot names must be unique across collision-prone inputs');

      // Each slot must respect Postgres' 63-byte limit.
      for (const s of slots) {
        expect(s.length <= 63, `slot name too long (${s.length} bytes): ${s}`);
      }

      // Pre-clean any leftovers from a previous run.
      for (const slot of slots) {
        await dropSlotIfExists(call, driver, slot);
      }

      try {
        // Create all five slots. If two collided, the second create call would
        // fail with `replication slot ... already exists`.
        for (const slot of slots) {
          await call('iii-database::execute', {
            db: driver,
            sql: `SELECT * FROM pg_create_logical_replication_slot('${slot}', 'pgoutput')`,
          });
        }

        // Verify Postgres now lists all five as distinct slots.
        const inList = slots.map((s) => `'${s}'`).join(', ');
        const q = await call('iii-database::query', {
          db: driver,
          sql: `SELECT slot_name FROM pg_replication_slots WHERE slot_name IN (${inList}) ORDER BY slot_name`,
        });
        expectEqual(q.row_count, ids.length, 'all collision-prone inputs produced distinct slots in postgres');
      } finally {
        // Cleanup so re-running the harness against the same docker volume is idempotent.
        for (const slot of slots) {
          try {
            await dropSlotIfExists(call, driver, slot);
          } catch {
            /* best-effort cleanup */
          }
        }
      }
    },
  },
  {
    name: 'row-change derive_names: long trigger_id stays within postgres slot-name limit',
    applies: ['pg_db'],
    async run({ driver, call }) {
      // Pathological trigger_id: 200 chars. Without truncation the derived
      // name would exceed Postgres' 63-byte slot_name cap and slot creation
      // would fail; the hash suffix preserves uniqueness across the truncation.
      const a = 'a'.repeat(200);
      const b = 'a'.repeat(200) + 'b'; // distinct trigger_id, same first-40 sanitized prefix
      const slotA = deriveSlotName(a);
      const slotB = deriveSlotName(b);

      expect(slotA !== slotB, `long trigger_ids collided: ${slotA}`);
      expect(slotA.length <= 63, `slotA too long (${slotA.length}): ${slotA}`);
      expect(slotB.length <= 63, `slotB too long (${slotB.length}): ${slotB}`);

      // Pre-clean.
      await dropSlotIfExists(call, driver, slotA);
      await dropSlotIfExists(call, driver, slotB);

      try {
        await call('iii-database::execute', {
          db: driver,
          sql: `SELECT * FROM pg_create_logical_replication_slot('${slotA}', 'pgoutput')`,
        });
        await call('iii-database::execute', {
          db: driver,
          sql: `SELECT * FROM pg_create_logical_replication_slot('${slotB}', 'pgoutput')`,
        });
        const q = await call('iii-database::query', {
          db: driver,
          sql: `SELECT slot_name FROM pg_replication_slots WHERE slot_name IN ('${slotA}', '${slotB}')`,
        });
        expectEqual(q.row_count, 2, 'long-trigger-id slots created and distinct');
      } finally {
        try { await dropSlotIfExists(call, driver, slotA); } catch { /* best-effort */ }
        try { await dropSlotIfExists(call, driver, slotB); } catch { /* best-effort */ }
      }
    },
  },
];
