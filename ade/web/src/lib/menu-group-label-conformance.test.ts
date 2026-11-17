import { readdirSync, readFileSync } from 'node:fs'
import { extname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { describe, expect, it } from 'vitest'

/**
 * Base UI menu and select labels are *group* labels: `Menu.GroupLabel` and
 * `Select.GroupLabel` read the group context and throw ("MenuGroupContext is
 * missing") when rendered outside `Menu.Group` / `Select.Group`. The throw
 * only happens when the menu opens, so a page renders fine until someone
 * clicks its menu and the whole worker page crashes (QA 2026-11-16: the
 * Sentinel issue page's Investigate / Resolve / Ignore menus).
 *
 * Every `DropdownMenuLabel`, `ShadcnDropdownMenuLabel`, `SelectLabel` or
 * `*.GroupLabel` in the console and in every worker UI must therefore sit
 * inside a `*MenuGroup` / `*SelectGroup` / `*.Group` / `*RadioGroup`
 * element in the same JSX tree.
 */

const repoRoot = fileURLToPath(new URL('../../../../', import.meta.url))
const sourceRoots = [
  join(repoRoot, 'ade/web/src'),
  join(repoRoot, 'packages/traces-ui/src'),
  ...readdirSync(repoRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(repoRoot, entry.name, 'ui/src')),
]

function sourceFiles(root: string): string[] {
  try {
    return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
      if (entry.name === 'node_modules' || entry.name === 'dist') return []
      const path = join(root, entry.name)
      if (entry.isDirectory()) return sourceFiles(path)
      return ['.tsx', '.jsx'].includes(extname(path)) ? [path] : []
    })
  } catch {
    return []
  }
}

/** `Menu.GroupLabel`, `DropdownMenuLabel`, `ShadcnSelectLabel`, … (not `ListGroupLabel`, a plain div). */
export function isGroupLabelName(name: string): boolean {
  if (/(^|\.)GroupLabel$/.test(name)) return true
  return /(MenuLabel|SelectLabel)$/.test(name) && !/ListGroupLabel$/.test(name)
}

/** `Menu.Group`, `DropdownMenuGroup`, `ShadcnDropdownMenuRadioGroup`, `SelectGroup`, … */
export function isGroupName(name: string): boolean {
  return /(^|\.)(Radio)?Group$/.test(name) || /(Menu|Select)(Radio)?Group$/.test(name)
}

function tagName(node: ts.JsxOpeningLikeElement): string {
  return node.tagName.getText()
}

/** Local name → imported name, so `ShadcnDropdownMenuLabel as DropdownMenuLabel` still reads as a label. */
function importAliases(file: ts.SourceFile): Map<string, string> {
  const aliases = new Map<string, string>()
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement)) continue
    const bindings = statement.importClause?.namedBindings
    if (!bindings || !ts.isNamedImports(bindings)) continue
    for (const element of bindings.elements) {
      if (element.propertyName) aliases.set(element.name.text, element.propertyName.getText(file))
    }
  }
  return aliases
}

function functionName(node: ts.Node): string | undefined {
  if (ts.isFunctionDeclaration(node) || ts.isFunctionExpression(node)) return node.name?.text
  if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) {
    const parent = node.parent
    if (ts.isVariableDeclaration(parent) && ts.isIdentifier(parent.name)) return parent.name.text
    // forwardRef((…) => <Label/>) assigned to a variable.
    if (ts.isCallExpression(parent) && ts.isVariableDeclaration(parent.parent) && ts.isIdentifier(parent.parent.name)) {
      return parent.parent.name.text
    }
  }
  return undefined
}

/** Labels in `source` with no group element around them. */
export function ungroupedLabels(source: string, path = 'file.tsx'): string[] {
  const file = ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const aliases = importAliases(file)
  const resolve = (name: string) => aliases.get(name) ?? name
  const offenders: string[] = []

  const visit = (node: ts.Node) => {
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const name = tagName(node)
      if (isGroupLabelName(resolve(name)) && !insideGroup(node, resolve)) {
        const { line } = file.getLineAndCharacterOfPosition(node.getStart(file))
        offenders.push(`${path}:${line + 1} <${name}>`)
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(file)
  return offenders
}

function insideGroup(label: ts.Node, resolve: (name: string) => string): boolean {
  // Start above the label's own element.
  let node: ts.Node | undefined = ts.isJsxOpeningElement(label) ? label.parent.parent : label.parent
  while (node) {
    if (ts.isJsxElement(node) && isGroupName(resolve(tagName(node.openingElement)))) return true
    if (ts.isFunctionLike(node)) {
      // A label component's own definition (`function DropdownMenuLabel() { return <Menu.GroupLabel/> }`)
      // passes the group requirement on to its callers, which this scan checks.
      const name = functionName(node)
      if (name && isGroupLabelName(name)) return true
    }
    node = node.parent
  }
  return false
}

describe('menu group label contract', () => {
  it('flags a label that is not inside a group', () => {
    expect(
      ungroupedLabels(`
        const a = (
          <DropdownMenuContent>
            <DropdownMenuLabel>Resolve</DropdownMenuLabel>
            <DropdownMenuItem>Now</DropdownMenuItem>
          </DropdownMenuContent>
        )`),
    ).toEqual(['file.tsx:4 <DropdownMenuLabel>'])
  })

  it('reads aliased and primitive labels', () => {
    expect(
      ungroupedLabels(`
        import { ShadcnDropdownMenuLabel as Head } from '@iii-dev/console-ui'
        const a = <Content><Head>x</Head><Menu.GroupLabel>y</Menu.GroupLabel></Content>`),
    ).toEqual(['file.tsx:3 <Head>', 'file.tsx:3 <Menu.GroupLabel>'])
  })

  it('accepts a label inside a group, however deep', () => {
    expect(
      ungroupedLabels(`
        const a = (
          <DropdownMenuContent>
            <DropdownMenuGroup>
              {open ? <ShadcnDropdownMenuLabel>x</ShadcnDropdownMenuLabel> : null}
              {items.map((i) => <SelectPrimitive.Group key={i}><SelectPrimitive.GroupLabel>{i}</SelectPrimitive.GroupLabel></SelectPrimitive.Group>)}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        )`),
    ).toEqual([])
  })

  it('accepts the definition of a label component', () => {
    expect(
      ungroupedLabels(`
        function DropdownMenuLabel(props) {
          return <MenuPrimitive.GroupLabel {...props} />
        }`),
    ).toEqual([])
  })

  it('ignores the plain list group label', () => {
    expect(ungroupedLabels(`const a = <List><ListGroupLabel>Recent</ListGroupLabel></List>`)).toEqual([])
  })

  it('keeps every menu and select label in the console and worker UIs inside a group', () => {
    const offenders = sourceRoots.flatMap(sourceFiles).flatMap((path) =>
      ungroupedLabels(readFileSync(path, 'utf8'), relative(repoRoot, path)),
    )
    expect(offenders).toEqual([])
  })
})
