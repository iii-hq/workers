/**
 * Pins the fiddly branches of the shared grep highlighter: the literal
 * (isRegex: false) path must never fire regex metacharacters, broken
 * regexes fall back to substring matching, zero-width matches are
 * skipped instead of looping, the highlight-count cap stops marking
 * but keeps the tail text, no-match lines pass through unhighlighted,
 * and case-insensitive substring slicing preserves the line's
 * original casing.
 */
import { isValidElement, type ReactNode } from 'react'
import { describe, expect, it } from 'vitest'
import { renderWithHighlight } from '../highlight'

/** Flatten the ReactNode result into round-trip text + highlighted runs. */
function flatten(node: ReactNode): { text: string; highlights: string[] } {
  const parts = Array.isArray(node) ? node : [node]
  const highlights: string[] = []
  let text = ''
  for (const part of parts) {
    if (typeof part === 'string') {
      text += part
    } else if (isValidElement<{ children: string }>(part)) {
      text += part.props.children
      highlights.push(part.props.children)
    }
  }
  return { text, highlights }
}

describe('renderWithHighlight', () => {
  it('returns the line untouched for an empty query', () => {
    expect(
      renderWithHighlight('keep me', '', { isRegex: false, ignoreCase: false }),
    ).toBe('keep me')
  })

  it('passes the line through unhighlighted when nothing matches', () => {
    const literal = flatten(
      renderWithHighlight('no hits here', 'absent', {
        isRegex: false,
        ignoreCase: false,
      }),
    )
    expect(literal.text).toBe('no hits here')
    expect(literal.highlights).toEqual([])

    const regex = flatten(
      renderWithHighlight('no hits here', 'absent\\d+', {
        isRegex: true,
        ignoreCase: false,
      }),
    )
    expect(regex.text).toBe('no hits here')
    expect(regex.highlights).toEqual([])
  })

  it('treats the query as a literal when isRegex is false', () => {
    // "a.b" as a regex would also match "axb"; the literal path must not.
    const { text, highlights } = flatten(
      renderWithHighlight('axb a.b axb', 'a.b', {
        isRegex: false,
        ignoreCase: false,
      }),
    )
    expect(text).toBe('axb a.b axb')
    expect(highlights).toEqual(['a.b'])
  })

  it('matches as a regex when isRegex is true', () => {
    const { text, highlights } = flatten(
      renderWithHighlight('TODO: fix FIXME later', 'TODO|FIXME', {
        isRegex: true,
        ignoreCase: false,
      }),
    )
    expect(text).toBe('TODO: fix FIXME later')
    expect(highlights).toEqual(['TODO', 'FIXME'])
  })

  it('falls back to substring matching when the regex does not compile', () => {
    // "(" is invalid JS regex; Rust `regex` would reject it too, but the
    // fallback keeps the view honest for any engine mismatch.
    const { text, highlights } = flatten(
      renderWithHighlight('call(arg)', '(', {
        isRegex: true,
        ignoreCase: false,
      }),
    )
    expect(text).toBe('call(arg)')
    expect(highlights).toEqual(['('])
  })

  it('skips zero-width regex matches without dropping real ones', () => {
    // "x*" matches the empty string at every position; only the real "x"
    // run should highlight and the call must terminate.
    const { text, highlights } = flatten(
      renderWithHighlight('axxb', 'x*', { isRegex: true, ignoreCase: false }),
    )
    expect(text).toBe('axxb')
    expect(highlights).toEqual(['xx'])
  })

  it('renders no highlights when every regex match is zero-width', () => {
    const { text, highlights } = flatten(
      renderWithHighlight('bbb', 'a*', { isRegex: true, ignoreCase: false }),
    )
    expect(text).toBe('bbb')
    expect(highlights).toEqual([])
  })

  it('caps highlighted matches but keeps the unhighlighted tail', () => {
    const line = 'a'.repeat(300)
    const { text, highlights } = flatten(
      renderWithHighlight(line, 'a', { isRegex: true, ignoreCase: true }),
    )
    // The loop breaks once the counter passes 200, so exactly 201 runs
    // are marked; the remaining text still round-trips unhighlighted.
    expect(highlights).toHaveLength(201)
    expect(text).toBe(line)
  })

  it('preserves original casing when slicing case-insensitive substrings', () => {
    const { text, highlights } = flatten(
      renderWithHighlight('ToDo and TODO, todone', 'todo', {
        isRegex: false,
        ignoreCase: true,
      }),
    )
    expect(text).toBe('ToDo and TODO, todone')
    expect(highlights).toEqual(['ToDo', 'TODO', 'todo'])
  })
})
