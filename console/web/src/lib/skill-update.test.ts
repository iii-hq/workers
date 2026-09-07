import { describe, expect, it } from 'vitest'
import { parseSkillUpdate, skillUpdateSummary } from './skill-update'

const BLOCK = [
  'The available skills have changed. This list supersedes the previous',
  'available skills list.',
  '<available_skills>',
  'A `<skill id="...">` block is already loaded; follow it directly without searching for or reloading it.',
  'For any listed skill not already loaded, call the pre-verified `directory::skills::get` directly with payload `{"id":"<exact id>"}`.',
  '- **console** — The iii web console.',
  '- **console/injectable-ui** — Build worker UI &lt;at runtime&gt; &amp; hot reload.',
  '- **fp**',
  '</available_skills>',
].join('\n')

describe('parseSkillUpdate', () => {
  it('turns the harness block into id/description rows, unescaped', () => {
    expect(parseSkillUpdate(BLOCK)).toEqual({
      available: true,
      entries: [
        { id: 'console', description: 'The iii web console.' },
        {
          id: 'console/injectable-ui',
          description: 'Build worker UI <at runtime> & hot reload.',
        },
        { id: 'fp', description: '' },
      ],
    })
  })

  it('ignores the preamble lines that are not rows', () => {
    const ids = parseSkillUpdate(BLOCK)?.entries.map((e) => e.id)
    expect(ids).not.toContain('skill id="..."')
    expect(ids).toHaveLength(3)
  })

  it('recognises the withdrawal message', () => {
    expect(
      parseSkillUpdate(
        'Skill guidance is no longer available. Do not use any previously listed skill.',
      ),
    ).toEqual({ available: false, entries: [] })
  })

  it('returns null for text that is not a skill update', () => {
    expect(parseSkillUpdate('The available skills have changed.')).toBeNull()
    expect(parseSkillUpdate('hello')).toBeNull()
  })

  it('yields an empty list for a block without rows', () => {
    expect(
      parseSkillUpdate('<available_skills>\nnothing here\n</available_skills>'),
    ).toEqual({ available: true, entries: [] })
  })
})

describe('skillUpdateSummary', () => {
  it('counts and pluralises', () => {
    expect(skillUpdateSummary({ available: true, entries: [] })).toBe(
      'updated · 0 available',
    )
    expect(
      skillUpdateSummary({
        available: true,
        entries: [{ id: 'a', description: '' }],
      }),
    ).toBe('updated · 1 available')
    expect(skillUpdateSummary({ available: false, entries: [] })).toBe(
      'unavailable',
    )
  })
})
