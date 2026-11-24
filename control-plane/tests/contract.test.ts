import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { acceptSummary, SUMMARY_SCHEMA, WRITER_READS } from '../src/summary.js'

const result = {
  checked_at: '2026-11-21T18:44:00Z',
  unchanged: false,
  brief: {
    headline: 'KAN-635 verification landed',
    happened: 'KAN-635 is verified.',
    happening: 'iii-hq/workers#1834 waits on CI.',
    remaining: '',
    refs: ['KAN-635', 'iii-hq/workers#1834'],
  },
  diary: [
    {
      at: '2026-11-21T18:40:00Z',
      text: 'KAN-635 verification passed.',
      refs: ['KAN-635'],
    },
  ],
  now: {
    'session:dispatch-kan635': {
      text: 'KAN-635 is waiting for its next wake.',
      refs: ['session:dispatch-kan635', 'KAN-635'],
    },
  },
  pr_waits: {
    'iii-hq/workers#1834': {
      text: 'iii-hq/workers#1834 waits on running CI.',
      refs: ['iii-hq/workers#1834'],
    },
  },
  needs_you: [
    {
      id: 'card:KAN-635',
      waiting: true,
      text: 'Choose the price table for Scale before billing goes live.',
      recommendation:
        'Take the usage table already reviewed on the card; it matches the plan limits.',
      refs: ['KAN-635'],
    },
  ],
}

describe('Control plane writer contract', () => {
  it('keeps the profile on the exact read-only policy', async () => {
    const profile = await readFile(
      join(import.meta.dirname, '../../kits/control-plane/agents/control-plane-writer.md'),
      'utf8',
    )
    const frontmatter = profile.split('---', 3)[1]
    const functions = [...frontmatter.matchAll(/^ {2}- (\S+)$/gm)].map((match) => match[1])

    expect(functions).toEqual(WRITER_READS)
    expect(frontmatter).toContain('hidden: true')
    expect(frontmatter).toContain('skills_index: false')
    expect(frontmatter).not.toMatch(/^extends:/m)
    expect(frontmatter).not.toMatch(/^model:/m)
  })

  it('accepts changed, unchanged and empty results', () => {
    const now = Date.parse(result.checked_at)
    expect(acceptSummary(result, null, now)).toEqual(result)
    expect(acceptSummary({ ...result, unchanged: true }, null, now)).toEqual({
      ...result,
      unchanged: true,
    })
    expect(
      acceptSummary(
        {
          ...result,
          brief: { headline: '', happened: '', happening: '', remaining: '', refs: [] },
          diary: [],
          now: {},
          pr_waits: {},
        },
        null,
        now,
      ),
    ).not.toBeNull()
    expect(SUMMARY_SCHEMA.required).not.toContain('direction')
  })

  it('accepts checks only for candidate ids and found cards or sessions', () => {
    const now = Date.parse(result.checked_at)
    const check = result.needs_you[0]
    for (const id of [
      'question:s1:q1',
      'stalled:s1',
      'card:KAN-1',
      'reply:s1',
      'found:KAN-1',
      'found:session:s1',
    ])
      expect(acceptSummary({ ...result, needs_you: [{ ...check, id }] }, null, now)).not.toBeNull()
    for (const id of ['found:https://example.com', 'note:KAN-1', 'found:release:r1'])
      expect(acceptSummary({ ...result, needs_you: [{ ...check, id }] }, null, now)).toBeNull()
  })

  it('rejects missing references, invalid subjects and extra fields', () => {
    const now = Date.parse(result.checked_at)
    expect(acceptSummary({ ...result, brief: { ...result.brief, refs: [] } }, null, now)).toBeNull()
    expect(
      acceptSummary(
        { ...result, now: { 'KAN-635': result.now['session:dispatch-kan635'] } },
        null,
        now,
      ),
    ).toBeNull()
    expect(acceptSummary({ ...result, extra: 'instruction' }, null, now)).toBeNull()
  })

  it('rejects invalid timestamps, references, diary bounds and missing fields', () => {
    const now = Date.parse(result.checked_at)
    expect(acceptSummary({ ...result, checked_at: 'yesterday' }, null, now)).toBeNull()
    expect(
      acceptSummary({ ...result, brief: { ...result.brief, refs: ['release:r1'] } }, null, now),
    ).toBeNull()
    expect(
      acceptSummary(
        { ...result, diary: Array.from({ length: 51 }, () => result.diary[0]) },
        null,
        now,
      ),
    ).toBeNull()
    const { diary: _diary, ...withoutDiary } = result
    expect(acceptSummary(withoutDiary, null, now)).toBeNull()
  })

  it('requires every top-level field and nonempty references on every prose object', () => {
    const now = Date.parse(result.checked_at)
    for (const field of Object.keys(result)) {
      const draft = structuredClone(result)
      Reflect.deleteProperty(draft, field)
      expect(acceptSummary(draft, null, now)).toBeNull()
    }
    for (let index = 0; index < 4; index += 1)
      for (const missing of [false, true]) {
        const draft = structuredClone(result)
        const notes = [
          draft.brief,
          draft.diary[0],
          draft.now['session:dispatch-kan635'],
          draft.pr_waits['iii-hq/workers#1834'],
        ]
        if (missing) Reflect.deleteProperty(notes[index], 'refs')
        else notes[index].refs = []
        expect(acceptSummary(draft, null, now)).toBeNull()
      }
  })

  it('permits digits in names', () => {
    const withNames = {
      ...result,
      brief: {
        ...result.brief,
        headline: 'KAN-635 verified billing-v2 for R1 and round 23.',
      },
    }
    expect(acceptSummary(withNames, null, Date.parse(result.checked_at))).toEqual(withNames)
  })
})
