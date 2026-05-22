import { requireString } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';
import type { AgentMessage } from '../types/agent-message.js';
import type { AgentFunction } from '../types/function.js';
import { buildConfig } from './auth.js';
import type { WorkerConfig } from './config.js';
import { collect, streamKimi } from './stream.js';

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    'provider::kimi::complete',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const model = requireString(obj, 'model');
      const system_prompt = typeof obj.system_prompt === 'string' ? obj.system_prompt : '';
      const messages = Array.isArray(obj.messages) ? (obj.messages as AgentMessage[]) : [];
      const tools = Array.isArray(obj.tools) ? (obj.tools as AgentFunction[]) : [];
      const cfg = await buildConfig(iii, worker, model);
      return await collect(streamKimi({ cfg, system_prompt, messages, tools }));
    },
    {
      description:
        'Legacy: drain a streamed Kimi chat-completion and return the final AssistantMessage.',
    },
  );
}
