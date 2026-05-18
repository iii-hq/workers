/**
 * `policy::check_permissions` — the bus surface for the loaded
 * iii-permissions.yaml. Approval-gate consults this to decide
 * allow / deny / needs_approval for each call.
 */

import { unwrapBody } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';
import { type PermissionsHandle, decisionToValue } from './policy.js';

export function register(iii: ISdk, handle: PermissionsHandle): void {
  iii.registerFunction(
    'policy::check_permissions',
    async (input: unknown) => {
      const body = unwrapBody(input);
      const function_id = typeof body.function_id === 'string' ? body.function_id : null;
      if (!function_id) throw new Error('missing function_id');
      const args = body.args ?? {};
      const decision = handle.current().check(function_id, args);
      return decisionToValue(decision);
    },
    {
      description:
        'Evaluate a function call against the current iii-permissions.yaml. Returns { decision: "allow" | "deny" | "needs_approval", rule_id?, matched_constraint? }.',
    },
  );
}
