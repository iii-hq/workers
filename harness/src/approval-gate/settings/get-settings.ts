import type { RegisterFunctionOptions, RemoteFunctionHandler } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';
import type { ISdk } from '../../runtime/iii.js';
import { readSettings } from './store.js';
import { sessionIdField, type GetSettingsReply } from './types.js';

const PayloadSchema = z.object({
  session_id: sessionIdField,
});

const options = {
  description: 'Read the per-session approval settings; returns defaults when none persisted.',
  request_format: zodToJsonSchema(PayloadSchema, { name: 'GetSettingsPayload' }),
} as RegisterFunctionOptions;

export function registerGetSettings(iii: ISdk): void {
  const handler: RemoteFunctionHandler<unknown, GetSettingsReply> = async (payload) => {
    const parsed = PayloadSchema.parse(payload);
    return readSettings(iii, parsed.session_id);
  };

  iii.registerFunction('approval::get_settings', handler, options);
}
