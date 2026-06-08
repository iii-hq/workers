import type { RegisterFunctionOptions, RemoteFunctionHandler } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';
import { PermissionModeSchema, type ApprovalSettings, type PermissionMode } from '../schemas.js';
import type { ISdk } from '../../runtime/iii.js';
import { mutationError, ok } from './reply.js';
import { updateSettings } from './store.js';
import { type MutationReply, sessionIdField } from './types.js';

const PayloadSchema = z.object({
  session_id: sessionIdField,
  mode: PermissionModeSchema,
});

const options = {
  description: 'Set the permission mode (manual | auto | full) for a session.',
  request_format: zodToJsonSchema(PayloadSchema, { name: 'SetModePayload' }),
} as RegisterFunctionOptions;

export async function setMode(
  iii: ISdk,
  session_id: string,
  mode: PermissionMode,
): Promise<ApprovalSettings> {
  // Field-scoped, no read: disjoint from add_always_allow/approve_always so both survive.
  return updateSettings(iii, session_id, [
    { type: 'set', path: 'mode', value: mode },
    { type: 'set', path: 'mode_set_at', value: Date.now() },
  ]);
}

export function registerSetMode(iii: ISdk): void {
  const handler: RemoteFunctionHandler<unknown, ApprovalSettings | MutationReply> = async (
    payload,
  ) => {
    const parsed = PayloadSchema.safeParse(payload);
    if (!parsed.success) return mutationError(parsed.error.message);
    try {
      return ok(await setMode(iii, parsed.data.session_id, parsed.data.mode));
    } catch (err) {
      return mutationError(err);
    }
  };

  iii.registerFunction('approval::set_mode', handler, options);
}
