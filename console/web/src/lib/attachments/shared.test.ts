import { describe, expect, it } from 'vitest'

import {
  describeWorkerFailure,
  escapeAttr,
  extensionOf,
  failureBlock,
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
    expect(described).toContain('iii trigger compose::add worker=document')
  })

  it('trims a very long message', () => {
    const described = describeWorkerFailure(new Error('x'.repeat(400)), 'pdf')
    expect(described.length).toBeLessThanOrEqual(160)
    expect(described.endsWith('…')).toBe(true)
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
