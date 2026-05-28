import type { RegisterFunctionOptions, RemoteFunctionHandler } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';
import type { ISdk } from '../../runtime/iii.js';
import { mutationError } from './reply.js';
import { clearSettings } from './store.js';
import { type MutationReply, sessionIdField } from './types.js';

const PayloadSchema = z.object({
  session_id: sessionIdField,
});

const options = {
  description: 'Clear the per-session approval settings on conversation deletion.',
  request_format: zodToJsonSchema(PayloadSchema, { name: 'ClearSettingsPayload' }),
} as RegisterFunctionOptions;

export function registerClearSettings(iii: ISdk): void {
  const handler: RemoteFunctionHandler<unknown, MutationReply> = async (payload) => {
    const parsed = PayloadSchema.safeParse(payload);
    if (!parsed.success) return mutationError(parsed.error.message);
    try {
      await clearSettings(iii, parsed.data.session_id);
      return { ok: true };
    } catch (err) {
      return mutationError(err);
    }
  };

  iii.registerFunction('approval::clear_settings', handler, options);
}
