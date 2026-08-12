/**
 * DOM-free tests for the chat renderer's pure payload logic: envelope
 * tolerance, list capping, scene element counting, and error extraction.
 */

import { describe, expect, it } from 'vitest'

import {
  LIST_CAP,
  SCENE_PARSE_CAP,
  capList,
  errorDisplay,
  formatDay,
  hasContent,
  looksFreeform,
  mergeViews,
  parseDeleteResponse,
  parseListResponse,
  parseRecordView,
  parseSyntaxFamilies,
  parseValidateResponse,
  sceneElementCount,
} from './parsers'

const record = {
  id: 'abc12345',
  name: 'auth flow',
  format: 'mermaid',
  source: 'flowchart TD\n  a --> b',
  family: 'flowchart',
  created_at: 1_700_000_000,
  updated_at: 1_700_000_100,
}

describe('parseRecordView', () => {
  it('reads a raw handler record', () => {
    const view = parseRecordView(record)
    expect(view.id).toBe('abc12345')
    expect(view.name).toBe('auth flow')
    expect(view.format).toBe('mermaid')
    expect(view.family).toBe('flowchart')
    expect(view.updated_at).toBe(1_700_000_100)
  })

  it('reads through the harness content/details envelope', () => {
    const view = parseRecordView({ content: [], details: record })
    expect(view.id).toBe('abc12345')
    expect(view.source).toBe(record.source)
  })

  it('parses nothing from non-record shapes', () => {
    for (const value of [null, undefined, 'x', 7, [record], {}]) {
      expect(hasContent(parseRecordView(value))).toBe(false)
    }
  })

  it('drops unknown format strings', () => {
    expect(parseRecordView({ ...record, format: 'svg' }).format).toBeUndefined()
  })
})

describe('mergeViews', () => {
  it('prefers output fields and falls back to input fields', () => {
    const merged = mergeViews(
      { id: 'abc12345' },
      { id: 'ignored0', name: 'from input', source: 'flowchart TD' },
    )
    expect(merged.id).toBe('abc12345')
    expect(merged.name).toBe('from input')
    expect(merged.source).toBe('flowchart TD')
  })
})

describe('capList', () => {
  it('keeps short lists whole', () => {
    const { shown, hidden } = capList([1, 2, 3])
    expect(shown).toEqual([1, 2, 3])
    expect(hidden).toBe(0)
  })

  it('keeps exactly LIST_CAP rows without a remainder', () => {
    const items = Array.from({ length: LIST_CAP }, (_, i) => i)
    const { shown, hidden } = capList(items)
    expect(shown).toHaveLength(LIST_CAP)
    expect(hidden).toBe(0)
  })

  it('caps at LIST_CAP and counts the rest', () => {
    const items = Array.from({ length: LIST_CAP + 5 }, (_, i) => i)
    const { shown, hidden } = capList(items)
    expect(shown).toHaveLength(LIST_CAP)
    expect(shown[LIST_CAP - 1]).toBe(LIST_CAP - 1)
    expect(hidden).toBe(5)
  })
})

describe('sceneElementCount', () => {
  it('counts live elements of an excalidraw scene', () => {
    const scene = JSON.stringify({
      elements: [
        { type: 'rectangle' },
        { type: 'arrow', isDeleted: true },
        { type: 'text' },
      ],
    })
    expect(sceneElementCount(scene)).toBe(2)
  })

  it('accepts a bare top-level element array', () => {
    expect(sceneElementCount('[{"type":"rectangle"},{"type":"text"}]')).toBe(2)
  })

  it('returns null for mermaid text, bad JSON, and non-scene JSON', () => {
    expect(sceneElementCount('flowchart TD\n  a --> b')).toBeNull()
    expect(sceneElementCount('{not json')).toBeNull()
    expect(sceneElementCount('{"appState":{}}')).toBeNull()
    expect(sceneElementCount(undefined)).toBeNull()
  })

  it('refuses to parse oversized scenes', () => {
    const big = `{"elements":[],"pad":"${'x'.repeat(SCENE_PARSE_CAP)}"}`
    expect(sceneElementCount(big)).toBeNull()
  })
})

describe('looksFreeform', () => {
  it('trusts a declared format', () => {
    expect(looksFreeform({ format: 'freeform' })).toBe(true)
    expect(looksFreeform({ format: 'mermaid', source: '{"elements":[]}' })).toBe(
      false,
    )
  })

  it('sniffs scene JSON when format is undeclared', () => {
    expect(looksFreeform({ source: '{"elements":[]}' })).toBe(true)
    expect(looksFreeform({ source: 'flowchart TD' })).toBe(false)
    expect(looksFreeform({ family: 'flowchart', source: '{"elements":[]}' })).toBe(
      false,
    )
  })
})

describe('list / delete / syntax / validate parsers', () => {
  it('parses an enveloped list response and rejects non-lists', () => {
    const items = parseListResponse({
      content: [],
      details: { canvases: [record], count: 1 },
    })
    expect(items).toHaveLength(1)
    expect(items?.[0].name).toBe('auth flow')
    expect(parseListResponse({})).toBeNull()
    expect(parseListResponse(undefined)).toBeNull()
  })

  it('parses delete confirmations strictly', () => {
    expect(parseDeleteResponse({ id: 'abc12345', deleted: true })).toEqual({
      id: 'abc12345',
      deleted: true,
    })
    expect(parseDeleteResponse({ id: 'abc12345' })).toBeNull()
    expect(parseDeleteResponse({})).toBeNull()
  })

  it('parses syntax families from entries or bare strings', () => {
    expect(
      parseSyntaxFamilies({
        families: [{ family: 'flowchart', summary: '', example: '' }],
      }),
    ).toEqual(['flowchart'])
    expect(parseSyntaxFamilies({ families: ['sequence'] })).toEqual(['sequence'])
    expect(parseSyntaxFamilies({})).toBeNull()
  })

  it('parses validate responses and tolerates malformed issues', () => {
    const valid = parseValidateResponse({
      valid: true,
      family: 'flowchart',
      issues: [],
    })
    expect(valid).toEqual({ valid: true, family: 'flowchart', issues: [] })

    const invalid = parseValidateResponse({
      valid: false,
      family: null,
      issues: [{ line: 3, message: 'unexpected token' }, { line: 'x' }],
    })
    expect(invalid?.issues).toEqual([{ line: 3, message: 'unexpected token' }])
    expect(parseValidateResponse({ issues: [] })).toBeNull()
  })
})

describe('errorDisplay', () => {
  it('extracts the transport function_error wrapper with its reason', () => {
    expect(
      errorDisplay({
        error: {
          kind: 'function_error',
          message: 'trigger_failed: canvas::create',
          details: { reason: 'name already taken' },
        },
      }),
    ).toBe('trigger_failed: canvas::create — name already taken')
  })

  it('extracts string errors and enveloped errors', () => {
    expect(errorDisplay({ error: 'boom' })).toBe('boom')
    expect(
      errorDisplay({ content: [], details: { error: { message: 'boom' } } }),
    ).toBe('boom')
  })

  it('returns null for success shapes', () => {
    expect(errorDisplay(record)).toBeNull()
    expect(errorDisplay(undefined)).toBeNull()
    expect(errorDisplay({ valid: false, issues: [] })).toBeNull()
  })
})

describe('formatDay', () => {
  it('formats unix seconds as a UTC day', () => {
    expect(formatDay(1_700_000_100)).toBe('2023-11-14')
    expect(formatDay(undefined)).toBeNull()
  })
})
