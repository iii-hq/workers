import { afterEach, describe, expect, it, vi } from 'vitest';
import { streamAnthropic } from '../../src/provider-anthropic/stream.js';
import type { AnthropicConfig } from '../../src/provider-anthropic/types.js';

function cfg(overrides: Partial<AnthropicConfig> = {}): AnthropicConfig {
  return {
    credential_value: 'sk-test',
    model: 'claude-sonnet-4-6',
    max_tokens: 32_000,
    api_url: 'https://api.example/v1/messages',
    auth_mode: 'api_key',
    ...overrides,
  };
}

type Captured = { body: Record<string, unknown>; headers: Record<string, string> };

async function capture(args: Parameters<typeof streamAnthropic>[0]): Promise<Captured> {
  let captured: Captured | null = null;
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async (_url: string, init: RequestInit) => {
      captured = {
        body: JSON.parse(init.body as string) as Record<string, unknown>,
        headers: init.headers as Record<string, string>,
      };
      return new Response('data: {"type":"message_stop"}\n\n', { status: 200 });
    }),
  );
  for await (const _ev of streamAnthropic(args)) {
    // drain
  }
  if (!captured) throw new Error('fetch was not called');
  return captured;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('streamAnthropic request body', () => {
  it('sends the resolved max_tokens and no temperature', async () => {
    const { body } = await capture({ cfg: cfg(), system_prompt: 's', messages: [], tools: [] });
    expect(body.max_tokens).toBe(32_000);
    expect(body).not.toHaveProperty('temperature');
    expect(body).not.toHaveProperty('thinking');
  });

  it('omits the anthropic-beta header when thinking is off', async () => {
    const { headers } = await capture({ cfg: cfg(), system_prompt: 's', messages: [], tools: [] });
    expect(headers).not.toHaveProperty('anthropic-beta');
  });

  it('sends thinking + interleaved-thinking beta header when enabled', async () => {
    const { body, headers } = await capture({
      cfg: cfg(),
      system_prompt: 's',
      messages: [],
      tools: [],
      thinking: { type: 'enabled', budget_tokens: 16_000 },
    });
    expect(body.thinking).toEqual({ type: 'enabled', budget_tokens: 16_000 });
    expect(headers['anthropic-beta']).toBe('interleaved-thinking-2025-05-14');
    expect(body).not.toHaveProperty('temperature');
  });

  it('keeps the thinking budget below max_tokens (invariant carried by the caller)', async () => {
    const { body } = await capture({
      cfg: cfg({ max_tokens: 32_000 }),
      system_prompt: 's',
      messages: [],
      tools: [],
      thinking: { type: 'enabled', budget_tokens: 31_999 },
    });
    expect(body.max_tokens).toBe(32_000);
    const thinking = body.thinking as { budget_tokens: number };
    expect(thinking.budget_tokens).toBeLessThan(body.max_tokens as number);
  });
});
