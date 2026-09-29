// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Message } from '@/components/chat/Message'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { applyDocumentTheme } from '@/hooks/use-theme'
import { Markdown } from '@/lib/markdown'
import { renderMermaid } from '@/lib/mermaid'

vi.mock('@/lib/mermaid', () => ({ renderMermaid: vi.fn() }))

const renderDiagram = vi.mocked(renderMermaid)
const createUrl = vi.fn()
const revokeUrl = vi.fn()
const source = 'flowchart LR\n  A[Hello] --> B[Mermaid]'
const markdown = `\`\`\`mermaid\n${source}\n\`\`\``
let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  applyDocumentTheme('light')
  renderDiagram
    .mockReset()
    .mockResolvedValue('<svg xmlns="http://www.w3.org/2000/svg" />')
  createUrl
    .mockReset()
    .mockImplementation(() => `blob:diagram-${createUrl.mock.calls.length}`)
  revokeUrl.mockReset()
  vi.stubGlobal(
    'URL',
    class extends URL {
      static createObjectURL = createUrl
      static revokeObjectURL = revokeUrl
    },
  )
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

async function render(content = markdown, streaming = false) {
  await act(async () => {
    root.render(
      <TooltipProvider>
        <Markdown streaming={streaming}>{content}</Markdown>
      </TooltipProvider>,
    )
  })
}

