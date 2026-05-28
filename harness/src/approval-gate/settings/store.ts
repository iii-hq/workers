import {
  ApprovalSettingsSchema,
  DEFAULT_APPROVAL_SETTINGS,
  SETTINGS_STATE_SCOPE,
  type ApprovalSettings,
} from '../schemas.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';

export async function readSettings(iii: ISdk, session_id: string): Promise<ApprovalSettings> {
  try {
    const raw = await iii.trigger<unknown, unknown>({
      function_id: 'state::get',
      payload: { scope: SETTINGS_STATE_SCOPE, key: session_id },
    });
    const parsed = ApprovalSettingsSchema.safeParse(raw);
    return parsed.success ? parsed.data : DEFAULT_APPROVAL_SETTINGS;
  } catch (err) {
    logger.warn('approval-settings read failed; using defaults', {
      session_id,
      err: String(err),
    });
    return DEFAULT_APPROVAL_SETTINGS;
  }
}

export async function writeSettings(
  iii: ISdk,
  session_id: string,
  settings: ApprovalSettings,
): Promise<void> {
  await iii.trigger({
    function_id: 'state::set',
    payload: { scope: SETTINGS_STATE_SCOPE, key: session_id, value: settings },
  });
}

export async function clearSettings(iii: ISdk, session_id: string): Promise<void> {
  await iii.trigger({
    function_id: 'state::set',
    payload: { scope: SETTINGS_STATE_SCOPE, key: session_id, value: null },
  });
}
