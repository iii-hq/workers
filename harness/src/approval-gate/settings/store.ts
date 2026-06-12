import {
  ApprovalSettingsSchema,
  DEFAULT_APPROVAL_SETTINGS,
  SETTINGS_STATE_SCOPE,
  type ApprovalSettings,
} from '../schemas.js';
import type { ISdk } from '../../runtime/iii.js';
import { createState, type UpdateOp } from '../../runtime/state.js';
import { getDefaultMode } from './default-mode.js';

// Reads degrade to defaults on failure (tolerant); writes rethrow so handlers
// can reply with mutationError (strict).
const tolerantState = (iii: ISdk) => createState(iii, { tolerant: true });
const strictState = (iii: ISdk) => createState(iii, { tolerant: false });

/** Default settings; `mode` follows the operator-configured permissions.default_mode. */
function defaultSettings(): ApprovalSettings {
  return { ...DEFAULT_APPROVAL_SETTINGS, mode: getDefaultMode() };
}

/**
 * Backfill missing fields from defaults, then validate. Field-scoped writes can
 * persist a partial record (the first add_always_allow stores only
 * `{ always_allow }`); merging over defaults keeps it valid on read.
 */
export function parseSettings(raw: unknown): ApprovalSettings {
  const base = defaultSettings();
  const merged =
    raw && typeof raw === 'object' && !Array.isArray(raw)
      ? { ...base, ...(raw as Record<string, unknown>) }
      : base;
  const parsed = ApprovalSettingsSchema.safeParse(merged);
  return parsed.success ? parsed.data : base;
}

export async function readSettings(iii: ISdk, session_id: string): Promise<ApprovalSettings> {
  const raw = await tolerantState(iii).get<unknown>({
    scope: SETTINGS_STATE_SCOPE,
    key: session_id,
  });
  return parseSettings(raw);
}

/**
 * Apply field-scoped ops atomically. Each op targets one field, so concurrent
 * mutations of different fields compose under the engine's per-key write-lock
 * instead of clobbering the whole record (the old read/write-whole lost update).
 */
export async function updateSettings(
  iii: ISdk,
  session_id: string,
  ops: UpdateOp[],
): Promise<ApprovalSettings> {
  const result = await strictState(iii).update<unknown>({
    scope: SETTINGS_STATE_SCOPE,
    key: session_id,
    ops,
  });
  if (result?.errors && result.errors.length > 0) {
    throw new Error(`approval-settings update rejected: ${JSON.stringify(result.errors)}`);
  }
  return parseSettings(result?.new_value ?? null);
}

export async function clearSettings(iii: ISdk, session_id: string): Promise<void> {
  // Delete the key rather than write a null tombstone; reads default either way.
  await strictState(iii).delete({ scope: SETTINGS_STATE_SCOPE, key: session_id });
}
