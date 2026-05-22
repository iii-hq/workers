import { describe, expect, it } from 'vitest';
import {
  contentBlockToWire,
  decodeToolName,
  encodeToolName,
  toWireMessages,
} from '../../src/provider-anthropic/wire-messages.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

describe('encode/decodeToolName', () => {
  it('replaces :: with __', () => {
    expect(encodeToolName('shell::fs::ls')).toBe('shell__fs__ls');
    expect(decodeToolName('shell__fs__ls')).toBe('shell::fs::ls');
  });
});

describe('contentBlockToWire', () => {
  it('encodes function_call as tool_use with encoded name', () => {
    const out = contentBlockToWire({
      type: 'function_call',
      id: 'tc1',
      function_id: 'shell::exec',
      arguments: { x: 1 },
    });
    expect(out).toEqual({
      type: 'tool_use',
      id: 'tc1',
      name: 'shell__exec',
      input: { x: 1 },
    });
  });

  it('skips other block kinds', () => {
    expect(
      contentBlockToWire({
        type: 'image',
        mime: 'image/png',
        data: 'aGk=',
      }),
    ).toBeNull();
  });
});

describe('toWireMessages', () => {
  it('converts user message to wire user', () => {
    const msgs: AgentMessage[] = [
      { role: 'user', content: [{ type: 'text', text: 'hi' }], timestamp: 0 },
    ];
    const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
    expect(wire[0]?.role).toBe('user');
    expect((wire[0] as { content: Array<{ type: string }> }).content[0]?.type).toBe('text');
  });

  it('collapses parallel function_results into one user message with tool_result blocks', () => {
    const mk = (id: string): AgentMessage => ({
      role: 'function_result',
      function_call_id: id,
      function_id: 'read',
      content: [{ type: 'text', text: id }],
      details: {},
      is_error: false,
      timestamp: 0,
    });
    const wire = toWireMessages([mk('a'), mk('b'), mk('c')]) as Array<Record<string, unknown>>;
    expect(wire.length).toBe(1);
    const content = (wire[0] as { content: Array<{ tool_use_id: string }> }).content;
    expect(content).toHaveLength(3);
    expect(content[0]?.tool_use_id).toBe('a');
    expect(content[2]?.tool_use_id).toBe('c');
  });

  it('skips custom messages', () => {
    const msgs: AgentMessage[] = [
      {
        role: 'custom',
        custom_type: 'note',
        content: [{ type: 'text', text: 'x' }],
        timestamp: 0,
      },
    ];
    expect(toWireMessages(msgs)).toEqual([]);
  });

  describe('boundary dedup of duplicate tool_result blocks', () => {
    // Production failure: messages.20.content.1: each tool_use must have a
    // single result. Found multiple tool_result blocks with id: toolu_...
    // Root cause is orchestrator re-entry. The wire layer also defends.

    const mkResult = (id: string, text: string): AgentMessage => ({
      role: 'function_result',
      function_call_id: id,
      function_id: 'shell::run',
      content: [{ type: 'text', text }],
      details: {},
      is_error: false,
      timestamp: 0,
    });

    it('collapses two function_results with the same id into one tool_result block (latest wins)', () => {
      const wire = toWireMessages([
        mkResult('toolu_01', 'first attempt'),
        mkResult('toolu_01', 'second attempt'),
      ]) as Array<Record<string, unknown>>;
      expect(wire.length).toBe(1);
      const content = (wire[0] as { content: Array<{ tool_use_id: string; content: string }> })
        .content;
      expect(content).toHaveLength(1);
      expect(content[0]?.tool_use_id).toBe('toolu_01');
      expect(content[0]?.content).toBe('second attempt');
    });

    it('preserves order of distinct tool_use_ids while deduping repeats', () => {
      const wire = toWireMessages([
        mkResult('a', 'A1'),
        mkResult('b', 'B1'),
        mkResult('a', 'A2'),
        mkResult('c', 'C1'),
        mkResult('b', 'B2'),
      ]) as Array<Record<string, unknown>>;
      const content = (wire[0] as { content: Array<{ tool_use_id: string; content: string }> })
        .content;
      expect(content.map((b) => b.tool_use_id)).toEqual(['a', 'b', 'c']);
      expect(content[0]?.content).toBe('A2');
      expect(content[1]?.content).toBe('B2');
      expect(content[2]?.content).toBe('C1');
    });

    it('dedup is scoped to the pending batch (across-batch repeats stay separate)', () => {
      // [function_result(a), assistant_msg, function_result(a)] produces
      // two SEPARATE user messages each with tool_result(a) — correct
      // wire format. Dedup must NOT collapse across the flush boundary.
      const msgs: AgentMessage[] = [
        mkResult('a', 'r1'),
        {
          role: 'assistant',
          content: [{ type: 'text', text: 'thinking…' }],
          stop_reason: 'end',
          model: 'claude',
          provider: 'anthropic',
          timestamp: 0,
        },
        mkResult('a', 'r2'),
      ];
      const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
      expect(wire).toHaveLength(3);
      expect((wire[0] as { role: string }).role).toBe('user');
      expect((wire[1] as { role: string }).role).toBe('assistant');
      expect((wire[2] as { role: string }).role).toBe('user');
    });
  });

  describe('boundary sanitization of orphan tool_use blocks', () => {
    // Production failure: messages.53: `tool_use` IDs were found without
    // `tool_result` blocks immediately after: toolu_01D7RTIXU6PXH7ER9PSRHH98.
    // Three orchestrator paths can leave a tool_use without its matching
    // function_result in flat-state (abort during awaiting_approval, sync
    // /compact mid-execution, prepared-call desync). The wire layer
    // defends by injecting a synthetic tool_result placeholder.

    const mkAsstWithToolUse = (id: string, fnId = 'shell::run'): AgentMessage => ({
      role: 'assistant',
      content: [
        {
          type: 'function_call',
          id,
          function_id: fnId,
          arguments: { command: 'echo hi' },
        },
      ],
      stop_reason: 'function_call',
      model: 'claude',
      provider: 'anthropic',
      timestamp: 0,
    });

    const mkResult = (id: string, text: string): AgentMessage => ({
      role: 'function_result',
      function_call_id: id,
      function_id: 'shell::run',
      content: [{ type: 'text', text }],
      details: {},
      is_error: false,
      timestamp: 0,
    });

    it('injects a synthetic tool_result placeholder for an orphan tool_use at end of conversation', async () => {
      // Exact production shape: flat-state ends with assistant(tool_use)
      // because handleFinalize never ran for that call.
      const msgs: AgentMessage[] = [
        {
          role: 'user',
          content: [{ type: 'text', text: 'do a thing' }],
          timestamp: 0,
        },
        mkAsstWithToolUse('toolu_orphan'),
      ];
      const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
      // [user, assistant(tool_use), user(synthetic tool_result)]
      expect(wire).toHaveLength(3);
      expect((wire[2] as { role: string }).role).toBe('user');
      const content = (wire[2] as { content: Array<Record<string, unknown>> }).content;
      expect(content).toHaveLength(1);
      expect(content[0]?.type).toBe('tool_result');
      expect(content[0]?.tool_use_id).toBe('toolu_orphan');
      expect(content[0]?.is_error).toBe(true);
      expect(content[0]?.content as string).toMatch(/interrupted before completing/i);
    });

    it('does NOT inject anything when every tool_use already has a function_result (no-op happy path)', () => {
      const msgs: AgentMessage[] = [
        {
          role: 'user',
          content: [{ type: 'text', text: 'hi' }],
          timestamp: 0,
        },
        mkAsstWithToolUse('toolu_a'),
        mkResult('toolu_a', 'output A'),
      ];
      const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
      expect(wire).toHaveLength(3);
      const content = (wire[2] as { content: Array<Record<string, unknown>> }).content;
      expect(content).toHaveLength(1);
      expect(content[0]?.tool_use_id).toBe('toolu_a');
      expect(content[0]?.is_error).toBe(false);
      // The synthetic placeholder text must NOT appear.
      expect(content[0]?.content as string).not.toMatch(/interrupted before completing/i);
    });

    it('mixes synthetic placeholders alongside real tool_results when only SOME tool_uses are orphaned', () => {
      // Assistant emits two tool_uses in one turn; only one runs to
      // completion, the other was interrupted.
      const msgs: AgentMessage[] = [
        {
          role: 'user',
          content: [{ type: 'text', text: 'do both' }],
          timestamp: 0,
        },
        {
          role: 'assistant',
          content: [
            {
              type: 'function_call',
              id: 'toolu_done',
              function_id: 'shell::run',
              arguments: {},
            },
            {
              type: 'function_call',
              id: 'toolu_orphan',
              function_id: 'shell::run',
              arguments: {},
            },
          ],
          stop_reason: 'function_call',
          model: 'claude',
          provider: 'anthropic',
          timestamp: 0,
        },
        mkResult('toolu_done', 'completed output'),
      ];
      const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
      expect(wire).toHaveLength(3);
      const content = (wire[2] as { content: Array<Record<string, unknown>> }).content;
      expect(content).toHaveLength(2);
      const byId = new Map(content.map((b) => [b.tool_use_id as string, b]));
      expect(byId.get('toolu_done')?.is_error).toBe(false);
      expect(byId.get('toolu_done')?.content).toBe('completed output');
      expect(byId.get('toolu_orphan')?.is_error).toBe(true);
      expect(byId.get('toolu_orphan')?.content as string).toMatch(/interrupted before completing/i);
    });

    it('injects placeholders in turn-order across multiple orphan turns', () => {
      // Long agentic session where two earlier turns each left an orphan.
      // The synthetic for toolu_x merges into the next user message
      // (with q2's text), preventing consecutive user messages.
      const msgs: AgentMessage[] = [
        {
          role: 'user',
          content: [{ type: 'text', text: 'q1' }],
          timestamp: 0,
        },
        mkAsstWithToolUse('toolu_x'),
        {
          role: 'user',
          content: [{ type: 'text', text: 'q2' }],
          timestamp: 0,
        },
        mkAsstWithToolUse('toolu_y'),
      ];
      const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
      expect(wire.map((m) => (m as { role: string }).role)).toEqual([
        'user', // q1
        'assistant', // tool_use_x
        'user', // synthetic tool_result_x + q2 text MERGED
        'assistant', // tool_use_y
        'user', // synthetic tool_result_y at end
      ]);
      // The merged user at index 2 carries BOTH the synthetic placeholder
      // AND the q2 text content.
      const m2 = wire[2] as { content: Array<Record<string, unknown>> };
      const placeholderBlock = m2.content.find(
        (b) =>
          b.type === 'tool_result' && (b as { tool_use_id?: string }).tool_use_id === 'toolu_x',
      );
      expect(placeholderBlock).toBeDefined();
      const q2TextBlock = m2.content.find(
        (b) => b.type === 'text' && (b as { text?: string }).text === 'q2',
      );
      expect(q2TextBlock).toBeDefined();
      // Tool_result must come BEFORE the user text (Anthropic convention).
      const placeholderIdx = m2.content.findIndex((b) => b.type === 'tool_result');
      const textIdx = m2.content.findIndex((b) => b.type === 'text');
      expect(placeholderIdx).toBeLessThan(textIdx);
      // The final user (after toolu_y assistant) carries the toolu_y placeholder.
      const m4 = wire[4] as { content: Array<Record<string, unknown>> };
      expect((m4.content[0] as { tool_use_id: string }).tool_use_id).toBe('toolu_y');
    });

    it('does not re-inject a placeholder if the same orphan id appears twice (defensive)', () => {
      // Pathological: same tool_use_id in two assistant turns and no
      // function_result for either. The first injection marks the id
      // resolved so the second skips. Anthropic gets one placeholder for
      // the id; the second tool_use will be unmatched in its own batch
      // but at least we don't generate two placeholders.
      const msgs: AgentMessage[] = [mkAsstWithToolUse('toolu_dup'), mkAsstWithToolUse('toolu_dup')];
      const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
      let placeholderCount = 0;
      for (const m of wire) {
        const role = (m as { role: string }).role;
        if (role !== 'user') continue;
        const content = (m as { content: Array<Record<string, unknown>> }).content;
        for (const b of content) {
          if (
            b.type === 'tool_result' &&
            typeof b.content === 'string' &&
            /interrupted/i.test(b.content)
          ) {
            placeholderCount++;
          }
        }
      }
      expect(placeholderCount).toBe(1);
    });
  });
});
