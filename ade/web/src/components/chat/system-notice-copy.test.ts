import { describe, expect, it } from 'vitest'
import { sentenceStart, splitNotice } from './system-notice-copy'

describe('sentenceStart', () => {
  it('raises a plain leading word', () => {
    expect(sentenceStart('compacting session…')).toBe('Compacting session…')
    expect(sentenceStart('compact: another compaction is in progress')).toBe(
      'Compact: another compaction is in progress',
    )
  })

  it('leaves slash commands, identifiers, and paths as authored', () => {
    expect(sentenceStart('/compact not supported by this backend.')).toBe(
      '/compact not supported by this backend.',
    )
    expect(sentenceStart('max_turns (8) reached')).toBe('max_turns (8) reached')
    expect(sentenceStart('/tmp/x is gone')).toBe('/tmp/x is gone')
    expect(sentenceStart('openai::gpt-5 unavailable')).toBe(
      'openai::gpt-5 unavailable',
    )
  })
})

describe('splitNotice', () => {
  it('splits "headline — detail" at the first spaced dash', () => {
    expect(
      splitNotice('could not attach spec.pdf — file exceeds the 20 MB limit'),
    ).toEqual({
      headline: 'Could not attach spec.pdf',
      detail: 'file exceeds the 20 MB limit',
    })
  })

  it('keeps a single-clause notice whole', () => {
    expect(splitNotice('select a model before sending.')).toEqual({
      headline: 'Select a model before sending.',
    })
  })

  it('does not split when the would-be headline is a paragraph', () => {
    const long = `${'a'.repeat(120)} — tail`
    expect(splitNotice(long)).toEqual({ headline: `A${long.slice(1)}` })
  })
})
