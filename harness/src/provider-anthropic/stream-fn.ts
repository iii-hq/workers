/**
 * Phase 2.A: `provider::anthropic::stream`. Takes a `ProviderStreamInput`
 * (with `writer_ref`), pushes each AssistantMessageEvent as a JSON text
 * message, closes the channel, returns `ProviderStreamOutput`.
 *
 * Mirrors the contract spelled out in `PHASE-2-PLAN.md` §4.
 */

import type { ChannelWriter } from 'iii-sdk';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import {
  ProviderStreamInputJsonSchema,
  ProviderStreamOutputJsonSchema,
  ProviderStreamRuntimeInputSchema,
} from '../types/provider.js';
import type { AssistantMessageEvent } from '../types/stream-event.js';
import { isTerminal } from '../types/stream-event.js';
import { buildConfig } from './auth.js';
import type { WorkerConfig } from './config.js';
import { streamAnthropic } from './stream.js';
import { buildThinkingConfig } from './thinking.js';

export const FUNCTION_ID = 'provider::anthropic::stream';

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (raw: unknown) => {
      const input = ProviderStreamRuntimeInputSchema.parse(raw);
      const writer = input.writer_ref as ChannelWriter;
      const cfg = await buildConfig(
        iii,
        worker,
        input.model,
        input.model_meta,
        input.resolution_key,
        input.max_output_tokens,
      );
      const thinking = buildThinkingConfig(input.thinking_level, cfg.max_tokens, cfg.catalog);
      try {
        const events = streamAnthropic({
          cfg,
          system_prompt: input.system_prompt ?? '',
          messages: input.messages as AgentMessage[],
          tools: input.tools as import('../types/function.js').AgentFunction[],
          ...(thinking ? { thinking } : {}),
        });
        for await (const ev of events) {
          writer.sendMessage(JSON.stringify(ev));
          if (isTerminal(ev as AssistantMessageEvent)) break;
        }
      } catch (err) {
        logger.warn('provider::anthropic::stream failed mid-flight', {
          err: String(err),
        });
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
        'Stream a single assistant turn from Anthropic into the caller-supplied channel. Each AssistantMessageEvent is sent as a JSON text message; the terminal event is Done or Error followed by close.',
      request_format: ProviderStreamInputJsonSchema as Record<string, unknown>,
      response_format: ProviderStreamOutputJsonSchema as Record<string, unknown>,
    },
  );
}
