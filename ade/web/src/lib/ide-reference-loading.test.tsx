// @vitest-environment jsdom

import { act, useCallback, useEffect, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const reveals = vi.hoisted(() => ({ line: vi.fn(), range: vi.fn() }))

// Only mock host-provided visual primitives, not file loading or its cache.
vi.mock('@iii-dev/console-ui', async () => {
  const React = await import('react')
  return {
    Button: ({
      children,
      variant: _variant,
      size: _size,
      ...props
    }: Record<string, unknown>) =>
      React.createElement('button', props, children as React.ReactNode),
    useConfirm: () => ({ confirm: async () => true, dialog: null }),
    IconButton: ({ children, label, ...props }: Record<string, unknown>) =>
      React.createElement(
        'button',
        { ...props, 'aria-label': label },
        children as React.ReactNode,
      ),
    CodeEditor: React.forwardRef(
      (
        {
          value,
          onChange,
        }: { value: string; onChange: (value: string) => void },
        ref,
      ) => {
        React.useImperativeHandle(ref, () => ({
          revealLine: reveals.line,
          revealLines: reveals.range,
        }))
        return React.createElement('textarea', {
          'aria-label': 'buffer',
          value,
          onChange: (e: React.ChangeEvent<HTMLTextAreaElement>) =>
            onChange(e.target.value),
        })
      },
    ),
    Markdown: () => null,
    CodeBlock: () => null,
    ImageViewer: () => null,
    Tooltip: ({ children }: { children: React.ReactNode }) => children,
  }
})

import {
  type EditorCache,
  EditorPane,
} from '../../../../ide/ui/src/page/EditorPane'
import { htmlPreviewLimits } from '../../../../ide/ui/src/page/rich-preview'

let container: HTMLDivElement
let root: Root
const cache: EditorCache = new Map()
const call = vi.fn()
const noop = () => {}
const host = { iii: { call } } as unknown as React.ComponentProps<
  typeof EditorPane
>['host']

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  cache.clear()
  call.mockReset()
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
    root.render(
      <EditorPane
        key={path}
        host={host}
        root="/repo"
        rootLabel="repo"
        relPath={path}
        cache={cache}
        createObjectUrl={() => ''}
        onDirtyChange={noop}
        onRevealDir={noop}
        onCompare={noop}
        reveal={reveal}
      />,
    )
  })
}

type RevealRequest = {
  path: string
  line: number
  seq: number
  endLine?: number
}

/** Model the page's request acknowledgement across file and refresh remounts. */
function RevealPage({
  path,
  request,
  bump = 0,
}: {
  path: string
  request: RevealRequest
  bump?: number
}) {
  const [pending, setPending] = useState<RevealRequest | null>(request)
  useEffect(() => setPending(request), [request])
  const handled = useCallback((path: string, seq: number) => {
    setPending((current) =>
      current?.path === path && current.seq === seq ? null : current,
    )
  }, [])
  return (
    <EditorPane
      key={`${path}:${bump}`}
      host={host}
      root="/repo"
      rootLabel="repo"
      relPath={path}
      cache={cache}
      createObjectUrl={noImage}
      onDirtyChange={noop}
      onRevealDir={noop}
      onCompare={noop}
      richPreview
      reveal={pending?.path === path ? pending : null}
      onRevealHandled={handled}
    />
  )
}

/** No object URLs are needed by these text-file fixtures. */
const noImage = () => ''

