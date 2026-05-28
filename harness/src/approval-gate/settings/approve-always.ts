import type { RegisterFunctionOptions, RemoteFunctionHandler } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';
import type { ApprovalSettings } from '../schemas.js';
import type { ISdk } from '../../runtime/iii.js';
import { type MutationReply, functionIdField, sessionIdField } from './types.js';
import { mutationError, ok } from './reply.js';
import { readSettings, writeSettings } from './store.js';

const PayloadSchema = z.object({
  session_id: sessionIdField,
  function_id: functionIdField,
});

const options = {
  description:
    'Remember an "approve always" decision for this session — the function id stops prompting in every mode. Idempotent.',
  request_format: zodToJsonSchema(PayloadSchema, { name: 'ApproveAlwaysPayload' }),
} as RegisterFunctionOptions;

/** Idempotent on `function_id`; separate from auto-mode `always_allow`. */
export async function approveAlways(
  iii: ISdk,
  session_id: string,
  function_id: string,
): Promise<ApprovalSettings> {
  const current = await readSettings(iii, session_id);
  if (current.approved_always.some((entry) => entry.function_id === function_id)) {
    return current;
  }
  const next: ApprovalSettings = {
    ...current,
    approved_always: [
      ...current.approved_always,
      { function_id, granted_at: Date.now(), granted_by: 'user_click' },
    ],
  };
  await writeSettings(iii, session_id, next);
  return next;
}

export function registerApproveAlways(iii: ISdk): void {
  const handler: RemoteFunctionHandler<unknown, ApprovalSettings | MutationReply> = async (
    payload,
  ) => {
    const parsed = PayloadSchema.safeParse(payload);
    if (!parsed.success) return mutationError(parsed.error.message);
    try {
      return ok(await approveAlways(iii, parsed.data.session_id, parsed.data.function_id));
    } catch (err) {
      return mutationError(err);
    }
  };

  iii.registerFunction('approval::approve_always', handler, options);
}
