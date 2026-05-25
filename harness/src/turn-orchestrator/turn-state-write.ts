/**
 * UI notification when agent-scope turn_state is persisted via `saveRecord` /
 * `persistRecord`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { emit } from './events.js';
import type { TurnStateView } from './schemas.js';

export async function emitTurnStateChanged(
  iii: ISdk,
  session_id: string,
  event_type: 'state:created' | 'state:updated',
  new_value: TurnStateView,
  old_value?: TurnStateView,
): Promise<void> {
  try {
    await emit(iii, session_id, {
      type: 'turn_state_changed',
      event_type,
      new_value,
      ...(old_value !== undefined && { old_value }),
    });
  } catch (err) {
    logger.warn('emitTurnStateChanged failed', {
      session_id,
      err: String(err),
    });
  }
}
