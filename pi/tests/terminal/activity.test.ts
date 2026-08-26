import { describe, expect, it } from 'vitest';
import { ActivityTracker } from '../../src/terminal/activity.js';
import type { AgentEvent } from '../../src/types.js';

function tracker() {
  const emitted: { session: string; event: AgentEvent }[] = [];
  const emit = async (session: string, event: AgentEvent) => {
    emitted.push({ session, event });
  };
  return { emitted, tracker: new ActivityTracker(emit) };
}

describe('a pi run becomes AgentEvent frames', () => {
  it('maps the prompt, a tool call, and the end of the run', async () => {
    const { emitted, tracker: subject } = tracker();
    const session_id = 'pi-1';

    await subject.handle({ event: 'session_start', session_id, cwd: '/ws' });
    await subject.handle({ event: 'agent_start', session_id, prompt: 'fix the flaky test' });
    await subject.handle({
      event: 'tool_start',
      session_id,
      call_id: 'call-1',
      tool: 'edit',
      args: { path: '/ws/app.ts' },
    });
    await subject.handle({
      event: 'tool_end',
      session_id,
      call_id: 'call-1',
      tool: 'edit',
      result: { content: [{ type: 'text', text: 'patched 2 lines' }] },
    });
    await subject.handle({ event: 'agent_end', session_id });

    expect(emitted.map((e) => e.event.type)).toEqual([
      'message_complete',
      'message_complete',
      'function_execution_start',
      'function_execution_end',
      'turn_end',
      'agent_end',
    ]);

    const start = emitted[2].event;
    if (start.type !== 'function_execution_start') throw new Error('expected a call start');
    expect(start).toMatchObject({ function_call_id: 'call-1', function_id: 'pi::edit' });

    const end = emitted[3].event;
    if (end.type !== 'function_execution_end') throw new Error('expected a call end');
    expect(end.is_error).toBe(false);
    // pi returns content blocks; the text is what a reader wants.
    expect(end.result.content).toEqual([{ type: 'text', text: 'patched 2 lines' }]);
  });

  it('marks a failed tool call', async () => {
    const { emitted, tracker: subject } = tracker();
    await subject.handle({ event: 'tool_start', session_id: 's', call_id: 'c', tool: 'bash' });
    await subject.handle({
      event: 'tool_end',
      session_id: 's',
      call_id: 'c',
      tool: 'bash',
      is_error: true,
      result: 'exit 1',
    });
    const end = emitted.at(-1)?.event;
    if (end?.type !== 'function_execution_end') throw new Error('expected a call end');
    expect(end.is_error).toBe(true);
  });

  it('pairs a call that carries no id of its own', async () => {
    const { emitted, tracker: subject } = tracker();
    await subject.handle({ event: 'tool_start', session_id: 's', tool: 'bash' });
    await subject.handle({ event: 'tool_end', session_id: 's', tool: 'bash', result: 'ok' });

    const start = emitted[1]?.event;
    const end = emitted.at(-1)?.event;
    if (start?.type !== 'function_execution_start') throw new Error('expected a call start');
    if (end?.type !== 'function_execution_end') throw new Error('expected a call end');
    // Same id on both halves, or the console shows a call that never ends.
    expect(end.function_call_id).toBe(start.function_call_id);
    expect(end.function_id).toBe('pi::bash');

    // A second call to the same tool is a second call, not the first one again.
    await subject.handle({ event: 'tool_start', session_id: 's', tool: 'bash' });
    const again = emitted.at(-1)?.event;
    if (again?.type !== 'function_execution_start') throw new Error('expected a call start');
    expect(again.function_call_id).not.toBe(start.function_call_id);
  });

  it('answers every event, including ones it does not map', async () => {
    const { tracker: subject } = tracker();
    expect(await subject.handle({ event: 'compaction', session_id: 's' })).toEqual({
      ok: true,
      event: 'compaction',
    });
    expect(await subject.handle({})).toEqual({ ok: true, event: 'unknown' });
  });
});
