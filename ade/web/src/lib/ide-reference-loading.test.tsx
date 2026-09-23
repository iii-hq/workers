// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Only mock host-provided visual primitives, not file loading or its cache.
vi.mock('@iii-dev/console-ui', async () => {
  const React = await import('react')
  return {
    Button: ({ children, variant: _variant, size: _size, ...props }: Record<string, unknown>) => React.createElement('button', props, children as React.ReactNode),
    IconButton: ({ children, label, ...props }: Record<string, unknown>) => React.createElement('button', { ...props, 'aria-label': label }, children as React.ReactNode),
    CodeEditor: React.forwardRef((_props, _ref) => null),
    Markdown: () => null,
    CodeBlock: () => null,
    ImageViewer: () => null,
    Tooltip: ({ children }: { children: React.ReactNode }) => children,
  }
})

import { EditorPane, type EditorCache } from '../../../../ide/ui/src/page/EditorPane'

let container: HTMLDivElement
let root: Root
const cache: EditorCache = new Map()
const trigger = vi.fn()
const noop = () => {}
const host = { iii: { trigger } } as unknown as React.ComponentProps<typeof EditorPane>['host']

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  cache.clear()
  trigger.mockReset()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

async function show(path: string, reveal?: { line: number; seq: number }) {
  await act(async () => {
    root.render(<EditorPane key={path} host={host} root="/repo" rootLabel="repo" relPath={path}
      cache={cache} createObjectUrl={() => ''} onSaved={noop} onDirtyChange={noop}
      onRevealDir={noop} onCompare={noop} reveal={reveal} />)
  })
}

describe('IDE reference loading', () => {
  it('does not resurrect a closed tab cache after its read resolves', async () => {
    let finish: (value: unknown) => void = noop
    trigger.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve }))
    trigger.mockResolvedValueOnce({ content: 'second', is_utf8: true, revision: 'v2' })
    await show('first.ts')
    await show('second.ts')
    await act(async () => { finish({ content: 'stale first', is_utf8: true }) })
    expect(cache.has('first.ts')).toBe(false)
    expect(cache.get('second.ts')?.draft).toBe('second')
  })

  it('keeps unsaved edits when navigating away and back', async () => {
    cache.set('dirty.ts', { draft: 'unsaved edits', savedContent: 'original', revision: 'v1', readOnly: null, size: 8, mode: 0o644 })
    trigger.mockResolvedValue({ content: 'other', is_utf8: true })
    await show('dirty.ts')
    await show('other.ts')
    await show('dirty.ts', { line: 1, seq: 1 })
    expect(cache.get('dirty.ts')).toMatchObject({ draft: 'unsaved edits', savedContent: 'original', revision: 'v1' })
    expect(trigger).toHaveBeenCalledTimes(1)
  })

  it('surfaces worker read failures and allows retrying', async () => {
    trigger.mockRejectedValueOnce(new Error('permission denied'))
    trigger.mockResolvedValueOnce({ content: 'readable', is_utf8: true })
    await show('protected.ts')
    expect(container.textContent).toContain('This file could not be opened')
    expect(container.textContent).toContain('permission denied')
    const retry = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes('Try again'))
    await act(async () => { retry?.click() })
    expect(cache.get('protected.ts')?.draft).toBe('readable')
  })

  it('warns when a citation is beyond the loaded text', async () => {
    trigger.mockResolvedValue({ content: 'one line', is_utf8: true })
    await show('short.ts', { line: 999, seq: 1 })
    expect(container.querySelector('[role="status"]')?.textContent).toContain('exceeds the current file')
  })
})
