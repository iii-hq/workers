import { readFileSync } from 'node:fs'
import ts from 'typescript'
import { describe, expect, it } from 'vitest'

/** Check the real JSX shell without bootstrapping the live worker graph. */
function jsxChildren(path: string, tag: string) {
  const source = ts.createSourceFile(
    path,
    readFileSync(new URL(path, import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  )
  let children: string[] = []
  const visit = (node: ts.Node) => {
    if (
      ts.isJsxElement(node) &&
      node.openingElement.tagName.getText(source) === tag
    ) {
      children = node.children.flatMap((child) => {
        if (ts.isJsxSelfClosingElement(child)) {
          return [child.tagName.getText(source)]
        }
        if (ts.isJsxElement(child)) {
          return [child.openingElement.tagName.getText(source)]
        }
        return []
      })
      return
    }
    ts.forEachChild(node, visit)
  }
  visit(source)
  return children
}

describe('failure notice placement', () => {
  it('keeps navigation above global notices and the workspace', () => {
    expect(jsxChildren('./App.tsx', 'Sheet').slice(0, 3)).toEqual([
      'Header',
      'ConnectionNotice',
      'WorkspacePanes',
    ])
  })
})
