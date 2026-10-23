import { readFileSync } from 'node:fs'
import ts from 'typescript'
import { describe, expect, it } from 'vitest'

/**
 * Transformed absolute children count toward a scroll container's overflow,
 * so the edge-nudge keyframe (translateX ±2px) would grow scrollWidth past
 * clientWidth and flash a classic horizontal scrollbar every cycle, costing
 * every `min-h-0 flex-1` pane ~15px of height. Keep the edge zones outside the
 * `workspace panels` scroller. Check the real JSX without rendering.
 */
function findJsxByAriaLabel(source: ts.SourceFile, label: string) {
  let match: ts.JsxElement | undefined
  const visit = (node: ts.Node) => {
    if (match) return
    if (ts.isJsxElement(node)) {
      const hasLabel = node.openingElement.attributes.properties.some(
        (attribute) =>
          ts.isJsxAttribute(attribute) &&
          attribute.name.getText(source) === 'aria-label' &&
          attribute.initializer !== undefined &&
          ts.isStringLiteral(attribute.initializer) &&
          attribute.initializer.text === label,
      )
      if (hasLabel) {
        match = node
        return
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(source)
  return match
}

function countJsxTags(root: ts.Node, source: ts.SourceFile, tag: string) {
  let count = 0
  const visit = (node: ts.Node) => {
    const opening = ts.isJsxElement(node) ? node.openingElement : node
    if (
      (ts.isJsxSelfClosingElement(opening) ||
        ts.isJsxOpeningElement(opening)) &&
      opening.tagName.getText(source) === tag
    ) {
      count += 1
    }
    ts.forEachChild(node, visit)
  }
  visit(root)
  return count
}

function nearestJsxParent(node: ts.Node) {
  let current = node.parent
  while (current && !ts.isJsxElement(current)) current = current.parent
  return current as ts.JsxElement | undefined
}

describe('workspace pane edge zones', () => {
  const path = './App.tsx'
  const source = ts.createSourceFile(
    path,
    readFileSync(new URL(path, import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  )
  const scroller = findJsxByAriaLabel(source, 'workspace panels')
  if (!scroller) throw new Error('missing the "workspace panels" scroller')

  it('keeps the edge add zones out of the horizontal scroller', () => {
    expect(countJsxTags(scroller, source, 'EdgeAddZone')).toBe(0)
  })

  it('positions both edge add zones against the scroller wrapper', () => {
    const wrapper = nearestJsxParent(scroller)
    if (!wrapper) throw new Error('the scroller has no wrapping JSX element')
    expect(countJsxTags(wrapper, source, 'EdgeAddZone')).toBe(2)
  })
})
