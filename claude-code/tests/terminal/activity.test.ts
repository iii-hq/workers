import { describe, expect, it } from 'vitest';
import { ActivityTracker, callId, toolFunctionId } from '../../src/terminal/activity.js';
import type { AgentEvent } from '../../src/terminal/types.js';

function tracker() {
  const emitted: { session: string; event: AgentEvent }[] = [];
  const emit = async (session: string, event: AgentEvent) => {
    emitted.push({ session, event });
  };
  return { emitted, tracker: new ActivityTracker(emit) };
}

describe('tool naming', () => {
  it('keeps an MCP tool with its own server and claims the rest', () => {
    expect(toolFunctionId('Write')).toBe('claude::Write');
    expect(toolFunctionId('mcp__github__create_issue')).toBe('github::create_issue');
    expect(toolFunctionId('')).toBe('claude::tool');
  });

  it('pairs a Pre with its Post even when the CLI sends no tool_use_id', () => {
    const pre = { tool_name: 'Bash', tool_input: { command: 'npm test' } };
    const post = { tool_name: 'Bash', tool_input: { command: 'npm test' } };
    expect(callId(pre)).toBe(callId(post));
    expect(callId({ tool_use_id: 'toolu_1', tool_name: 'Bash' })).toBe('toolu_1');
    expect(callId(pre)).not.toBe(callId({ tool_name: 'Bash', tool_input: { command: 'ls' } }));
  });
});

describe('a terminal turn becomes AgentEvent frames', () => {
  it('maps prompt, tool call, and stop', async () => {
    const { emitted, tracker: subject } = tracker();
    const session_id = 'session-1';

    await subject.handle({ hook_event_name: 'SessionStart', session_id });
    await subject.handle({
      hook_event_name: 'UserPromptSubmit',
      session_id,
      prompt: 'add a health endpoint',
    });
    await subject.handle({
      hook_event_name: 'PreToolUse',
      session_id,
      tool_use_id: 'toolu_1',
      tool_name: 'Write',
      tool_input: { file_path: '/ws/health.ts' },
    });
    await subject.handle({
      hook_event_name: 'PostToolUse',
      session_id,
      tool_use_id: 'toolu_1',
      tool_name: 'Write',
      tool_response: { success: true },
    });
    await subject.handle({ hook_event_name: 'Stop', session_id });

    expect(emitted.map((e) => e.event.type)).toEqual([
      'message_complete',
      'message_complete',
      'function_execution_start',
      'function_execution_end',
      'turn_end',
      'agent_end',
    ]);
    expect(emitted.every((e) => e.session === session_id)).toBe(true);

    const prompt = emitted[0].event;
    if (prompt.type !== 'message_complete') throw new Error('expected the prompt first');
    expect(prompt.message).toMatchObject({
      role: 'user',
      content: [{ type: 'text', text: 'add a health endpoint' }],
    });

    const start = emitted[2].event;
    if (start.type !== 'function_execution_start') throw new Error('expected a call start');
    expect(start).toMatchObject({
      function_call_id: 'toolu_1',
      function_id: 'claude::Write',
      args: { file_path: '/ws/health.ts' },
    });

    const end = emitted[3].event;
    if (end.type !== 'function_execution_end') throw new Error('expected a call end');
    expect(end.is_error).toBe(false);
    expect(end.duration_ms).toBeGreaterThanOrEqual(0);

    const turn = emitted[4].event;
    if (turn.type !== 'turn_end') throw new Error('expected turn_end');
    expect(turn.function_results).toHaveLength(1);

    const agentEnd = emitted[5].event;
    if (agentEnd.type !== 'agent_end') throw new Error('expected agent_end');
    // user prompt + assistant tool call + tool result + closing assistant
    expect(agentEnd.messages.map((m) => m.role)).toEqual([
      'user',
      'assistant',
      'function_result',
      'assistant',
    ]);
  });

  it('reads a failed tool from the response the hook carries', async () => {
    const { emitted, tracker: subject } = tracker();
    await subject.handle({
      hook_event_name: 'PreToolUse',
      session_id: 's',
      tool_use_id: 'toolu_2',
      tool_name: 'Bash',
      tool_input: { command: 'npm test' },
    });
    await subject.handle({
      hook_event_name: 'PostToolUse',
      session_id: 's',
      tool_use_id: 'toolu_2',
      tool_name: 'Bash',
      tool_response: { exit_code: 1, stderr: 'FAIL src/app.test.ts' },
    });

    const end = emitted.at(-1)?.event;
    if (end?.type !== 'function_execution_end') throw new Error('expected a call end');
    expect(end.is_error).toBe(true);
    expect(JSON.stringify(end.result.content)).toContain('FAIL src/app.test.ts');
  });

  it('reports a Post with no Pre rather than dropping it', async () => {
    const { emitted, tracker: subject } = tracker();
    await subject.handle({
      hook_event_name: 'PostToolUse',
      session_id: 's',
      tool_name: 'Read',
      tool_input: { file_path: '/ws/x.ts' },
      tool_response: 'contents',
    });
    const end = emitted.at(-1)?.event;
    if (end?.type !== 'function_execution_end') throw new Error('expected a call end');
    expect(end.function_id).toBe('claude::Read');
    expect(end.duration_ms).toBe(0);
  });

  it('answers every event, including ones it does not map', async () => {
    const { tracker: subject } = tracker();
    expect(await subject.handle({ hook_event_name: 'Notification', session_id: 's' })).toEqual({
      ok: true,
      event: 'Notification',
    });
    expect(await subject.handle({})).toEqual({ ok: true, event: 'unknown' });
  });
});
