import { describe, expect, it } from 'vitest'

import {
  describeWorkerFailure,
  escapeAttr,
  extensionOf,
  failureBlock,
  isWorkerMissing,
  reportDropped,
} from './shared'

describe('describeWorkerFailure', () => {
  /* The browser SDK rejects with a plain object, so `String(err)` produced the
     literal text `[object Object]` — which is exactly what a person saw on the
     chip the first time a conversion failed. */
  it('reads the message out of a non-Error rejection', () => {
    expect(
      describeWorkerFailure({ message: 'malformed document' }, 'document'),
    ).toBe('malformed document')
    expect(
      describeWorkerFailure({ error: 'conversion failed' }, 'document'),
    ).toBe('conversion failed')
    expect(
      describeWorkerFailure(
        { error: { message: 'nested detail' } },
        'document',
      ),
    ).toBe('nested detail')
  })

  it('never renders an object as [object Object]', () => {
    const described = describeWorkerFailure({ status: 500 }, 'document')
    expect(described).not.toContain('[object Object]')
    expect(described).toContain('500')
  })

  it('names the missing worker and how to install it', () => {
    const described = describeWorkerFailure(
      { message: 'function document::to-markdown not found' },
      'document',
    )
    expect(described).toContain('iii call compose::add worker=document')
  })

  it('trims a very long message', () => {
    const described = describeWorkerFailure(new Error('x'.repeat(400)), 'pdf')
    expect(described.length).toBeLessThanOrEqual(160)
    expect(described.endsWith('…')).toBe(true)
  })
})

describe('isWorkerMissing', () => {
  it('knows each way the engine says it, and nothing else', () => {
    expect(isWorkerMissing("'document::to-markdown' is not registered")).toBe(
      true,
    )
    expect(isWorkerMissing({ code: 'NOT_FOUND' })).toBe(true)
    expect(isWorkerMissing('function document::to-markdown not found')).toBe(
      true,
    )
    expect(isWorkerMissing(new Error('malformed document'))).toBe(false)
  })
})

describe('failureBlock', () => {
  /* The header is parsed by finding the first `>`, so an unescaped one in a
     file name would cut it short and lose every attribute after it. */
  it('escapes the characters that would break the header', () => {
    const block = failureBlock('we>ird&"name.docx', 'nope')
    expect(block).toContain('&gt;')
    expect(block).toContain('&amp;')
    expect(block).toContain('&quot;')
    // The only unescaped `>` is the one that closes the block.
    expect(block.indexOf('>')).toBe(block.length - 1)
    const stored = failureBlock('a.docx', 'nope', 'a>1')
    expect(stored.indexOf('>')).toBe(stored.length - 1)
  })

  it('names the stored id and the call that reads it', () => {
    expect(failureBlock('a.docx', 'nope', 'a_1')).toBe(
      '<attached-file path="a.docx" id="a_1" error="nope" read-with="session::read-attachment" />',
    )
  })

  it('names neither when nothing was stored', () => {
    expect(failureBlock('a.docx', 'nope')).toBe(
      '<attached-file path="a.docx" error="nope" />',
    )
  })
})

describe('reportDropped', () => {
  it('passes each stored id to its stub', () => {
    const into = { blocks: [], failures: [] }
    reportDropped(
      [{ name: 'a.pdf', attachmentId: 'a_1' }, { name: 'b.pdf' }],
      'nope',
      into,
    )
    expect(into.blocks).toEqual([
      failureBlock('a.pdf', 'nope', 'a_1'),
      failureBlock('b.pdf', 'nope'),
    ])
  })
})

describe('escapeAttr / extensionOf', () => {
  it('escapes and extracts as the block writers expect', () => {
    expect(escapeAttr('a&b')).toBe('a&amp;b')
    expect(extensionOf('Report.FINAL.DocX')).toBe('docx')
    expect(extensionOf('Makefile')).toBe('')
    expect(extensionOf('.gitignore')).toBe('')
    expect(extensionOf('trailing.')).toBe('')
  })
})
