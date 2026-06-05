import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Model } from '../../src/models-catalog/types.js';
import { streamOpenai } from '../../src/provider-openai/stream.js';
import type { ChatCompletionsConfig } from '../../src/provider-openai/types.js';

function cfg(overrides: Partial<ChatCompletionsConfig> = {}): ChatCompletionsConfig {
  return {
    url: 'https://api.example/v1/chat/completions',
    provider_name: 'openai',
    model: 'gpt-5',
    api_key: 'sk-test',
    max_tokens: 32_000,
    ...overrides,
  };
}

function catalogModel(extra: Partial<Model> = {}): Model {
  return {
    id: 'gpt-4o',
    provider: 'openai',
    api: 'openai-responses',
    display_name: 'GPT-4o',
    context_window: 128_000,
    ...extra,
  };
}

async function captureBody(
  args: Parameters<typeof streamOpenai>[0],
): Promise<Record<string, unknown>> {
  let captured: Record<string, unknown> | null = null;
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async (_url: string, init: RequestInit) => {
      captured = JSON.parse(init.body as string) as Record<string, unknown>;
      return new Response('data: [DONE]\n\n', { status: 200 });
    }),
  );
  for await (const _ev of streamOpenai(args)) {
    // drain
  }
  if (!captured) throw new Error('fetch was not called');
  return captured;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('streamOpenai request body', () => {
  it('sends the resolved max_completion_tokens', async () => {
    const body = await captureBody({ cfg: cfg(), system_prompt: 's', messages: [], tools: [] });
    expect(body.max_completion_tokens).toBe(32_000);
  });

  it('defaults reasoning_effort to medium for reasoning models', async () => {
    const body = await captureBody({ cfg: cfg(), system_prompt: 's', messages: [], tools: [] });
    expect(body.reasoning_effort).toBe('medium');
  });

  it('maps thinking_level onto reasoning_effort', async () => {
    const body = await captureBody({
      cfg: cfg({ model: 'gpt-5.2' }),
      system_prompt: 's',
      messages: [],
      tools: [],
      thinking_level: 'xhigh',
    });
    expect(body.reasoning_effort).toBe('xhigh');
  });

  it('omits reasoning_effort for non-reasoning models', async () => {
    const body = await captureBody({
      cfg: cfg({ model: 'gpt-4o' }),
      system_prompt: 's',
      messages: [],
      tools: [],
    });
    expect(body).not.toHaveProperty('reasoning_effort');
  });

  it('catalog supports_thinking=true enables reasoning_effort for non-pattern ids', async () => {
    const body = await captureBody({
      cfg: cfg({ model: 'o3-custom', catalog: catalogModel({ supports_thinking: true }) }),
      system_prompt: 's',
      messages: [],
      tools: [],
    });
    expect(body.reasoning_effort).toBe('medium');
  });

  it('catalog supports_thinking=false disables reasoning_effort despite the id pattern', async () => {
    const body = await captureBody({
      cfg: cfg({ model: 'gpt-5', catalog: catalogModel({ supports_thinking: false }) }),
      system_prompt: 's',
      messages: [],
      tools: [],
    });
    expect(body).not.toHaveProperty('reasoning_effort');
  });
});
