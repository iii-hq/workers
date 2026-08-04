/**
 * `redactRaw` — an injected renderer's declaration of what the card's RAW
 * exits may show (types/injectable-ui.ts).
 *
 * The card always mounts a `raw json` tab rendering `message.input` /
 * `message.output`, with a copy button beside each pane, so a renderer that
 * redacts a capability inside its own card has NOT contained it. Both exits
 * are pinned here: what the panes render (server-rendered HTML) and what the
 * copy button copies (`paneCopyText` — the single derivation `ValuePane`
 * hands `PaneShell`; console/web has no DOM in unit tests, so the clipboard
 * is pinned at that seam rather than by clicking).
 *
 * `@/lib/ui-slots` is mocked because `useSyncExternalStore` would hand back
 * the (always empty) server snapshot under `renderToStaticMarkup`; the
 * fencing, `rawRedactor` and the panes below it are the real code. `Tabs` is
 * mocked to a passthrough because Radix renders only the ACTIVE tab — and
 * the leaking pane is the inactive one.
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { RegisteredRenderer } from '@/lib/ui-slots'
import type { FunctionTriggerMessage } from '@/types/chat'
import type { FunctionTriggerRenderer } from '@/types/injectable-ui'
import { FunctionTriggerCard, paneCopyText } from './FunctionTriggerCard'
import {
  functionTriggerRenderers,
  RAW_REDACTION_FAILED,
  rawRedactor,
} from './renderer-registry'

const { injected } = vi.hoisted(() => ({
  injected: [] as RegisteredRenderer[],
}))

vi.mock('@/lib/ui-slots', () => ({
  useExtRenderers: () => injected,
}))

vi.mock('@/components/ui/Tabs', () => ({
  Tabs: ({ children }: { children?: React.ReactNode }) => <div>{children}</div>,
  TabsList: ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  ),
  TabsTrigger: ({ children }: { children?: React.ReactNode }) => (
    <button type="button">{children}</button>
  ),
  TabsContent: ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  ),
}))

const SECRET = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const MASKED = 'rt-3f9a…'
const FN = 'code-runner::eval'

/** A worker-shaped redactor: every string, object keys included. */
function maskDeep(value: unknown): unknown {
  if (typeof value === 'string') return value.replaceAll(SECRET, MASKED)
  if (value === null || typeof value !== 'object') return value
  if (Array.isArray(value)) return value.map(maskDeep)
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([k, v]) => [
      maskDeep(k),
      maskDeep(v),
    ]),
  )
}

function register(renderer: Partial<FunctionTriggerRenderer>) {
  injected.push({
    scope: 'code-runner',
    path: 'code-runner/page.js',
    renderer: {
      id: 'code-runner/page.js#test',
      isMatch: (functionId) => functionId === FN,
      tryRender: () => null,
      ...renderer,
    },
  })
}

function message(
  extra: Partial<FunctionTriggerMessage> = {},
): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId: FN,
    createdAt: 0,
    input: { runtime_id: SECRET, code: `// ran in ${SECRET}` },
    output: { registered: [`code-runner::${SECRET}::foo`] },
    durationMs: 12,
    ...extra,
  }
}

function html(extra: Partial<FunctionTriggerMessage> = {}): string {
  return renderToStaticMarkup(
    <FunctionTriggerCard message={message(extra)} defaultOpen />,
  )
}

/**
 * What the card's two copy buttons would put on the clipboard: the same
 * resolution the card does (`rawRedactor` over the live dispatch list), then
 * the pane's own copy-text derivation.
 */
function copyTexts(extra: Partial<FunctionTriggerMessage> = {}): string[] {
  const m = message(extra)
  const redact = rawRedactor(functionTriggerRenderers(injected), m.functionId)
  return [m.input, m.output].map((v) => paneCopyText(redact ? redact(v) : v))
}

