import type { ReactElement } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { RevisionPane } from '../EditorPane'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
  memo: <T>(component: T) => component,
}))

// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({
  Button: () => null,
  CodeEditor: () => null,
  IconButton: () => null,
  Markdown: () => null,
}))

const reads = vi.hoisted(() => ({ count: 0 }))
vi.mock('../diff-load', () => ({
  loadRevisionFile: async (_host: unknown, _root: string, path: string, sha: string) => {
    reads.count += 1
    if (path.endsWith('.png')) throw new Error('binary file')
    return `${path} at ${sha}`
  },
}))

type Props = Parameters<typeof RevisionPane>[0]

/** The props of the first element in `node` that `test` accepts. */
function propsWhere(node: unknown, test: (props: Record<string, unknown>) => boolean): Record<string, unknown> | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const hit = propsWhere(child, test)
      if (hit) return hit
    }
    return null
  }
  if (node === null || typeof node !== 'object') return null
  const props = (node as { props?: Record<string, unknown> }).props
  if (props === undefined) return null
  return test(props) ? props : propsWhere(props.children, test)
}

/** The editor: the element whose `value` is set. */
const editorProps = (node: unknown) => propsWhere(node, (props) => 'value' in props)

describe('RevisionPane', () => {
  beforeEach(() => {
    reads.count = 0
  })

  it('reads a version once, then shows it from the page cache', async () => {
    const cache = new Map<string, string>()
    const props = {
      host: {},
      root: '/r',
      rootLabel: 'r',
      relPath: 'a.ts',
      sha: 'abc1234def',
      id: 'revision:abc1234def:a.ts',
      cache,
      onRevealDir: () => {},
      onClose: () => {},
    } as unknown as Props
    const pane = RevisionPane as unknown as (props: Props) => ReactElement

    const first = mount(pane, props)
    expect(editorProps(first.result)).toBeNull()
    await vi.waitFor(() =>
      expect(editorProps(first.result)).toMatchObject({ value: 'a.ts at abc1234def', readOnly: true }),
    )
    expect(cache.get('revision:abc1234def:a.ts')).toBe('a.ts at abc1234def')
    first.unmount()

    // The tab coming forward again: no second read, nothing loading.
    const again = mount(pane, props)
    expect(editorProps(again.result)).toMatchObject({ value: 'a.ts at abc1234def' })
    expect(reads.count).toBe(1)
  })

  it('says a binary version has no text, with no retry that would fail again', async () => {
    const props = {
      host: {},
      root: '/r',
      rootLabel: 'r',
      relPath: 'logo.png',
      sha: 'abc1234def',
      id: 'revision:abc1234def:logo.png',
      cache: new Map<string, string>(),
      onRevealDir: () => {},
      onClose: () => {},
    } as unknown as Props
    const pane = mount(RevisionPane as unknown as (props: Props) => ReactElement, props)
    const notice = () => propsWhere(pane.result, (each) => typeof each.title === 'string' && 'Icon' in each)
    await vi.waitFor(() => expect(notice()).toMatchObject({ title: 'Binary file: no text to show' }))
    expect(notice()?.actions).toBeUndefined()
    pane.unmount()
  })
})