/** Exercise textarea edits through React's real change-event delegation. */
async function editBuffer(value: string) {
  const input = container.querySelector('textarea')
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'value',
  )?.set
  if (!input || !setValue) throw new Error('Expected an editable buffer')
  await act(async () => {
    setValue.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('one-shot reference reveals', () => {
  it('preserves preview and does not replay a citation after tab or refresh remounts', async () => {
    call.mockResolvedValue({ content: 'one\ntwo', is_utf8: true })
    const request = { path: 'note.md', line: 2, seq: 1 }
    await act(async () =>
      root.render(<RevealPage path="note.md" request={request} />),
    )
    expect(reveals.line).toHaveBeenCalledExactlyOnceWith(2, undefined)
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>('[aria-label="Show preview"]')
        ?.click(),
    )
    await act(async () =>
      root.render(<RevealPage path="other.ts" request={request} />),
    )
    await act(async () =>
      root.render(<RevealPage path="note.md" request={request} />),
    )
    expect(container.querySelector('[aria-label="Show source"]')).not.toBeNull()
    await act(async () =>
      root.render(<RevealPage path="note.md" request={request} bump={1} />),
    )
    expect(container.querySelector('[aria-label="Show source"]')).not.toBeNull()
    expect(reveals.line).toHaveBeenCalledTimes(1)
    await act(async () =>
      root.render(
        <RevealPage path="note.md" request={{ ...request, seq: 2 }} bump={1} />,
      ),
    )
    expect(reveals.line).toHaveBeenCalledTimes(2)
    expect(
      container.querySelector('[aria-label="Show preview"]'),
    ).not.toBeNull()
  })

  it('snapshots warnings at reveal time and clears them on edit or remount', async () => {
    call.mockResolvedValue({ content: 'one\ntwo', is_utf8: true })
    const request = { path: 'note.ts', line: 2, seq: 1 }
    await act(async () =>
      root.render(<RevealPage path="note.ts" request={request} />),
    )
    await editBuffer('short')
    expect(cache.get('note.ts')?.draft).toBe('short')
    expect(container.querySelector('[role="status"]')).toBeNull()
    const next = { ...request, seq: 2 }
    await act(async () =>
      root.render(<RevealPage path="note.ts" request={next} />),
    )
    expect(container.querySelector('[role="status"]')?.textContent).toContain(
      'exceeds',
    )
    await editBuffer('shorter')
    expect(container.querySelector('[role="status"]')).toBeNull()
    const third = { ...request, seq: 3 }
    await act(async () =>
      root.render(<RevealPage path="note.ts" request={third} />),
    )
    expect(container.querySelector('[role="status"]')).not.toBeNull()
    await act(async () =>
      root.render(<RevealPage path="other.ts" request={third} />),
    )
    await act(async () =>
      root.render(<RevealPage path="note.ts" request={third} />),
    )
    expect(container.querySelector('[role="status"]')).toBeNull()
    expect(reveals.line).toHaveBeenCalledTimes(3)
  })

  it('keeps a pending reveal after a failed read until retry succeeds', async () => {
    call.mockRejectedValueOnce(new Error('permission denied'))
    call.mockResolvedValueOnce({ content: 'one', is_utf8: true })
    const request = { path: 'retry.ts', line: 1, seq: 1 }
    await act(async () =>
      root.render(<RevealPage path="retry.ts" request={request} />),
    )
    expect(reveals.line).not.toHaveBeenCalled()
    const retry = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Try again'),
    )
    await act(async () => retry?.click())
    expect(reveals.line).toHaveBeenCalledExactlyOnceWith(1, undefined)
  })
})

describe('IDE reference loading', () => {
  it('does not resurrect a closed tab cache after its read resolves', async () => {
    let finish: (value: unknown) => void = noop
    call.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        }),
    )
    call.mockResolvedValueOnce({
      content: 'second',
      is_utf8: true,
      revision: 'v2',
    })
    await show('first.ts')
    await show('second.ts')
    await act(async () => {
      finish({ content: 'stale first', is_utf8: true })
    })
    expect(cache.has('first.ts')).toBe(false)
    expect(cache.get('second.ts')?.draft).toBe('second')
  })

  it('keeps unsaved edits when navigating away and back', async () => {
    cache.set('dirty.ts', {
      draft: 'unsaved edits',
      savedContent: 'original',
      revision: 'v1',
      readOnly: null,
      size: 8,
      mode: 0o644,
    })
    call.mockResolvedValue({ content: 'other', is_utf8: true })
    await show('dirty.ts')
    await show('other.ts')
    await show('dirty.ts', { line: 1, seq: 1 })
    expect(cache.get('dirty.ts')).toMatchObject({
      draft: 'unsaved edits',
      savedContent: 'original',
      revision: 'v1',
    })
    expect(call).toHaveBeenCalledTimes(1)
  })

  it('surfaces worker read failures and allows retrying', async () => {
    call.mockRejectedValueOnce(new Error('permission denied'))
    call.mockResolvedValueOnce({ content: 'readable', is_utf8: true })
    await show('protected.ts')
    expect(container.textContent).toContain('This file could not be opened')
    expect(container.textContent).toContain('permission denied')
    const retry = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Try again'),
    )
    await act(async () => {
      retry?.click()
    })
    expect(cache.get('protected.ts')?.draft).toBe('readable')
  })

  it('warns when a citation is beyond the loaded text', async () => {
    call.mockResolvedValue({ content: 'one line', is_utf8: true })
    await show('short.ts', { line: 999, seq: 1 })
    expect(container.querySelector('[role="status"]')?.textContent).toContain(
      'exceeds the current file',
    )
  })
})

/** Model the page's Open preview request, acknowledged like a reveal, with
    the IDE's default of opening files as source. */
