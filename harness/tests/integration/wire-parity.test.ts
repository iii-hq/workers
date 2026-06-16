/**
 * Wire-parity tests. The Node ports must produce JSON byte-identical to
 * the Rust types so a Node harness can run alongside Rust providers and
 * vice-versa during cutover. These tests assert the on-the-wire shape of
 * the most important values.
 */

import { describe, expect, it } from 'vitest';
import { FunctionResolvePayloadSchema } from '../../src/turn-orchestrator/schemas.js';
import { HookOutputSchema, type HookInput } from '../../src/turn-orchestrator/hooks/types.js';
import type { AgentEvent } from '../../src/types/agent-event.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';
import { formatFunctionResultContent } from '../../src/types/wire.js';

describe('AgentEvent wire shape', () => {
  it('turn_end carries message + function_results', () => {
    const asst: AssistantMessage = {
      role: 'assistant',
      content: [{ type: 'text', text: 'hi' }],
      stop_reason: 'end',
      error_message: null,
      error_kind: null,
      usage: null,
      model: 'm',
      provider: 'p',
      timestamp: 0,
    };
    const evt: AgentEvent = {
      type: 'turn_end',
      message: asst,
      function_results: [],
    };
    const j = JSON.parse(JSON.stringify(evt)) as Record<string, unknown>;
    expect(j.type).toBe('turn_end');
    expect(j.function_results).toEqual([]);
    expect((j.message as Record<string, unknown>).role).toBe('assistant');
  });

  it('message_update has llm_event with snake_case type tag', () => {
    const asst: AssistantMessage = {
      role: 'assistant',
      content: [{ type: 'text', text: 'hi' }],
      stop_reason: 'end',
      error_message: null,
      error_kind: null,
      usage: null,
      model: 'm',
      provider: 'p',
      timestamp: 0,
    };
    const evt: AgentEvent = {
      type: 'message_update',
      message: asst,
      llm_event: { type: 'text_delta', partial: asst, delta: 'h' },
    };
    const j = JSON.parse(JSON.stringify(evt)) as Record<string, unknown>;
    expect(j.type).toBe('message_update');
    expect((j.llm_event as Record<string, unknown>).type).toBe('text_delta');
  });
});

describe('FunctionResult / formatFunctionResultContent wire shape', () => {
  it('plain text result returns body unchanged', () => {
    expect(
      formatFunctionResultContent({
        role: 'function_result',
        function_call_id: 'tc',
        function_id: 'shell::exec',
        content: [{ type: 'text', text: 'output' }],
        details: { ok: true },
        is_error: false,
        timestamp: 0,
      }),
    ).toBe('output');
  });

  it('denial wrapped with [PERMISSION_DENIED] marker + JSON envelope + reason', () => {
    const out = formatFunctionResultContent({
      role: 'function_result',
      function_call_id: 'tc',
      function_id: 'shell::exec',
      content: [{ type: 'text', text: 'Permission denied: …' }],
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'permissions',
        function_id: 'shell::exec',
        rule_id: 'git/no-force-push',
        rule_action: 'deny',
        reason: 'Permission denied: …',
      },
      is_error: true,
      timestamp: 0,
    });
    const lines = out.split('\n');
    expect(lines[0]).toBe('[PERMISSION_DENIED]');
    const env = JSON.parse(lines[1] ?? '');
    expect(env.status).toBe('denied');
    expect(env.rule_id).toBe('git/no-force-push');
    expect(out.endsWith('Permission denied: …')).toBe(true);
  });
});

describe('approval-gate worker wire parity', () => {
  // These shapes must match the Rust worker's serde forms
  // (approval-gate/src/types.rs): HookInput/HookOutput and the
  // harness::function::resolve payloads it sends.

  it('HookInput matches what approval::gate deserializes', () => {
    const input: HookInput = {
      point: 'pre_trigger',
      session_id: 's_1',
      turn_id: 't_1',
      step: 3,
      depth: 0,
      call: { id: 'c_1', function_id: 'shell::run', arguments: { cmd: 'ls' } },
    };
    expect(JSON.parse(JSON.stringify(input))).toEqual({
      point: 'pre_trigger',
      session_id: 's_1',
      turn_id: 't_1',
      step: 3,
      depth: 0,
      call: { id: 'c_1', function_id: 'shell::run', arguments: { cmd: 'ls' } },
    });
  });

  it('HookOutput accepts the three Rust serde forms and rejects garbage', () => {
    expect(HookOutputSchema.parse({ decision: 'continue' })).toEqual({ decision: 'continue' });
    expect(HookOutputSchema.parse({ decision: 'deny', reason: 'nope' })).toEqual({
      decision: 'deny',
      reason: 'nope',
    });
    expect(HookOutputSchema.parse({ decision: 'hold', pending_timeout_ms: 1_800_000 })).toEqual({
      decision: 'hold',
      pending_timeout_ms: 1_800_000,
    });
    expect(HookOutputSchema.safeParse('what even is this').success).toBe(false);
    expect(HookOutputSchema.safeParse({ decision: 'hold' }).success).toBe(false);
  });

  it('function::resolve accepts the exact payloads the Rust resolve/sweep send', () => {
    // approval-gate/src/functions/resolve.rs — allow:
    expect(
      FunctionResolvePayloadSchema.parse({
        session_id: 's_1',
        turn_id: 't_9',
        function_call_id: 'c_1',
        action: 'execute',
      }).action,
    ).toBe('execute');

    // approval-gate/src/functions/resolve.rs — deny (rendered DenialEnvelope):
    const deny = FunctionResolvePayloadSchema.parse({
      session_id: 's_1',
      turn_id: 't_9',
      function_call_id: 'c_1',
      action: 'deliver',
      is_error: true,
      content: [{ type: 'text', text: 'too risky' }],
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'user',
        function_id: 'shell::run',
        args_excerpt: { cmd: 'ls' },
        reason: 'too risky',
      },
    });
    expect(deny.is_error).toBe(true);
    expect(deny.content?.[0]).toEqual({ type: 'text', text: 'too risky' });
  });
});
