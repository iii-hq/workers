import { describe, expect, it } from 'vitest'
import type { FindRelevantResponse, RelevantFile } from '../coder'
import {
  effectivePattern,
  flattenSearchRows,
  groupContentMatches,
  locateHit,
  previewRow,
  relevantAsMatches,
  searchSummary,
  stepSearchRow,
} from '../search-model'

const opts = { query: 'needle', regex: false, ignoreCase: true, wholeWord: false }

describe('effectivePattern', () => {
  it('escapes literals and wraps whole words', () => {
    expect(effectivePattern({ ...opts, query: 'a.b' })).toEqual({ pattern: 'a\\.b', regex: false })
    expect(effectivePattern({ ...opts, query: 'a.b', wholeWord: true })).toEqual({
      pattern: '\\b(?:a\\.b)\\b',
      regex: true,
    })
    expect(effectivePattern({ ...opts, query: 'a.b', regex: true })).toEqual({ pattern: 'a.b', regex: true })
  })
})

describe('previewRow', () => {
  it('trims the lead to a window and keeps the hit and trail', () => {
    const text = `${'x'.repeat(80)} needle ${'y'.repeat(200)}`
    const row = previewRow({ path: '/r/a.ts', line: 3, column: 82, text }, /needle/i)
    expect(row.hit).toBe('needle')
    expect(row.leadCut).toBe(true)
    expect(row.lead.length).toBeLessThanOrEqual(28)
    expect(row.trail.startsWith(' yyy')).toBe(true)
    expect(row.trail.length).toBeLessThanOrEqual(120)
  })

  it('drops leading indentation from the lead', () => {
    const row = previewRow({ path: '/r/a.ts', line: 1, column: 9, text: '        needle()' }, /needle/i)
    expect(row.lead).toBe('')
    expect(row.hit).toBe('needle')
    expect(row.trail).toBe('()')
  })

  it('falls back to a regex scan when the byte column misses (multi-byte prefix)', () => {
    const text = 'ééé needle'
    // Bytes: é is 2 bytes, so the worker reports column 8 for char index 4.
    expect(locateHit(text, 8, /needle/)).toEqual({ start: 4, end: 10 })
  })
})

describe('groupContentMatches + rows', () => {
  const matches = [
    { path: '/r/src/a.ts', line: 1, column: 1, text: 'needle one' },
    { path: '/r/src/a.ts', line: 7, column: 3, text: '  needle two' },
    { path: '/r/b.md', line: 2, column: 1, text: 'needle three' },
  ]

  it('groups by file with root-relative names and directories', () => {
    const groups = groupContentMatches(matches, '/r', opts)
    expect(groups.map((g) => [g.rel, g.name, g.dir, g.matches.length])).toEqual([
      ['src/a.ts', 'a.ts', 'src', 2],
      ['b.md', 'b.md', '', 1],
    ])
  })

  it('flattens to header + match rows, honouring collapsed files', () => {
    const groups = groupContentMatches(matches, '/r', opts)
    const rows = flattenSearchRows(groups, [], new Set(['/r/src/a.ts']))
    expect(rows.map((r) => r.type)).toEqual(['file', 'file', 'match'])
    const open = flattenSearchRows(groups, [], new Set())
    expect(open.map((r) => r.type)).toEqual(['file', 'match', 'match', 'file', 'match'])
    expect(open.map((r) => r.key)).toEqual([
      'file:/r/src/a.ts',
      'match:/r/src/a.ts:1:1',
      'match:/r/src/a.ts:7:3',
      'file:/r/b.md',
      'match:/r/b.md:2:1',
    ])
  })

  it('adds sections when both names and text matched', () => {
    const groups = groupContentMatches(matches, '/r', opts)
    const rows = flattenSearchRows(groups, [{ path: '/r/needle.ts', rel: 'needle.ts', name: 'needle.ts', dir: '', kind: 'file' }], new Set())
    expect(rows[0]).toMatchObject({ type: 'section', label: 'Files and folders', count: 1 })
    expect(rows[1].type).toBe('path')
    expect(rows[2]).toMatchObject({ type: 'section', label: 'Text matches', count: 3 })
  })

  it('summarizes counts and truncation', () => {
    const groups = groupContentMatches(matches, '/r', opts)
    expect(searchSummary(groups, [], false)).toBe('3 results in 2 files')
    expect(searchSummary(groups, [], true)).toContain('refine')
    expect(searchSummary([], [], false)).toBe('No results')
  })

  it('steps a roving focus over section rows', () => {
    const groups = groupContentMatches(matches, '/r', opts)
    const rows = flattenSearchRows(groups, [{ path: '/r/n.ts', rel: 'n.ts', name: 'n.ts', dir: '', kind: 'file' }], new Set())
    expect(stepSearchRow(rows, 1, 1)).toBe(3)
    expect(stepSearchRow(rows, 3, -1)).toBe(1)
    expect(stepSearchRow(rows, 1, -1)).toBe(1)
    expect(stepSearchRow(rows, rows.length - 1, 1)).toBe(rows.length - 1)
  })
})

