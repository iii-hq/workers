/**
 * The completion value — `result` — is the field code-runner's wire adds over
 * `sandbox-code-runner`'s, so it is the part of this UI that is NOT a proven
 * port and needs its own tests.
 *
 * Three things are asserted here:
 *
 *  1. A value RENDERS, and renders as JSON rather than as `[object Object]`.
 *  2. A NULL result renders explicitly, with the engine's return convention
 *     beside it. `result` is always present on the wire and never skipped,
 *     because a null result is information ("the code returned nothing"), not
 *     an absence (run.rs) — and the overwhelmingly common cause is the
 *     convention mismatch: node code is a function body (`return 2 + 2`),
 *     python code is a module (assign `result`). A card that hid the null, or
 *     showed it without the hint, would leave the reader with no way to tell a
 *     working call from a mis-written one.
 *  3. A runtime id INSIDE the result is redacted. This is the leak the
 *     `RuntimeChip` does not cover: a script that calls back into the engine
 *     can RETURN its runtime id, and `JSON.stringify` of the raw value would
 *     carry the capability into the feed.
 *
 * Renders through `react-dom/server` against a stubbed `@iii-dev/console-ui`
 * (the real package's JS entry throws — it is compile-time-only, served at
 * runtime by the console's import map). The stub still renders every prop it is
 * handed, so a card that stopped routing text through `redactRuntimeIds` is
 * caught here rather than hidden behind an inert mock.
 */

import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { truncateRuntimeId } from '../lib/shared'
import { createRunRenderer } from './run'

// Hoisted above the imports by vitest, so the renderer module resolves the
// stub, never the real package's throwing JS entry.
vi.mock('@iii-dev/console-ui', () => ({
  Tooltip: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children?: React.ReactNode }) => <span data-stub="tooltip-content">{children}</span>,
  CodeHighlight: ({ code }: { code: string }) => <pre data-stub="code-highlight">{code}</pre>,
  JsonHighlight: ({ code }: { code: string }) => <pre data-stub="json-highlight">{code}</pre>,
}))

/** Both engines mint `rt-<uuid>` — node-core's and python-core's `manager.rs`
 * alike (`format!("rt-{}", Uuid::new_v4())`). */
const RUNTIME_ID = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const TRUNCATED = truncateRuntimeId(RUNTIME_ID)

const FUNCTION_ID = 'code-runner::run'
const HOST = {} as unknown as Host

function msg(input: unknown, output: unknown): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId: FUNCTION_ID,
    input,
    output,
    running: false,
    createdAt: 0,
  } as FunctionTriggerMessage
}

/** The settled card's HTML for one request/response pair. */
function renderSettled(input: unknown, output: unknown): string {
  const node = createRunRenderer(HOST).tryRender(msg(input, output))
  expect(node).not.toBeNull()
  return renderToStaticMarkup(node as React.ReactElement)
}

/** A well-formed response, with `result` swapped per case. */
function response(result: unknown, extra: Record<string, unknown> = {}) {
  return {
    stdout: '',
    stderr: '',
    exit_code: 0,
    success: true,
    duration_ms: 12,
    result,
    ...extra,
  }
}

describe('the completion value', () => {
  it('renders an object result as JSON', () => {
    const html = renderSettled({ lang: 'python', code: 'result = {"n": 42}' }, response({ n: 42 }))
    expect(html).toContain('result')
    expect(html).toContain('&quot;n&quot;')
    expect(html).toContain('42')
    // Never the useless stringification of an object.
    expect(html).not.toContain('[object Object]')
  })

  it('renders a scalar result', () => {
    const html = renderSettled({ lang: 'node', code: 'return 4' }, response(4))
    expect(html).toContain('4')
  })

  it('renders a null result explicitly rather than hiding the section', () => {
    const html = renderSettled({ lang: 'node', code: '2 + 2' }, response(null))
    expect(html).toContain('null')
    expect(html).toContain('returned nothing')
  })

  /**
   * The `return`-vs-`result` mismatch is the single most common way a call
   * comes back null having "worked", so the null state names the convention
   * for the engine that actually ran.
   */
  it('explains node’s return convention on a null result', () => {
    const html = renderSettled({ lang: 'node', code: '2 + 2' }, response(null))
    expect(html).toContain('function body')
    expect(html).toContain('return')
  })

  it('explains python’s result convention on a null result', () => {
    const html = renderSettled({ lang: 'python', code: '2 + 2' }, response(null))
    expect(html).toContain('module')
    expect(html).toContain('result')
  })

  /**
   * A reuse can omit `lang` (the language belongs to the runtime), and there is
   * then no honest convention to quote — so the null still renders, without a
   * hint invented for an engine we cannot name.
   */
  it('omits the convention hint when the request did not say which engine', () => {
    const html = renderSettled({ runtime_id: RUNTIME_ID, code: 'whatever' }, response(null))
    expect(html).toContain('returned nothing')
    expect(html).not.toContain('function body')
    expect(html).not.toContain('module')
  })

  /**
   * `result` is always present on this wire, so a response without it is
   * malformed — and saying so beats rendering a card that silently implies the
   * code returned nothing.
   */
  it('reports a response that carried no result field at all', () => {
    const { result: _dropped, ...withoutResult } = response(null)
    const html = renderSettled({ lang: 'node', code: 'return 1' }, withoutResult)
    expect(html).toContain('no `result` field')
  })

  /**
   * The leak `RuntimeChip` does not cover: a script that calls back into the
   * engine can RETURN its runtime id, and the stringified raw value would
   * otherwise carry the capability into the feed.
   */
  it('redacts a runtime id returned inside the result', () => {
    const html = renderSettled({ lang: 'python', code: 'result = rt' }, response({ created: RUNTIME_ID }))
    expect(html).not.toContain(RUNTIME_ID)
    expect(html).toContain(TRUNCATED)
  })

  it('redacts a runtime id nested deep inside the result', () => {
    const html = renderSettled({ lang: 'node', code: 'return x' }, response({ runtimes: [{ id: RUNTIME_ID }] }))
    expect(html).not.toContain(RUNTIME_ID)
  })

  /** Object KEYS carry ids too — a payload can map by runtime. */
  it('redacts a runtime id used as a result object key', () => {
    const html = renderSettled({ lang: 'node', code: 'return x' }, response({ [RUNTIME_ID]: 'alive' }))
    expect(html).not.toContain(RUNTIME_ID)
  })

  /**
   * A non-zero exit is a RESPONSE, not an error: the card must still render,
   * still show the result slot, and must not escalate to the alert tone that
   * means the worker failed.
   */
  it('still renders the result on a non-zero exit', () => {
    const html = renderSettled(
      { lang: 'python', code: 'raise ValueError("nope")' },
      response(null, {
        exit_code: 1,
        success: false,
        stderr: 'ValueError: nope\n',
      }),
    )
    expect(html).toContain('exit 1')
    expect(html).toContain('ValueError: nope')
    expect(html).toContain('returned nothing')
    expect(html).not.toContain('cr-ui-alert')
  })
})
