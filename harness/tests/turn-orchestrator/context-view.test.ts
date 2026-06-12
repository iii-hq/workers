import { describe, expect, it } from 'vitest';
import { buildSummaryMessage } from '../../src/context-compaction/flat-state.js';
import { buildContextView } from '../../src/turn-orchestrator/state-runtime/context-view.js';
import type { MessageWithEntryId } from '../../src/runtime/session.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

function user(text: string): AgentMessage {
  return { role: 'user', content: [{ type: 'text', text }], timestamp: 0 };
}

function asst(text: string): AgentMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text }],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model: 'm',
    provider: 'p',
    timestamp: 0,
  };
}

function entry(id: string, message: AgentMessage): MessageWithEntryId {
  return { entry_id: id, message };
}

describe('buildContextView', () => {
  it('returns raw path when there is no compaction', () => {
    const messages = [entry('a', user('one')), entry('b', asst('two'))];
    expect(buildContextView(messages, [])).toEqual([user('one'), asst('two')]);
  });

  it('drops empty assistant placeholders from the provider window', () => {
    const placeholder: AgentMessage = { ...asst(''), content: [] };
    const messages = [entry('a', user('one')), entry('b', placeholder)];
    expect(buildContextView(messages, [])).toEqual([user('one')]);
  });

  it('reconstructs summary + tail from tail_start_id', () => {
    const messages = [
      entry('head', user('old')),
      entry('tail1', asst('keep')),
      entry('last', user('in flight')),
    ];
    const compactions = [{ summary: 'condensed', tail_start_id: 'tail1', timestamp: 100 }];

    expect(buildContextView(messages, compactions)).toEqual([
      buildSummaryMessage('condensed'),
      asst('keep'),
      user('in flight'),
    ]);
  });

  it('uses the latest compaction when several exist', () => {
    const messages = [
      entry('h', user('old')),
      entry('t1', asst('early tail')),
      entry('t2', user('recent')),
    ];
    const compactions = [
      { summary: 'first', tail_start_id: 'h', timestamp: 10 },
      { summary: 'latest', tail_start_id: 't2', timestamp: 20 },
    ];

    expect(buildContextView(messages, compactions)).toEqual([
      buildSummaryMessage('latest'),
      user('recent'),
    ]);
  });

  it('keeps the whole tail when tail_start_id is absent from the path', () => {
    const messages = [entry('a', user('one')), entry('b', asst('two'))];
    const compactions = [{ summary: 's', tail_start_id: 'gone', timestamp: 1 }];

    expect(buildContextView(messages, compactions)).toEqual([
      buildSummaryMessage('s'),
      user('one'),
      asst('two'),
    ]);
  });
});
