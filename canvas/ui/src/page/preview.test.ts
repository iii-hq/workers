import { describe, expect, it } from 'vitest'
import {
  INITIAL_PREVIEW,
  beginRender,
  parseErrorLine,
  renderFailed,
  renderSucceeded,
} from './preview'

describe('preview state machine', () => {
  it('begins a render by taking ownership of the seq', () => {
    const s = beginRender(INITIAL_PREVIEW, 1)
    expect(s.seq).toBe(1)
    expect(s.rendering).toBe(true)
    expect(s.svg).toBeNull()
  })

  it('ignores out-of-order begins', () => {
    const s = beginRender(INITIAL_PREVIEW, 2)
    expect(beginRender(s, 1)).toBe(s)
    expect(beginRender(s, 2)).toBe(s)
  })

  it('settles a success: svg set, error cleared', () => {
    let s = beginRender(INITIAL_PREVIEW, 1)
    s = renderSucceeded(s, 1, '<svg a/>')
    expect(s).toMatchObject({
      rendering: false,
      svg: '<svg a/>',
      error: null,
      errorLine: null,
    })
  })

  it('keeps the last good svg through a failure', () => {
    let s = renderSucceeded(beginRender(INITIAL_PREVIEW, 1), 1, '<svg a/>')
    s = beginRender(s, 2)
    s = renderFailed(s, 2, 'Parse error on line 4:\nsomething')
    expect(s.svg).toBe('<svg a/>')
    expect(s.error).toContain('Parse error')
    expect(s.errorLine).toBe(4)
    expect(s.rendering).toBe(false)
  })

  it('clears the error on the next success', () => {
    let s = renderFailed(beginRender(INITIAL_PREVIEW, 1), 1, 'bad')
    s = beginRender(s, 2)
    s = renderSucceeded(s, 2, '<svg b/>')
    expect(s.error).toBeNull()
    expect(s.errorLine).toBeNull()
    expect(s.svg).toBe('<svg b/>')
  })

  it('drops stale settlements — a slow old render never wins', () => {
    let s = beginRender(INITIAL_PREVIEW, 1)
    s = beginRender(s, 2)
    // seq 1 settles late, in both flavors: ignored.
    expect(renderSucceeded(s, 1, '<svg stale/>')).toBe(s)
    expect(renderFailed(s, 1, 'stale error')).toBe(s)
    // the owner still settles normally.
    const settled = renderSucceeded(s, 2, '<svg live/>')
    expect(settled.svg).toBe('<svg live/>')
  })
})

describe('parseErrorLine', () => {
  it('extracts the line from mermaid parse errors', () => {
    expect(parseErrorLine('Parse error on line 3:\n...')).toBe(3)
    expect(
      parseErrorLine('Error: Lexical error on line 12. Unrecognized text.'),
    ).toBe(12)
  })

  it('returns null when no line is named', () => {
    expect(parseErrorLine('No diagram type detected')).toBeNull()
    expect(parseErrorLine('on line zero')).toBeNull()
  })
})
