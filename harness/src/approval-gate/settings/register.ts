/**
 * Per-session approval settings: permission mode (manual / auto / full),
 * the auto-mode `always_allow` list, and the per-session
 * `approved_always` grants ("approve always" decisions made from a
 * prompt, honored in every mode).
 *
 * Settings live in shared state under scope `approval_settings`, keyed by
 * `<session_id>`. `policy::check_permissions` reads a snapshot at the
 * start of each check; mutations made during a check do not affect that
 * check. The handlers registered here are the only writers — every entry
 * carries `granted_by: 'user_click'` so audit can distinguish user grants
 * from any future programmatic source.
 *
 * The handlers are intentionally usable only from the frontend RPC path.
 * Agent-issued calls funnel through `consultBefore`, which carries a
 * hardcoded deny for these function ids; user-initiated SDK calls
 * (e.g. `approval::set_mode` from `console/web`) bypass the hook and
 * land here directly.
 */

import type { ISdk } from '../../runtime/iii.js';
import { registerAddAlwaysAllow } from './add-always-allow.js';
import { registerApproveAlways } from './approve-always.js';
import { registerClearSettings } from './clear-settings.js';
import { registerGetSettings } from './get-settings.js';
import { registerRemoveAlwaysAllow } from './remove-always-allow.js';
import { registerSetMode } from './set-mode.js';

export function registerSettingsHandlers(iii: ISdk): void {
  registerSetMode(iii);
  registerAddAlwaysAllow(iii);
  registerRemoveAlwaysAllow(iii);
  registerApproveAlways(iii);
  registerGetSettings(iii);
  registerClearSettings(iii);
}
