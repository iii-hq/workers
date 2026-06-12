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
import { streamOpenai } from './stream.js';

export const FUNCTION_ID = 'provider::openai::stream';

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (raw: unknown) => {
      const input = ProviderStreamRuntimeInputSchema.parse(raw);
      // The iii-sdk auto-hydrates `writer_ref` (a StreamChannelRef on the
      // wire) into a `ChannelWriter` instance before this handler runs —
      // use it directly instead of re-instantiating from the wire shape
      // (which no longer exists by the time we get here).
      const writer = input.writer_ref as ChannelWriter;
      const cfg = await buildConfig(iii, worker, input.model, input.max_output_tokens);
      try {
        const events = streamOpenai({
          cfg,
          system_prompt: input.system_prompt ?? '',
          messages: input.messages as AgentMessage[],
          tools: input.tools as import('../types/function.js').AgentFunction[],
          ...(input.thinking_level ? { thinking_level: input.thinking_level } : {}),
        });
        for await (const ev of events) {
          writer.sendMessage(JSON.stringify(ev));
          if (isTerminal(ev)) break;
        }
      } catch (err) {
        logger.warn('provider::openai::stream failed mid-flight', { err: String(err) });
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
        'Stream a single assistant turn from OpenAI Chat Completions into the caller-supplied channel.',
      request_format: ProviderStreamInputJsonSchema as Record<string, unknown>,
      response_format: ProviderStreamOutputJsonSchema as Record<string, unknown>,
    },
  );
}
