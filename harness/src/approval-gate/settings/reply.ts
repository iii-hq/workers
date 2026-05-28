import type { MutationReply } from './types.js';

export function ok<T>(value: T): T {
  return value;
}

export function mutationError(err: unknown): MutationReply {
  return { ok: false, error: String(err) };
}
