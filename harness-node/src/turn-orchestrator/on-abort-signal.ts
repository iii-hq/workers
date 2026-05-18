import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

export const STEP_TOPIC = 'turn::step_requested';
export const HANDLER_FN_ID = 'turn::on_abort_signal';
export const CONDITION_FN_ID = 'turn::is_abort_signal_set';
const ABORT_SIGNAL_KEY_RE = /^session\/([^/]+)\/abort_signal$/;

export function isAbortSignalWrite(event: unknown): boolean {
  if (!event || typeof event !== 'object') return false;
  const obj = event as Record<string, unknown>;
  if (obj.event_type !== 'state:created' && obj.event_type !== 'state:updated') return false;
  if (obj.new_value !== true) return false;
  const key = obj.key;
  if (typeof key !== 'string') return false;
  return ABORT_SIGNAL_KEY_RE.test(key);
}

function extractSessionId(key: string): string | null {
  const m = ABORT_SIGNAL_KEY_RE.exec(key);
  return m ? (m[1] ?? null) : null;
}

export async function handleAbortSignalWrite(iii: ISdk, event: unknown): Promise<void> {
  if (!event || typeof event !== 'object') return;
  const obj = event as Record<string, unknown>;
  const key = obj.key;
  if (typeof key !== 'string') return;
  const session_id = extractSessionId(key);
  if (!session_id) return;

  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'iii::durable::publish',
      payload: { topic: STEP_TOPIC, data: { session_id } },
    });
  } catch (err) {
    logger.warn('turn::on_abort_signal: publish failed', {
      session_id,
      err: String(err),
    });
  }
}
