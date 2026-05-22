import type { ChannelWriter } from 'iii-sdk';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import {
  ProviderStreamInputJsonSchema,
  ProviderStreamOutputJsonSchema,
  ProviderStreamRuntimeInputSchema,
} from '../types/provider.js';
import { isTerminal } from '../types/stream-event.js';
import { buildConfig } from './auth.js';
import type { WorkerConfig } from './config.js';
import { streamLmstudio } from './stream.js';

export const FUNCTION_ID = 'provider::lmstudio::stream';

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (raw: unknown) => {
      const input = ProviderStreamRuntimeInputSchema.parse(raw);
      // The iii-sdk auto-hydrates `writer_ref` (a StreamChannelRef on the
      // wire) into a `ChannelWriter` instance before this handler runs.
      const writer = input.writer_ref as ChannelWriter;
      const cfg = await buildConfig(iii, worker, input.model);
      try {
        const events = streamLmstudio({
          cfg,
          system_prompt: input.system_prompt ?? '',
          messages: input.messages as AgentMessage[],
          tools: input.tools as import('../types/function.js').AgentFunction[],
        });
        for await (const ev of events) {
          writer.sendMessage(JSON.stringify(ev));
          if (isTerminal(ev)) break;
        }
      } catch (err) {
        logger.warn('provider::lmstudio::stream failed mid-flight', { err: String(err) });
      } finally {
        try {
          writer.close();
        } catch (err) {
          logger.debug('writer.close failed', { err: String(err) });
        }
      }
      return { ok: true };
    },
    {
      description:
        'Stream a single assistant turn from a local LM Studio Chat Completions server into the caller-supplied channel.',
      request_format: ProviderStreamInputJsonSchema as Record<string, unknown>,
      response_format: ProviderStreamOutputJsonSchema as Record<string, unknown>,
    },
  );
}
