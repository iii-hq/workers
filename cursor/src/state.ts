import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import { SessionRecordSchema, type SessionRecord } from './types.js';

const SCOPE = 'cursor_sessions';
const CompareAndSetResponseSchema = z.object({
  swapped: z.boolean(),
  current: z.unknown().optional(),
});

export type SessionSwap =
  | { swapped: true; record: SessionRecord }
  | { swapped: false; current: SessionRecord | null };

export async function loadSession(
  iii: IIIClient,
  sessionId: string,
): Promise<SessionRecord | null> {
  const value = await iii.trigger<unknown, unknown>({
    function_id: 'state::get',
    payload: { scope: SCOPE, key: sessionId },
  });
  if (value == null) return null;
  return SessionRecordSchema.parse(value);
}

export async function compareAndSetSession(
  iii: IIIClient,
  expected: SessionRecord | null,
  record: SessionRecord,
): Promise<SessionSwap> {
  const response = CompareAndSetResponseSchema.parse(
    await iii.trigger<unknown, unknown>({
      function_id: 'state::compare-and-set',
      payload: {
        scope: SCOPE,
        key: record.session_id,
        ...(expected ? { expected } : {}),
        value: SessionRecordSchema.parse(record),
      },
    }),
  );
  if (response.swapped) return { swapped: true, record };
  if (response.current == null) return { swapped: false, current: null };
  return { swapped: false, current: SessionRecordSchema.parse(response.current) };
}

export async function updateSession(
  iii: IIIClient,
  sessionId: string,
  update: (current: SessionRecord) => SessionRecord | null,
): Promise<SessionRecord | null> {
  let current = await loadSession(iii, sessionId);
  for (let attempt = 0; current && attempt < 8; attempt += 1) {
    const next = update(structuredClone(current));
    if (!next) return current;
    const result = await compareAndSetSession(iii, current, next);
    if (result.swapped) return next;
    current = result.current;
    if (current) {
      await new Promise((resolvePromise) =>
        setTimeout(resolvePromise, 2 + Math.floor(Math.random() * 7)),
      );
    }
  }
  if (!current) return null;
  throw new Error(`Cursor session ${sessionId} changed too frequently to update safely`);
}

export async function listSessions(iii: IIIClient): Promise<SessionRecord[]> {
  const value = await iii.trigger<unknown, unknown>({
    function_id: 'state::list',
    payload: { scope: SCOPE },
  });
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry) => {
    const parsed = SessionRecordSchema.safeParse(entry);
    return parsed.success ? [parsed.data] : [];
  });
}
