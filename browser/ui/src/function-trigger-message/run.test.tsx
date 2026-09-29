import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { createBrowserRenderer } from './index'

// Structural console components pass children (and MetaRow items) through
// so the rendered text is inspectable.
vi.mock('@iii-dev/console-ui', () => {
  const passthrough = ({
    children,
    className,
  }: {
    children?: React.ReactNode
    className?: string
  }) => <div className={className}>{children}</div>
  return {
    ActionLine: passthrough,
    Badge: ({ children, variant }: { children?: React.ReactNode; variant?: string }) => (
      <span data-variant={variant}>{children}</span>
    ),
    Chip: passthrough,
    EmptyState: () => null,
    JsonHighlight: () => <pre>raw-json</pre>,
    MetaRow: ({
      children,
      items,
    }: {
      children?: React.ReactNode
      items?: { label: string; value: string }[]
    }) => (
      <div>
        {items?.map((item) => (
          <span key={item.label}>
            {item.label}={item.value}
          </span>
        ))}
        {children}
      </div>
    ),
    Skeleton: () => null,
    Table: passthrough,
    TableBody: passthrough,
    TableCell: passthrough,
    TableRow: passthrough,
    TableViewport: passthrough,
  }
})

const renderer = createBrowserRenderer({} as Host)

function render(output: unknown, goal = 'File a Bug titled Login loop and save it.') {
  const message = {
    functionId: 'browser::run',
    input: { session_id: 'b54', goal },
    output,
    running: false,
  } as unknown as FunctionTriggerMessage
  return renderToStaticMarkup(<>{renderer.tryRender(message)}</>)
}

const page = { url: 'http://127.0.0.1:8766/', title: 'Tracker · Saved', elements: [{}, {}, {}] }

describe('browser::run card', () => {
  it('shows where the run ended, the goal, every step and the page it left', () => {
    const html = render({
      status: 'done',
      steps: [
        { operation: 'CLICK', ref: 'n9', label: 'Accept cookies', probability: 0.95, page_changed: true, judge_ms: 267 },
        { operation: 'SELECT', ref: 'n2', label: 'Type', option: 'Task', probability: 1, page_changed: true, judge_ms: 302 },
        { operation: 'CLICK', ref: 'n7', label: 'Save issue', probability: 0.5, page_changed: false, judge_ms: 226 },
      ],
      page,
      judge_requests: 4,
      elapsed_ms: 2358,
    })
    expect(html).toContain('steps=3')
    expect(html).toContain('judge=4 req')
    expect(html).toContain('time=2.4 s')
    expect(html).toContain('data-variant="ok"')
    expect(html).toContain('File a Bug titled Login loop and save it.')
    expect(html).toContain('Accept cookies')
    expect(html).toContain('select')
    expect(html).toContain('→ Task')
    expect(html).toContain('95% · 267 ms')
    expect(html).toContain('no change on the page')
    expect(html).toContain('Tracker · Saved')
    expect(html).toContain('3 controls')
    expect(html).toContain('session <span class="br-ui-call-sid">b54</span>')
    expect(html).not.toContain('raw-json')
  })

  it('flags a refused step, a missing text and an unavailable judge', () => {
    const html = render({
      status: 'needs_text',
      steps: [
        { operation: 'CLICK', label: 'Save', probability: 0.9, page_changed: false, judge_ms: 200, error: 'click on n7 refused: the element is covered by div#consent' },
      ],
      needs_text: { ref: 'n1', label: 'Title', input_keys: ['summary'] },
      page,
      judge_requests: 2,
      elapsed_ms: 900,
    })
    expect(html).toContain('data-variant="warn"')
    expect(html).toContain('needs text')
    expect(html).toContain('covered by div#consent')
    expect(html).toContain('no inputs key matched (summary)')
    const down = render({
      status: 'judge_unavailable',
      reason: 'judge unavailable: provider_unavailable',
      steps: [],
      page,
      judge_requests: 1,
      elapsed_ms: 12,
    })
    expect(down).toContain('judge unavailable: provider_unavailable')
    const loading = render({ ...{ status: 'max_steps', steps: [], judge_requests: 1, elapsed_ms: 9 }, page: { ...page, busy: true } })
    expect(loading).toContain('still loading')
  })

  it('leaves a payload that is not a run to the generic card', () => {
    const html = render({ unexpected: true })
    expect(html).not.toContain('steps=')
    expect(html).not.toContain('controls')
  })
})
