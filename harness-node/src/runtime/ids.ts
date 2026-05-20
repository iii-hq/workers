/** Tiny id / time helpers used across workers. */

import { randomUUID } from 'node:crypto';

let counter = 0n;

/**
 * Process-wide unique-ish id. Mirrors the Rust `uuid_like()` helper used
 * for stream `item_id`s and synthetic `function_call_id`s. Seeds from a
 * UUIDv4 so collisions across processes are negligible.
 */
export function uuidLike(): string {
  const seq = (counter++).toString(16);
  const t = Date.now().toString(16);
  return `${t}-${seq}-${randomUUID().slice(0, 8)}`;
}

export function nowMs(): number {
  return Date.now();
}

export function nowSecs(): number {
  return Math.floor(Date.now() / 1000);
}