beforeEach(() => {
  injected.length = 0
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('a renderer that declares redactRaw', () => {
  beforeEach(() => {
    register({ redactRaw: maskDeep })
  })

  it('redacts both panes of the default (no custom terminal) card', () => {
    const out = html()
    expect(out).not.toContain(SECRET)
    expect(out).toContain(MASKED)
  })

  it('redacts both panes of the raw json tab behind a custom terminal', () => {
    injected.length = 0
    // A card that renders none of the payload: the raw tab is exactly the
    // exposure this test exists for.
    register({
      redactRaw: maskDeep,
      tryRender: () => <span>custom terminal</span>,
    })
    const out = html()
    expect(out).toContain('custom terminal')
    expect(out).toContain('raw json')
    expect(out).not.toContain(SECRET)
    expect(out).toContain(MASKED)
  })

  it('redacts the in-flight response pane and the streaming args tail', () => {
    expect(html({ running: true })).not.toContain(SECRET)
    const streaming = html({
      running: true,
      output: undefined,
      input: { _streaming: `{"runtime_id":"${SECRET}"` },
    })
    expect(streaming).not.toContain(SECRET)
    expect(streaming).toContain(MASKED)
  })

  it('redacts the pending-approval request pane', () => {
    const out = html({
      pendingApproval: true,
      output: undefined,
      durationMs: undefined,
    })
    expect(out).not.toContain(SECRET)
    expect(out).toContain(MASKED)
  })

  it('redacts what the copy buttons copy', () => {
    const copied = copyTexts()
    expect(copied).toHaveLength(2)
    for (const text of copied) {
      expect(text).not.toContain(SECRET)
      expect(text).toContain(MASKED)
    }
  })

  it('redacts the copy of a result envelope, whose copy text is the whole value', () => {
    const [, response] = copyTexts({
      output: {
        content: [{ type: 'text', text: `ran in ${SECRET}` }],
        details: { runtime_id: SECRET },
      },
    })
    expect(response).not.toContain(SECRET)
    expect(response).toContain(MASKED)
  })
})

describe('without a matching redactRaw', () => {
  // These are the positive controls: they prove the payload's secret reaches
  // the HTML (and the clipboard) verbatim through the very same code path,
  // so the `not.toContain` assertions above are not vacuous.
  it('leaves the raw panes untouched when the renderer declares none', () => {
    register({})
    expect(html()).toContain(SECRET)
    expect(copyTexts()[0]).toContain(SECRET)
  })

  it('leaves a message the renderer does not claim untouched', () => {
    register({ redactRaw: maskDeep })
    expect(html({ functionId: 'shell::run' })).toContain(SECRET)
  })

  it('is untouched when nothing is injected at all', () => {
    expect(html()).toContain(SECRET)
  })
})

describe('a throwing redactRaw', () => {
  beforeEach(() => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    register({
      redactRaw: () => {
        throw new Error('boom')
      },
    })
  })

  it('fails closed: the placeholder renders, never the raw value', () => {
    const out = html()
    expect(out).not.toContain(SECRET)
    expect(out).toContain('redaction failed')
  })

  it('fails closed on the copy path too', () => {
    for (const text of copyTexts()) {
      expect(text).not.toContain(SECRET)
      expect(text).toBe(RAW_REDACTION_FAILED)
    }
  })
})

describe('rawRedactor', () => {
  it('picks the first renderer that both claims the id and declares redactRaw', () => {
    const renderers: FunctionTriggerRenderer[] = [
      { id: 'a', isMatch: () => true, tryRender: () => null },
      {
        id: 'b',
        isMatch: (f) => f === FN,
        tryRender: () => null,
        redactRaw: () => 'b',
      },
      {
        id: 'c',
        isMatch: () => true,
        tryRender: () => null,
        redactRaw: () => 'c',
      },
    ]
    expect(rawRedactor(renderers, FN)?.({})).toBe('b')
    expect(rawRedactor(renderers, 'other::fn')?.({})).toBe('c')
    expect(rawRedactor(renderers.slice(0, 1), FN)).toBeUndefined()
  })
})
