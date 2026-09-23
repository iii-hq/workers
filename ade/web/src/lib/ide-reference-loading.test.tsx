// @vitest-environment jsdom

import { act, useCallback, useEffect, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const reveals = vi.hoisted(() => ({ line: vi.fn(), range: vi.fn() }))

// Only mock host-provided visual primitives, not file loading or its cache.
vi.mock('@iii-dev/console-ui', async () => {
  const React = await import('react')
  return {
    Button: ({ children, variant: _variant, size: _size, ...props }: Record<string, unknown>) => React.createElement('button', props, children as React.ReactNode),
    IconButton: ({ children, label, ...props }: Record<string, unknown>) => React.createElement('button', { ...props, 'aria-label': label }, children as React.ReactNode),
    CodeEditor: React.forwardRef(({ value, onChange }: { value: string; onChange: (value: string) => void }, ref) => {
      React.useImperativeHandle(ref, () => ({ revealLine: reveals.line, revealLines: reveals.range }))
      return React.createElement('textarea', { 'aria-label': 'buffer', value, onChange: (e: React.ChangeEvent<HTMLTextAreaElement>) => onChange(e.target.value) })
    }),
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
  reveals.line.mockClear()
  reveals.range.mockClear()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

/** Mount the real pane with an isolated host transport and page-owned cache. */
async function show(path: string, reveal?: { line: number; seq: number }) {
  await act(async () => {
    root.render(<EditorPane key={path} host={host} root="/repo" rootLabel="repo" relPath={path}
      cache={cache} createObjectUrl={() => ''} onSaved={noop} onDirtyChange={noop}
      onRevealDir={noop} onCompare={noop} reveal={reveal} />)
  })
}

type RevealRequest = { path: string; line: number; seq: number; endLine?: number }

/** Model the page's request acknowledgement across file and refresh remounts. */
function RevealPage({ path, request, bump = 0 }: { path: string; request: RevealRequest; bump?: number }) {
  const [pending, setPending] = useState<RevealRequest | null>(request)
  useEffect(() => setPending(request), [request])
  const handled = useCallback((path: string, seq: number) => {
    setPending((current) => current?.path === path && current.seq === seq ? null : current)
  }, [])
  return <EditorPane key={`${path}:${bump}`} host={host} root="/repo" rootLabel="repo" relPath={path}
    cache={cache} createObjectUrl={noImage} onSaved={noop} onDirtyChange={noop}
    onRevealDir={noop} onCompare={noop} richPreview
    reveal={pending?.path === path ? pending : null} onRevealHandled={handled} />
}

/** No object URLs are needed by these text-file fixtures. */
const noImage = () => ''

/** Exercise textarea edits through React's real change-event delegation. */
async function editBuffer(value: string) {
  const input = container.querySelector('textarea')
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set
  if (!input || !setValue) throw new Error('Expected an editable buffer')
  await act(async () => {
    setValue.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('one-shot reference reveals', () => {
  it('preserves preview and does not replay a citation after tab or refresh remounts', async () => {
    trigger.mockResolvedValue({ content: 'one\ntwo', is_utf8: true })
    const request = { path: 'note.md', line: 2, seq: 1 }
    await act(async () => root.render(<RevealPage path="note.md" request={request} />))
    expect(reveals.line).toHaveBeenCalledExactlyOnceWith(2, undefined)
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Show preview"]')?.click())
    await act(async () => root.render(<RevealPage path="other.ts" request={request} />))
    await act(async () => root.render(<RevealPage path="note.md" request={request} />))
    expect(container.querySelector('[aria-label="Show source"]')).not.toBeNull()
    await act(async () => root.render(<RevealPage path="note.md" request={request} bump={1} />))
    expect(container.querySelector('[aria-label="Show source"]')).not.toBeNull()
    expect(reveals.line).toHaveBeenCalledTimes(1)
    await act(async () => root.render(<RevealPage path="note.md" request={{ ...request, seq: 2 }} bump={1} />))
    expect(reveals.line).toHaveBeenCalledTimes(2)
    expect(container.querySelector('[aria-label="Show preview"]')).not.toBeNull()
  })

  it('snapshots warnings at reveal time and clears them on edit or remount', async () => {
    trigger.mockResolvedValue({ content: 'one\ntwo', is_utf8: true })
    const request = { path: 'note.ts', line: 2, seq: 1 }
    await act(async () => root.render(<RevealPage path="note.ts" request={request} />))
    await editBuffer('short')
    expect(cache.get('note.ts')?.draft).toBe('short')
    expect(container.querySelector('[role="status"]')).toBeNull()
    const next = { ...request, seq: 2 }
    await act(async () => root.render(<RevealPage path="note.ts" request={next} />))
    expect(container.querySelector('[role="status"]')?.textContent).toContain('exceeds')
    await editBuffer('shorter')
    expect(container.querySelector('[role="status"]')).toBeNull()
    const third = { ...request, seq: 3 }
    await act(async () => root.render(<RevealPage path="note.ts" request={third} />))
    expect(container.querySelector('[role="status"]')).not.toBeNull()
    await act(async () => root.render(<RevealPage path="other.ts" request={third} />))
    await act(async () => root.render(<RevealPage path="note.ts" request={third} />))
    expect(container.querySelector('[role="status"]')).toBeNull()
    expect(reveals.line).toHaveBeenCalledTimes(3)
  })

  it('keeps a pending reveal after a failed read until retry succeeds', async () => {
    trigger.mockRejectedValueOnce(new Error('permission denied'))
    trigger.mockResolvedValueOnce({ content: 'one', is_utf8: true })
    const request = { path: 'retry.ts', line: 1, seq: 1 }
    await act(async () => root.render(<RevealPage path="retry.ts" request={request} />))
    expect(reveals.line).not.toHaveBeenCalled()
    const retry = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes('Try again'))
    await act(async () => retry?.click())
    expect(reveals.line).toHaveBeenCalledExactlyOnceWith(1, undefined)
  })
})

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
