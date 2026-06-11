/**
 * In-process MCP server handed to every Claude Code run. Exposes the whole
 * iii bus to the model as three tools:
 *
 *   mcp__iii__functions_list — engine::functions::list (discover the catalog)
 *   mcp__iii__functions_info — engine::functions::info (one function's schema)
 *   mcp__iii__trigger        — invoke any registered iii function
 *
 * This is what makes the worker bidirectional: iii drives Claude Code via
 * claude::run, and Claude Code drives iii back through this bridge.
 */

import { createSdkMcpServer, tool } from '@anthropic-ai/claude-agent-sdk';
import type { ISdk } from 'iii-sdk';
import { z } from 'zod';

function asText(value: unknown) {
  return {
    content: [{ type: 'text' as const, text: JSON.stringify(value ?? null, null, 2) }],
  };
}

export function makeIiiBridge(iii: ISdk) {
  return createSdkMcpServer({
    name: 'iii',
    version: '0.1.0',
    tools: [
      tool(
        'functions_list',
        'List every function currently registered on the iii engine.',
        {},
        async () =>
          asText(await iii.trigger({ function_id: 'engine::functions::list', payload: {} })),
      ),
      tool(
        'functions_info',
        'Get the description and input schema of one iii function.',
        { function_id: z.string() },
        async ({ function_id }) =>
          asText(
            await iii.trigger({ function_id: 'engine::functions::info', payload: { function_id } }),
          ),
      ),
      tool(
        'trigger',
        'Invoke an iii function with a JSON payload and return its result.',
        {
          function_id: z.string(),
          payload: z.record(z.string(), z.unknown()).default({}),
          timeout_ms: z.number().int().positive().optional(),
        },
        async ({ function_id, payload, timeout_ms }) =>
          asText(
            await iii.trigger({
              function_id,
              payload,
              ...(timeout_ms ? { timeoutMs: timeout_ms } : {}),
            }),
          ),
      ),
    ],
  });
}