describe('Mermaid in chat Markdown', () => {
  it('renders a fenced diagram as an inert image and keeps its source accessible', async () => {
    await render()
    expect(renderDiagram).toHaveBeenCalledWith(source, 'light')
    expect(container.querySelector('img')?.getAttribute('src')).toBe(
      'blob:diagram-1',
    )
    expect(container.querySelector('details code')?.textContent).toBe(source)
    expect(container.querySelector('svg')).toBeNull()
    expect(createUrl.mock.calls[0][0].type).toBe('image/svg+xml')

    // New prose must not remount or re-render the unchanged diagram.
    await render(`${markdown}\n\nDone.`)
    expect(renderDiagram).toHaveBeenCalledTimes(1)
  })

  it('waits for the actual assistant message to finish streaming', async () => {
    const message = {
      id: 'mermaid-test',
      role: 'assistant' as const,
      content: markdown,
      createdAt: 0,
    }
    await act(async () => {
      root.render(<Message message={{ ...message, streaming: true }} />)
    })
    expect(renderDiagram).not.toHaveBeenCalled()
    expect(container.querySelector('code')?.textContent).toBe(source)
    expect(container.textContent).toContain('message is complete')
    await act(async () => root.render(<Message message={message} />))
    expect(renderDiagram).toHaveBeenCalledTimes(1)
    expect(container.querySelector('img')).not.toBeNull()
  })

  it('shows source during incomplete fences without rendering partial syntax', async () => {
    await render('```mermaid\nflowchart LR\nA -->', true)
    expect(renderDiagram).not.toHaveBeenCalled()
    expect(container.querySelector('code')?.textContent).toContain('A -->')
    expect(container.textContent).not.toContain('Unable to render')
  })

  it('falls back to escaped source on error and recovers when the source changes', async () => {
    renderDiagram.mockRejectedValueOnce(new Error('Invalid diagram'))
    await render('```mermaid\n<script>alert(1)</script>\n```')
    expect(container.textContent).toContain('Unable to render this diagram')
    expect(container.querySelector('code')?.textContent).toBe(
      '<script>alert(1)</script>',
    )
    expect(container.querySelector('script')).toBeNull()
    await render()
    expect(container.querySelector('img')).not.toBeNull()
    expect(container.textContent).not.toContain('Unable to render')
  })

  it('ignores an obsolete async result when the source changes', async () => {
    let finish!: (svg: string) => void
    renderDiagram.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        }),
    )
    await render()
    await render('```mermaid\nflowchart LR\nC --> D\n```')
    await act(async () => finish('<svg>obsolete</svg>'))
    expect(createUrl).toHaveBeenCalledTimes(1)
    expect(container.querySelector('img')?.getAttribute('src')).toBe(
      'blob:diagram-1',
    )
    expect(container.querySelector('code')?.textContent).toContain('C --> D')
  })

  it('re-renders for theme changes and releases image URLs on replacement and unmount', async () => {
    await render()
    await act(async () => applyDocumentTheme('dark'))
    expect(renderDiagram).toHaveBeenLastCalledWith(source, 'dark')
    expect(revokeUrl).toHaveBeenCalledWith('blob:diagram-1')
    expect(container.querySelector('img')?.getAttribute('src')).toBe(
      'blob:diagram-2',
    )
    await act(async () => root.render(null))
    expect(revokeUrl).toHaveBeenCalledWith('blob:diagram-2')
  })

  it('does not render inline Mermaid mentions or other code languages as diagrams', async () => {
    await render(
      '`mermaid`\n\n```javascript\nconst value = 1\n```\n\n```json\n{"ok":true}\n```',
    )
    expect(renderDiagram).not.toHaveBeenCalled()
    expect(container.querySelector('[data-mermaid-diagram]')).toBeNull()
    expect(container.querySelector('code.language-javascript')).not.toBeNull()
    expect(container.textContent).toContain('"ok"')
  })

  it('offers fit and actual-size views without re-rendering the SVG', async () => {
    await render()
    const image = container.querySelector('img')
    const viewport = container.querySelector(
      '[aria-label="Mermaid diagram viewport"]',
    )
    const actual = container.querySelector<HTMLButtonElement>(
      '[aria-label="Show diagram at actual size"]',
    )
    const fit = container.querySelector<HTMLButtonElement>(
      '[aria-label="Fit entire diagram"]',
    )
    expect(image?.style.maxWidth).toBe('100%')
    expect(Number.parseFloat(image?.style.maxHeight ?? '')).toBeGreaterThan(0)
    expect(image?.style.objectFit).toBe('contain')
    expect(viewport?.className).toContain('overflow-auto')
    expect(viewport?.getAttribute('tabindex')).toBe('0')
    expect(container.querySelector('figure')?.className).not.toContain(
      'overflow-hidden',
    )
    await act(async () => actual?.click())
    expect(image?.style.maxWidth).toBe('none')
    expect(image?.style.maxHeight).toBe('none')
    expect(actual?.getAttribute('aria-pressed')).toBe('true')
    await act(async () => fit?.click())
    expect(image?.style.maxWidth).toBe('100%')
    expect(fit?.getAttribute('aria-pressed')).toBe('true')
    expect(renderDiagram).toHaveBeenCalledTimes(1)
  })

  it('fits tall diagrams to the transcript height and follows panel resizing', async () => {
    container.dataset.messageList = ''
    let transcriptHeight = 472
    let resize: () => void = () => {}
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(callback: () => void) {
          resize = callback
        }
        observe() {}
        disconnect() {}
      },
    )
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(
      function (this: HTMLElement) {
        return this === container ? transcriptHeight : 300
      },
    )
    vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(400)
    await render()
    const image = container.querySelector('img')
    const viewport = container.querySelector<HTMLElement>(
      '[aria-label="Mermaid diagram viewport"]',
    )
    expect(image?.style.maxHeight).toBe('372px')
    expect(viewport?.style.maxHeight).toBe('404px')
    transcriptHeight = 300
    await act(async () => resize())
    expect(image?.style.maxHeight).toBe('200px')
    expect(renderDiagram).toHaveBeenCalledTimes(1)
  })

  it('provides an explicit expand action for diagrams of any aspect ratio', async () => {
    await render()
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[aria-label="Expand diagram"]')
        ?.click()
    })
    expect(document.querySelector('[role="dialog"]')).not.toBeNull()
    expect(document.querySelector('[data-image-viewer-root]')).not.toBeNull()
    await act(async () => {
      document.querySelector<HTMLButtonElement>('[aria-label="Close"]')?.click()
    })
    expect(document.querySelector('[role="dialog"]')).toBeNull()
  })
})
