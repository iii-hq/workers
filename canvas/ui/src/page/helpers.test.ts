import { describe, expect, it } from 'vitest'
import { exportFilename, familyBadgeLabel } from './helpers'

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
