import { describe, expect, it } from 'vitest';
import type { FunctionResultMessage } from '../../src/types/agent-message.js';
import { formatFunctionResultBlocks, formatFunctionResultContent } from '../../src/types/wire.js';

const baseMsg = (details: unknown, text = 'hello'): FunctionResultMessage => ({
  role: 'function_result',
  function_call_id: 'tc-1',
  function_id: 'shell::exec',
  content: [{ type: 'text', text }],
  details,
  is_error: false,
  timestamp: 0,
});

describe('formatFunctionResultContent', () => {
  it('returns plain text for non-denied results', () => {
    const out = formatFunctionResultContent(baseMsg({ ok: true }, 'output'));
    expect(out).toBe('output');
  });

  it('prefixes [PERMISSION_DENIED] when status is denied', () => {
    const out = formatFunctionResultContent(
      baseMsg(
        {
          schema_version: 1,
          status: 'denied',
          denied_by: 'permissions',
          function_id: 'shell::exec',
          rule_id: 'git/no-force-push',
          reason: 'Permission denied',
        },
        'Permission denied',
      ),
    );
    expect(out.startsWith('[PERMISSION_DENIED]\n')).toBe(true);
    const lines = out.split('\n');
    const json = JSON.parse(lines[1] ?? '');
    expect(json.status).toBe('denied');
    expect(out.endsWith('Permission denied')).toBe(true);
  });

  it('joins multiple text blocks with newline before formatting', () => {
    const msg: FunctionResultMessage = {
      ...baseMsg({}),
      content: [
        { type: 'text', text: 'one' },
        { type: 'text', text: 'two' },
      ],
    };
    expect(formatFunctionResultContent(msg)).toBe('one\ntwo');
  });

  it('skips non-text content blocks', () => {
    const msg: FunctionResultMessage = {
      ...baseMsg({}),
      content: [
        { type: 'text', text: 'kept' },
        { type: 'image', mime: 'image/png', data: 'aGVsbG8=' },
      ],
    };
    expect(formatFunctionResultContent(msg)).toBe('kept');
  });
});

describe('formatFunctionResultBlocks', () => {
  it('returns a single text block for text-only results (same body as the flat string)', () => {
    const blocks = formatFunctionResultBlocks(baseMsg({ ok: true }, 'output'));
    expect(blocks).toEqual([{ type: 'text', text: 'output' }]);
  });

  it('preserves image blocks after the joined text body', () => {
    const msg: FunctionResultMessage = {
      ...baseMsg({}),
      content: [
        { type: 'text', text: 'caption' },
        { type: 'image', mime: 'image/png', data: 'aGVsbG8=' },
      ],
    };
    expect(formatFunctionResultBlocks(msg)).toEqual([
      { type: 'text', text: 'caption' },
      { type: 'image', mime: 'image/png', data: 'aGVsbG8=' },
    ]);
  });

  it('returns only the image block when there is no text', () => {
    const msg: FunctionResultMessage = {
      ...baseMsg({}),
      content: [{ type: 'image', mime: 'image/jpeg', data: 'eA==' }],
    };
    expect(formatFunctionResultBlocks(msg)).toEqual([
      { type: 'image', mime: 'image/jpeg', data: 'eA==' },
    ]);
  });

  it('keeps the [PERMISSION_DENIED] text BEFORE the image block (denied + image)', () => {
    const msg: FunctionResultMessage = {
      ...baseMsg({ status: 'denied', reason: 'no' }, 'denied text'),
      content: [
        { type: 'text', text: 'denied text' },
        { type: 'image', mime: 'image/png', data: 'aGVsbG8=' },
      ],
    };
    const blocks = formatFunctionResultBlocks(msg);
    expect(blocks).toHaveLength(2);
    const first = blocks[0];
    if (first?.type !== 'text') throw new Error('expected a text block first');
    expect(first.text.startsWith('[PERMISSION_DENIED]\n')).toBe(true);
    expect(blocks[1]).toEqual({ type: 'image', mime: 'image/png', data: 'aGVsbG8=' });
  });

  it('keeps the [PERMISSION_DENIED] envelope in the text block', () => {
    const msg: FunctionResultMessage = {
      ...baseMsg({ status: 'denied', reason: 'no' }, 'denied text'),
    };
    const blocks = formatFunctionResultBlocks(msg);
    expect(blocks).toHaveLength(1);
    const first = blocks[0];
    if (first?.type === 'text') {
      expect(first.text.startsWith('[PERMISSION_DENIED]\n')).toBe(true);
    } else {
      throw new Error('expected a text block');
    }
  });
});
