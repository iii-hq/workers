import { describe, expect, it } from 'vitest'
import {
  PROMPT_NAME_RE,
  parsePromptEntries,
  parsePromptWithBody,
} from './prompts'

/**
 * Parser contract against the iii-directory worker's wire shapes
 * (`directory::prompts::list` / `::get`). Strategy tolerance matters: older
 * workers omit `strategy` entirely and must degrade to `enrich`, never drop
 * the row.
 */

const ENTRY = {
  name: 'code-reviewer',
  description: 'Reviews diffs.',
  strategy: 'override',
  modified_at: '2026-07-21T12:00:00Z',
}

describe('parsePromptEntries', () => {
  it('parses a full row', () => {
    const rows = parsePromptEntries([ENTRY])
    expect(rows).toHaveLength(1)
    expect(rows[0]?.strategy).toBe('override')
  })

  it('defaults strategy to enrich when absent (older workers) or unknown', () => {
    expect(parsePromptEntries([{ name: 'a' }])[0]?.strategy).toBe('enrich')
    expect(
      parsePromptEntries([{ name: 'a', strategy: 'bananas' }])[0]?.strategy,
    ).toBe('enrich')
  })

  it('drops invalid rows instead of failing the whole list', () => {
    expect(
      parsePromptEntries([ENTRY, { description: 'no name' }, null]),
    ).toHaveLength(1)
  })
})

describe('parsePromptWithBody', () => {
  it('requires a body', () => {
    expect(parsePromptWithBody(ENTRY)).toBeNull()
    expect(parsePromptWithBody({ ...ENTRY, body: 'Be strict.' })?.body).toBe(
      'Be strict.',
    )
  })

  it('degrades unknown strategy to enrich', () => {
    expect(
      parsePromptWithBody({ name: 'a', body: 'b', strategy: 'wat' })?.strategy,
    ).toBe('enrich')
  })
})

describe('PROMPT_NAME_RE', () => {
  it('mirrors the server-side name rule', () => {
    expect(PROMPT_NAME_RE.test('code-reviewer')).toBe(true)
    expect(PROMPT_NAME_RE.test('v2_final')).toBe(true)
    expect(PROMPT_NAME_RE.test('Bad Name')).toBe(false)
    expect(PROMPT_NAME_RE.test('(none)')).toBe(false)
    expect(PROMPT_NAME_RE.test('')).toBe(false)
    expect(PROMPT_NAME_RE.test('x'.repeat(65))).toBe(false)
  })
})