function PreviewPage({
  path,
  request,
  bump = 0,
}: {
  path: string
  request: { path: string; seq: number }
  bump?: number
}) {
  const [pending, setPending] = useState<{ path: string; seq: number } | null>(
    request,
  )
  useEffect(() => setPending(request), [request])
  const handled = useCallback((path: string, seq: number) => {
    setPending((current) =>
      current?.path === path && current.seq === seq ? null : current,
    )
  }, [])
  return (
    <EditorPane
      key={`${path}:${bump}`}
      host={host}
      root="/repo"
      rootLabel="repo"
      relPath={path}
      cache={cache}
      createObjectUrl={noImage}
      onDirtyChange={noop}
      onRevealDir={noop}
      onCompare={noop}
      previewRequest={pending?.path === path ? pending : null}
      onPreviewHandled={handled}
    />
  )
}

describe('explorer Open preview', () => {
  it('opens an HTML file rendered, once, and keeps a later Show source', async () => {
    call.mockResolvedValue({
      content: '<h1>Hello</h1>',
      is_utf8: true,
    })
    const request = { path: 'site/index.html', seq: 1 }
    await act(async () =>
      root.render(<PreviewPage path="site/index.html" request={request} />),
    )
    expect(container.querySelector('iframe[title="preview site/index.html"]')).not.toBeNull()
    // Plain markup renders whole: no note about what is missing.
    expect(container.querySelector('[role="note"]')).toBeNull()
    await act(async () =>
      container
        .querySelector<HTMLButtonElement>('[aria-label="Show source"]')
        ?.click(),
    )
    expect(container.querySelector('iframe')).toBeNull()
    // A remount (tab switch or refresh) does not replay the request.
    await act(async () =>
      root.render(<PreviewPage path="site/index.html" request={request} bump={1} />),
    )
    expect(container.querySelector('iframe')).toBeNull()
    expect(container.querySelector('[aria-label="Show preview"]')).not.toBeNull()
    // A new request on the same file shows it again.
    await act(async () =>
      root.render(
        <PreviewPage path="site/index.html" request={{ ...request, seq: 2 }} bump={1} />,
      ),
    )
    expect(container.querySelector('iframe')).not.toBeNull()
  })

  it('says what a static preview leaves out of an app shell', async () => {
    call.mockResolvedValue({
      content:
        '<div id="root"></div><script type="module" src="/src/main.tsx"></script>',
      is_utf8: true,
    })
    await act(async () =>
      root.render(<PreviewPage path="index.html" request={{ path: 'index.html', seq: 1 }} />),
    )
    expect(container.querySelector('[role="note"]')?.textContent).toContain(
      "scripts don't run",
    )
  })

  it('waits for a failed read to succeed before switching', async () => {
    call.mockRejectedValueOnce(new Error('permission denied'))
    call.mockResolvedValueOnce({ content: '<p>ok</p>', is_utf8: true })
    await act(async () =>
      root.render(<PreviewPage path="page.htm" request={{ path: 'page.htm', seq: 1 }} />),
    )
    expect(container.textContent).toContain('This file could not be opened')
    const retry = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Try again'),
    )
    await act(async () => retry?.click())
    expect(container.querySelector('iframe[title="preview page.htm"]')).not.toBeNull()
  })

  it('leaves a file with no rendered view as source', async () => {
    call.mockResolvedValue({ content: 'const a = 1', is_utf8: true })
    await act(async () =>
      root.render(<PreviewPage path="main.ts" request={{ path: 'main.ts', seq: 1 }} />),
    )
    expect(container.querySelector('textarea')).not.toBeNull()
    expect(container.querySelector('iframe')).toBeNull()
  })
})

describe('htmlPreviewLimits', () => {
  it('is null for markup that renders whole', () => {
    expect(htmlPreviewLimits('<h1>Hi</h1><style>h1{color:red}</style>')).toBeNull()
    expect(
      htmlPreviewLimits(
        '<link rel="stylesheet" href="https://cdn.example/a.css"><img src="data:image/png;base64,AA">',
      ),
    ).toBeNull()
    // Links the page navigates to are not resources it needs.
    expect(htmlPreviewLimits('<a href="./about.html">About</a>')).toBeNull()
  })

  it('names scripts, local files, or both', () => {
    expect(htmlPreviewLimits('<script>1</script>')).toContain("scripts don't run")
    expect(htmlPreviewLimits('<link rel="stylesheet" href="style.css">')).toContain(
      "linked local files",
    )
    expect(htmlPreviewLimits("<img src='/logo.png'>")).toContain('linked local files')
    expect(htmlPreviewLimits('<script src="./app.js"></script>')).toContain(
      "scripts don't run and linked local files don't load",
    )
  })
})
