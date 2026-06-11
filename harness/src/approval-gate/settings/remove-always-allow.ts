import type { RegisterFunctionOptions, RemoteFunctionHandler } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';
import type { ApprovalSettings } from '../schemas.js';
import type { ISdk } from '../../runtime/iii.js';
import { type MutationReply, functionIdField, sessionIdField } from './types.js';
import { mutationError, ok } from './reply.js';
import { readSettings, updateSettings } from './store.js';

const PayloadSchema = z.object({
  session_id: sessionIdField,
  function_id: functionIdField,
});

const options = {
  description: 'Remove a function id from the per-session always-allow list.',
  request_format: zodToJsonSchema(PayloadSchema, { name: 'RemoveAlwaysAllowPayload' }),
} as RegisterFunctionOptions;

export async function removeAlwaysAllow(
  iii: ISdk,
  session_id: string,
  function_id: string,
): Promise<ApprovalSettings> {
  const current = await readSettings(iii, session_id);
  const always_allow = current.always_allow.filter((entry) => entry.function_id !== function_id);
  // No array-element-remove op, so set just the always_allow field (not the whole
  // record). Known race: concurrent add/remove on this field is last-writer-wins.
  return updateSettings(iii, session_id, [
    { type: 'set', path: 'always_allow', value: always_allow },
  ]);
}

export function registerRemoveAlwaysAllow(iii: ISdk): void {
  const handler: RemoteFunctionHandler<unknown, ApprovalSettings | MutationReply> = async (
    payload,
  ) => {
    const parsed = PayloadSchema.safeParse(payload);
    if (!parsed.success) return mutationError(parsed.error.message);
    try {
      return ok(await removeAlwaysAllow(iii, parsed.data.session_id, parsed.data.function_id));
    } catch (err) {
      return mutationError(err);
    }
  };

  iii.registerFunction('approval::remove_always_allow', handler, options);
}
