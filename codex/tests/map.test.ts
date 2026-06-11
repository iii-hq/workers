import { describe, expect, it } from 'vitest';
import {
  argsForItem,
  functionIdForItem,
  isErrorItem,
  isExecItem,
  lastAssistant,
  makeAssistantMessage,
  makeFunctionResult,
  mapUsage,
  resultContentForItem,
} from '../src/map.js';
import type { AgentMessage } from '../src/types.js';

describe('functionIdForItem', () => {
  it('maps built-in Codex item types to bus-style ids', () => {
    expect(functionIdForItem({ id: 'i', type: 'command_execution' })).toBe('codex::shell');
    expect(functionIdForItem({ id: 'i', type: 'file_change' })).toBe('codex::apply_patch');
    expect(functionIdForItem({ id: 'i', type: 'web_search' })).toBe('codex::web_search');
  });

  it('maps MCP tool calls to server::tool ids', () => {
    expect(
      functionIdForItem({ id: 'i', type: 'mcp_tool_call', server: 'github', tool: 'create_issue' }),
    ).toBe('github::create_issue');
  });
});

describe('isExecItem', () => {
  it('treats command, patch, mcp, and web search items as executions', () => {
    for (const type of ['command_execution', 'file_change', 'mcp_tool_call', 'web_search']) {
      expect(isExecItem({ id: 'i', type })).toBe(true);
    }
    expect(isExecItem({ id: 'i', type: 'agent_message' })).toBe(false);
    expect(isExecItem({ id: 'i', type: 'todo_list' })).toBe(false);
  });
});

describe('argsForItem / resultContentForItem', () => {
  it('carries the command and its aggregated output', () => {
    const item = {
      id: 'i',
      type: 'command_execution',
      command: 'ls -la',
      aggregated_output: 'total 0',
    };
    expect(argsForItem(item)).toEqual({ command: 'ls -la' });
    expect(resultContentForItem(item)).toEqual([{ type: 'text', text: 'total 0' }]);
  });

  it('serializes file changes', () => {
    const changes = [{ path: 'a.ts', kind: 'update' }];
    const item = { id: 'i', type: 'file_change', changes };
    expect(argsForItem(item)).toEqual({ changes });
    expect(resultContentForItem(item)).toEqual([{ type: 'text', text: JSON.stringify(changes) }]);
  });

  it('prefers the MCP error message over the result payload', () => {
    const item = {
      id: 'i',
      type: 'mcp_tool_call',
      server: 's',
      tool: 't',
      error: { message: 'boom' },
      result: { ok: true },
    };
    expect(resultContentForItem(item)).toEqual([{ type: 'text', text: 'boom' }]);
  });
});

describe('isErrorItem', () => {
  it('flags failed status and non-zero exit codes', () => {
    expect(isErrorItem({ id: 'i', type: 'mcp_tool_call', status: 'failed' })).toBe(true);
    expect(
      isErrorItem({ id: 'i', type: 'command_execution', exit_code: 2, status: 'completed' }),
    ).toBe(true);
    expect(
      isErrorItem({ id: 'i', type: 'command_execution', exit_code: 0, status: 'completed' }),
    ).toBe(false);
  });
});

describe('mapUsage', () => {
  it('maps SDK usage fields onto the wire shape', () => {
    expect(
      mapUsage({
        input_tokens: 10,
        cached_input_tokens: 100,
        output_tokens: 5,
        reasoning_output_tokens: 7,
      }),
    ).toEqual({
      input_tokens: 10,
      output_tokens: 5,
      cache_read_tokens: 100,
      reasoning_tokens: 7,
    });
  });

  it('returns null for non-objects', () => {
    expect(mapUsage(null)).toBeNull();
    expect(mapUsage('x')).toBeNull();
  });
});

describe('message constructors', () => {
  it('builds an assistant message with provider codex', () => {
    const msg = makeAssistantMessage([{ type: 'text', text: 'hi' }], 'gpt-5.2-codex', null);
    expect(msg.role).toBe('assistant');
    expect(msg.provider).toBe('codex');
    expect(msg.stop_reason).toBe('end');
  });

  it('builds a function_result message', () => {
    const fr = makeFunctionResult('i', 'codex::shell', [{ type: 'text', text: 'ok' }], false);
    expect(fr.role).toBe('function_result');
    expect(fr.function_call_id).toBe('i');
    expect(fr.is_error).toBe(false);
  });
});

describe('lastAssistant', () => {
  it('returns the last assistant message', () => {
    const a1 = makeAssistantMessage([{ type: 'text', text: 'one' }], 'm', null);
    const a2 = makeAssistantMessage([{ type: 'text', text: 'two' }], 'm', null);
    const fr = makeFunctionResult('id', 'fn', [], false);
    const messages: AgentMessage[] = [a1, a2, fr];
    expect(lastAssistant(messages)).toBe(a2);
  });

  it('falls back to the final message when no assistant exists', () => {
    const fr = makeFunctionResult('id', 'fn', [], false);
    expect(lastAssistant([fr])).toBe(fr);
  });

  it('returns a synthetic assistant message for an empty transcript', () => {
    const msg = lastAssistant([]);
    expect(msg).toMatchObject({ role: 'assistant', content: [] });
  });
});
