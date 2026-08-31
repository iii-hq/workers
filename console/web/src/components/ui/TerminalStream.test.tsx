/**
 * Render assertions go through `renderToStaticMarkup` (no jsdom —
 * console/web's convention, see redact-raw.test.tsx); the clamp itself is
 * a pure function tested directly.
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { clampStream, TerminalStream } from './TerminalStream'

const LINES = (n: number) =>
  Array.from({ length: n }, (_, i) => `line ${i + 1}`).join('\n')

describe('clampStream', () => {
  it('passes short text through unclamped', () => {
    expect(clampStream('a\nb', 12, 2000)).toEqual({
      full: 'a\nb',
      shown: 'a\nb',
      clamped: false,
      totalLines: 2,
    })
  })

  it('clamps past the line limit and keeps the true total', () => {
    const { shown, clamped, totalLines } = clampStream(LINES(13), 12, 2000)
    expect(clamped).toBe(true)
    expect(totalLines).toBe(13)
    expect(shown.split('\n')).toHaveLength(12)
    expect(shown).not.toContain('line 13')
  })

  it('clamps a single huge line at the character ceiling', () => {
    const { shown, clamped, totalLines } = clampStream(
      'x'.repeat(3000),
      12,
      2000,
    )
    expect(clamped).toBe(true)
    expect(totalLines).toBe(1)
    expect(shown).toHaveLength(2000)
  })

  it('does not count one trailing newline as a line', () => {
    expect(clampStream('a\nb\n', 12, 2000).totalLines).toBe(2)
    expect(clampStream('a\nb\n', 12, 2000).shown).toBe('a\nb')
  })
})

describe('TerminalStream', () => {
  it('renders the uppercase label and the body', () => {
    const html = renderToStaticMarkup(
      <TerminalStream label="stdout" text="hello" />,
    )
    expect(html).toContain('stdout')
    expect(html).toContain('uppercase')
    expect(html).toContain('hello')
  })

  it('renders nothing for empty text', () => {
    expect(
      renderToStaticMarkup(<TerminalStream label="stdout" text="" />),
    ).toBe('')
  })

  it('shows the expand toggle with the full line count when clamped', () => {
    const html = renderToStaticMarkup(
      <TerminalStream label="stdout" text={LINES(30)} />,
    )
    expect(html).toContain('expand · 30 lines')
    expect(html).not.toContain('line 13')
  })

  it('omits the toggle under the clamp', () => {
    const html = renderToStaticMarkup(
      <TerminalStream label="stdout" text={LINES(3)} />,
    )
    expect(html).not.toContain('expand')
  })

  it('honors custom clamp props', () => {
    const html = renderToStaticMarkup(
      <TerminalStream label="stdout" text={LINES(5)} clampLines={2} />,
    )
    expect(html).toContain('expand · 5 lines')
    expect(html).not.toContain('line 3')
  })

  it('tints the err tone warn, out tone ink', () => {
    const err = renderToStaticMarkup(
      <TerminalStream label="stderr" tone="err" text="boom" />,
    )
    expect(err).toContain('text-warn')
    const out = renderToStaticMarkup(
      <TerminalStream label="stdout" text="fine" />,
    )
    expect(out).toContain('text-ink')
    expect(out).not.toContain('text-warn')
  })

  it('renders ANSI colors through AnsiText when ansi is set', () => {
    const html = renderToStaticMarkup(
      <TerminalStream label="stdout" ansi text={'\u001b[32mpassed\u001b[0m'} />,
    )
    expect(html).toContain('text-ok')
    expect(html).toContain('passed')
    expect(html).not.toContain('\u001b')
  })
})