describe('relevantAsMatches', () => {
  const file = (path: string, rest: Partial<RelevantFile> = {}): RelevantFile => ({
    path,
    score: 0.9,
    roles: [],
    excerpts: [],
    leads: [],
    source_omitted: false,
    ...rest,
  })
  const noHighlight = { query: '', regex: false, ignoreCase: true, wholeWord: false }

  it('keeps the worker ranking and opens excerpts at their first non-blank line', () => {
    const out: FindRelevantResponse = {
      status: 'complete',
      files: [
        file('/r/z.ts', { excerpts: [{ line_from: 10, line_to: 14, text: '\n  \nfunction pick() {\n}' }] }),
        file('/r/a.ts', { excerpts: [{ line_from: 3, line_to: 3, text: 'const a = 1' }] }),
      ],
    }
    const rows = relevantAsMatches(out)
    expect(rows.map((r) => [r.path, r.line, r.text])).toEqual([
      ['/r/z.ts', 12, 'function pick() {'],
      ['/r/a.ts', 3, 'const a = 1'],
    ])
    const groups = groupContentMatches(rows, '/r', noHighlight)
    expect(groups.map((g) => g.rel)).toEqual(['z.ts', 'a.ts'])
    expect(groups[0].matches[0]).toMatchObject({ hit: '', lead: 'function pick() {' })
  })

  it('shows leads as line ranges, prefixed by their name when present', () => {
    const out: FindRelevantResponse = {
      status: 'incomplete',
      files: [
        file('/r/a.ts', {
          leads: [
            { name: 'Store.load', line_from: 20, line_to: 40, score: 0.4 },
            { line_from: 50, line_to: 60, score: 0.3 },
          ],
        }),
      ],
    }
    expect(relevantAsMatches(out).map((r) => [r.line, r.text])).toEqual([
      [20, 'Store.load lines 20-40'],
      [50, 'lines 50-60'],
    ])
  })

  it('keys rows uniquely when an excerpt and a lead share a line', () => {
    const out: FindRelevantResponse = {
      status: 'complete',
      files: [
        file('/r/a.ts', {
          excerpts: [{ line_from: 1, line_to: 3, text: 'function first() {' }],
          leads: [{ name: 'first', line_from: 1, line_to: 3, score: 0.9 }],
        }),
      ],
    }
    const keys = flattenSearchRows(groupContentMatches(relevantAsMatches(out), '/r', noHighlight), [], new Set()).map(
      (r) => r.key,
    )
    expect(keys).toEqual(['file:/r/a.ts', 'match:/r/a.ts:1:1', 'match:/r/a.ts:1:1#1'])
  })

  it('gives a file with neither excerpts nor leads one row at line 1', () => {
    const out: FindRelevantResponse = { status: 'complete', files: [file('/r/b.py', { source_omitted: true })] }
    expect(relevantAsMatches(out)).toEqual([{ path: '/r/b.py', line: 1, column: 1, text: 'relevant file' }])
  })
})
