import { describe, expect, it } from 'vitest'
import { baselineFor, relativeToRoot, turnLabel, turnTitle } from '../turns'

describe('relativeToRoot', () => {
  it('relativizes paths under the root and rejects the rest', () => {
    expect(relativeToRoot('/r/a/b.ts', '/r')).toBe('a/b.ts')
    expect(relativeToRoot('/r/a/b.ts', '/r/')).toBe('a/b.ts')
    expect(relativeToRoot('/r', '/r')).toBeNull()
    expect(relativeToRoot('/rx/a', '/r')).toBeNull()
    expect(relativeToRoot('/other/a', '/r')).toBeNull()
  })
})

describe('baselineFor', () => {
  it('maps the stored pre-image onto the review baseline contract', () => {
    expect(baselineFor({ content: 'old' })).toBe('old')
    expect(baselineFor({ missing: true })).toBe('')
    expect(baselineFor({ content: 'part', truncated: true })).toBeNull()
    expect(baselineFor({ binary: true, revision: 'sha256:x' })).toBeNull()
    expect(baselineFor(null)).toBeNull()
    expect(baselineFor(undefined)).toBeNull()
    expect(baselineFor({})).toBeNull()
  })
})

describe('turnLabel', () => {
  it('shows a time and a file count', () => {
    expect(turnLabel({ started_at: Date.UTC(2026, 7, 20, 12, 0), file_count: 1 })).toMatch(/1 file$/)
    expect(turnLabel({ started_at: Date.UTC(2026, 7, 20, 12, 0), file_count: 3 })).toMatch(/3 files$/)
    expect(turnLabel({ started_at: Number.NaN, file_count: 0 })).toBe('turn · 0 files')
  })
})

describe('turnTitle', () => {
  it('prefers the message preview and falls back to the ordinal', () => {
    expect(turnTitle({ title: '  Fix the login bug ' }, 3)).toBe('Fix the login bug')
    expect(turnTitle({ title: '' }, 3)).toBe('Turn 3')
    expect(turnTitle({}, 1)).toBe('Turn 1')
  })
})
