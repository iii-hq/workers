import { describe, expect, it } from 'vitest'
import {
  errorMessage,
  exportFilename,
  familyBadgeLabel,
  relativeTime,
} from './helpers'

describe('familyBadgeLabel', () => {
  it('maps known mermaid families to short labels', () => {
    expect(familyBadgeLabel('mermaid', 'flowchart')).toBe('flow')
    expect(familyBadgeLabel('mermaid', 'graph')).toBe('flow')
    expect(familyBadgeLabel('mermaid', 'gantt')).toBe('gantt')
    expect(familyBadgeLabel('mermaid', 'pie')).toBe('pie')
    expect(familyBadgeLabel('mermaid', 'c4')).toBe('c4')
    expect(familyBadgeLabel('mermaid', 'gitGraph')).toBe('git')
    expect(familyBadgeLabel('mermaid', 'mindmap')).toBe('mind')
  })

  it('normalizes Diagram/version suffixes before mapping', () => {
    expect(familyBadgeLabel('mermaid', 'sequenceDiagram')).toBe('seq')
    expect(familyBadgeLabel('mermaid', 'classDiagram')).toBe('class')
    expect(familyBadgeLabel('mermaid', 'stateDiagram-v2')).toBe('state')
    expect(familyBadgeLabel('mermaid', 'erDiagram')).toBe('er')
    expect(familyBadgeLabel('mermaid', 'flowchart-v2')).toBe('flow')
    expect(familyBadgeLabel('mermaid', 'xychart-beta')).toBe('xy')
  })

  it('labels the formats when no family applies', () => {
    expect(familyBadgeLabel('freeform', null)).toBe('freeform')
    // freeform wins even if a family somehow rides along
    expect(familyBadgeLabel('freeform', 'flowchart')).toBe('freeform')
    expect(familyBadgeLabel('mermaid', null)).toBe('mermaid')
  })

  it('passes unknown families through, truncated to badge width', () => {
    expect(familyBadgeLabel('mermaid', 'zenuml')).toBe('zenuml')
    expect(familyBadgeLabel('mermaid', 'hypothetical-long-family')).toBe(
      'hypothetic',
    )
  })
})

describe('exportFilename', () => {
  it('slugs the canvas name', () => {
    expect(exportFilename('My Cool Diagram', 'svg')).toBe('my-cool-diagram.svg')
  })

  it('collapses runs of non-alphanumerics and trims edge dashes', () => {
    expect(exportFilename('  --weird__ name!! ', 'png')).toBe('weird-name.png')
  })

  it('folds accents to ascii', () => {
    expect(exportFilename('café flow', 'svg')).toBe('cafe-flow.svg')
  })

  it('falls back to canvas when nothing survives', () => {
    expect(exportFilename('', 'svg')).toBe('canvas.svg')
    expect(exportFilename('***', 'png')).toBe('canvas.png')
  })

  it('caps very long names', () => {
    const name = 'x'.repeat(200)
    const file = exportFilename(name, 'svg')
    expect(file.endsWith('.svg')).toBe(true)
    expect(file.length).toBeLessThanOrEqual(64 + '.svg'.length)
  })
})

describe('relativeTime', () => {
  const now = 1_700_000_000

  it('reads recent times as just now', () => {
    expect(relativeTime(now - 10, now)).toBe('just now')
  })

  it('clamps future timestamps instead of going negative', () => {
    expect(relativeTime(now + 500, now)).toBe('just now')
  })

  it('scales through minutes, hours and days', () => {
    expect(relativeTime(now - 120, now)).toBe('2m ago')
    expect(relativeTime(now - 7200, now)).toBe('2h ago')
    expect(relativeTime(now - 172_800, now)).toBe('2d ago')
  })

  it('falls back to the date past 30 days', () => {
    expect(relativeTime(0, 86_400 * 100)).toBe('1970-01-01')
  })
})

describe('errorMessage', () => {
  it('reads Errors, strings and message-shaped objects', () => {
    expect(errorMessage(new Error('boom'))).toBe('boom')
    expect(errorMessage('plain')).toBe('plain')
    expect(errorMessage({ message: 'wire error' })).toBe('wire error')
  })

  it('serializes other objects and stringifies primitives', () => {
    expect(errorMessage({ code: 7 })).toBe('{"code":7}')
    expect(errorMessage(42)).toBe('42')
    expect(errorMessage(undefined)).toBe('undefined')
  })
})
