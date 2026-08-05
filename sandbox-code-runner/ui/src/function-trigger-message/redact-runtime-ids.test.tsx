/**
 * The capability sweep: `runtime_id` is a capability — whoever holds one can
 * run into or tear down that sandbox — so the full value must never reach the
 * DOM. `RuntimeChip` truncates the `runtime_id` FIELD, but every other site
 * that renders free text has to redact independently, and sandbox-code-runner
 * gives that text three ways in:
 *
 *  - ERROR MESSAGES QUOTE IT BY DESIGN. `RuntimeNotFound` is
 *    "unknown runtime_id {id}" and `Expired` is "runtime {id} expired: …"
 *    (src/error.rs) — a documented exception aimed at the caller who already
 *    holds the id. The console feed is not that caller, which is why these
 *    cards render their own `ErrorCard` instead of falling through to the
 *    console's unredacted default error view.
 *  - PROGRAM OUTPUT. A script that calls back into the engine prints its own
 *    runtime id to stdout/stderr.
 *  - SUBMITTED SOURCE. That same script carries the id as a string literal,
 *    and `register_function` publishes source verbatim.
 *
 * So this renders all three cards over settled / running / preview / error and
 * asserts the uuid never appears in any of them.
 *
 * Renders through `react-dom/server` against a stubbed `@iii-dev/console-ui`
 * (the real package's JS entry throws — it is compile-time-only, served at
 * runtime by the console's import map, see packages/console-ui/index.js). The
 * stub still renders every prop it is handed (`code`, `children`), so a
 * renderer that stopped routing text through `redactRuntimeIds` is caught here
 * rather than hidden behind an inert mock.
 */

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
  Host,
} from '@iii-dev/console-ui'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { redactRuntimeIdsDeep, truncateRuntimeId } from '../lib/shared'
import { createRegisterFunctionRenderer } from './register-function'
import { createRunRenderer } from './run'
import { createTeardownRenderer } from './teardown'

// Hoisted above these imports by vitest, so every renderer module above
// resolves the stub, never the real package's throwing JS entry.
vi.mock('@iii-dev/console-ui', () => ({
  Tooltip: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children?: React.ReactNode }) => (
    <>{children}</>
  ),
  TooltipContent: ({ children }: { children?: React.ReactNode }) => (
    <span data-stub="tooltip-content">{children}</span>
  ),
  CodeHighlight: ({ code }: { code: string }) => (
    <pre data-stub="code-highlight">{code}</pre>
  ),
  JsonHighlight: ({ code }: { code: string }) => (
    <pre data-stub="json-highlight">{code}</pre>
  ),
}))

/** `manager.rs` mints `rt-<uuid>` (`format!("rt-{}", Uuid::new_v4())`). */
const RUNTIME_ID = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const TRUNCATED = truncateRuntimeId(RUNTIME_ID)

/** A script that calls back into the engine carries its runtime id inline —
 * so the id is in the SOURCE, in STDOUT and in STDERR, not just the field. */
const CODE = [
  'import { evalIn } from "engine"',
  `const rt = "${RUNTIME_ID}"`,
  'console.log("running in", rt)',
  'await evalIn(rt, "1 + 1")',
].join('\n')
const STDOUT = `running in ${RUNTIME_ID}\ndone\n`
const STDERR = `Error: connect ECONNREFUSED\n    at evalIn (${RUNTIME_ID}/vm.js:3:9)\n`

const HOST = {} as unknown as Host

function baseMsg(
  input: unknown,
  extra: Partial<FunctionTriggerMessage> = {},
): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId: extra.functionId ?? '',
    input,
    createdAt: 0,
    ...extra,
  }
}

/** Every HTML string the renderer produces for one payload, across the
 * settled / running / preview states — mirroring how the console actually
 * calls a `FunctionTriggerRenderer` (see FunctionTriggerCard.tsx). */
function renderAllStates(
  renderer: FunctionTriggerRenderer,
  functionId: string,
  input: unknown,
  output: unknown,
): string[] {
  const settled = renderer.tryRender(
    baseMsg(input, { functionId, output, running: false }),
  )
  const running = (renderer.tryRenderRunning ?? renderer.tryRender)(
    baseMsg(input, { functionId, running: true }),
  )
  const preview = renderer.tryRenderPreview?.(
    baseMsg(input, { functionId, pendingApproval: true }),
  )
  return [settled, running, preview]
    .filter((n): n is React.ReactNode => n !== null && n !== undefined)
    .map((n) => renderToStaticMarkup(n))
}

/** One node per op: renderer, its function id, a request/response pair whose
 * every free-text field embeds the runtime id, and an error output carrying
 * one of `error.rs`'s two real id-quoting messages. */
const CARDS: {
  name: string
  renderer: FunctionTriggerRenderer
  functionId: string
  input: unknown
  output: unknown
  errorOutput: unknown
}[] = [
  {
    name: 'run',
    renderer: createRunRenderer(HOST),
    functionId: 'sandbox-code-runner::run',
    input: {
      code: CODE,
      runtime_id: RUNTIME_ID,
      lang: 'node',
      network: true,
      timeout_ms: 4000,
    },
    output: {
      runtime_id: RUNTIME_ID,
      stdout: STDOUT,
      stderr: STDERR,
      exit_code: 1,
      success: false,
      duration_ms: 128,
    },
    // error.rs `Expired`.
    errorOutput: {
      error: { message: `runtime ${RUNTIME_ID} expired: idle for 600s` },
    },
  },
  {
    name: 'register_function',
    renderer: createRegisterFunctionRenderer(HOST),
    functionId: 'sandbox-code-runner::register_function',
    input: {
      runtime_id: RUNTIME_ID,
      // A caller is free to name a function after its runtime — nothing stops
      // them, so the id can arrive through `function_id` and `description` too.
      function_id: `${RUNTIME_ID}::handler`,
      source: CODE,
      description: `handler for ${RUNTIME_ID}`,
    },
    output: { function_id: `${RUNTIME_ID}::handler`, registered: true },
    // error.rs `RuntimeNotFound`.
    errorOutput: { error: { message: `unknown runtime_id ${RUNTIME_ID}` } },
  },
  {
    name: 'teardown',
    renderer: createTeardownRenderer(HOST),
    functionId: 'sandbox-code-runner::teardown',
    input: { runtime_id: RUNTIME_ID },
    output: {
      runtime_id: RUNTIME_ID,
      torn_down: true,
      unregistered: [`${RUNTIME_ID}::a`, `${RUNTIME_ID}::b`],
    },
    errorOutput: { error: { message: `unknown runtime_id ${RUNTIME_ID}` } },
  },
]

describe('runtime id redaction across the injected UI', () => {
  for (const card of CARDS) {
    it(`${card.name}: the full runtime id never reaches the DOM (settled/running/preview)`, () => {
      const htmls = renderAllStates(
        card.renderer,
        card.functionId,
        card.input,
        card.output,
      )
      // All three cards render in all three states — a card that quietly
      // stopped rendering would otherwise pass this test by abstention.
      expect(htmls).toHaveLength(3)
      for (const html of htmls) {
        expect(html).not.toContain(RUNTIME_ID)
      }
      // Not vacuous: every state actually surfaced the truncated id, so these
      // payloads exercise the capability-bearing fields rather than skip them.
      for (const html of htmls) {
        expect(html).toContain(TRUNCATED)
      }
    })

    it(`${card.name}: an error output is rendered here, redacted, not fallen through`, () => {
      const message = baseMsg(card.input, {
        functionId: card.functionId,
        output: card.errorOutput,
        running: false,
      })
      const node = card.renderer.tryRender(message)
      // Falling through (null) would hand the message to the console's default
      // error view, which prints error.rs's id-quoting message verbatim.
      expect(node).not.toBeNull()
      const html = renderToStaticMarkup(node)
      expect(html).not.toContain(RUNTIME_ID)
      expect(html).toContain(TRUNCATED)
    })
  }
})

/**
 * The deep walker behind `redactRaw` — what the console runs over the raw
 * request/response before the `raw json` tab renders them and before its
 * copy button copies them. The card's own redaction stops at what the card
 * draws; this is the other exit.
 */
describe('redactRuntimeIdsDeep', () => {
  it('redacts ids nested in objects, arrays and object KEYS', () => {
    const value = {
      runtime_id: RUNTIME_ID,
      registered: [`sandbox-code-runner::${RUNTIME_ID}::foo`, { nested: [{ deep: STDERR }] }],
      // A payload keyed by runtime id: the key itself is the capability.
      [`runtime:${RUNTIME_ID}`]: { note: `owned by ${RUNTIME_ID}` },
    }
    const out = redactRuntimeIdsDeep(value)
    expect(JSON.stringify(out)).not.toContain(RUNTIME_ID)
    expect(JSON.stringify(out)).toContain(TRUNCATED)
    // Positive control: the input really did carry the secret everywhere.
    expect(JSON.stringify(value)).toContain(RUNTIME_ID)
  })

  it('preserves shape and leaves the input untouched', () => {
    const value = {
      n: 1,
      b: false,
      nil: null,
      list: [1, 'a', null, [true]],
      obj: { k: 'v' },
    }
    const snapshot = JSON.stringify(value)
    expect(redactRuntimeIdsDeep(value)).toEqual(value)
    expect(redactRuntimeIdsDeep(value)).not.toBe(value)
    expect(JSON.stringify(value)).toBe(snapshot)
    expect(redactRuntimeIdsDeep(RUNTIME_ID)).toBe(TRUNCATED)
    expect(redactRuntimeIdsDeep(undefined)).toBeUndefined()
    expect(redactRuntimeIdsDeep(null)).toBeNull()
  })

  it('terminates on a self-referential value instead of hanging', () => {
    const cyclic: Record<string, unknown> = { runtime_id: RUNTIME_ID }
    cyclic.self = cyclic
    cyclic.list = [cyclic]
    const out = redactRuntimeIdsDeep(cyclic) as Record<string, unknown>
    expect(out.runtime_id).toBe(TRUNCATED)
    expect(out.self).toBe('[circular]')
    expect(out.list).toEqual(['[circular]'])
  })

  it('redacts a value repeated on two branches (not mistaken for a cycle)', () => {
    const shared = { id: RUNTIME_ID }
    const out = redactRuntimeIdsDeep({ a: shared, b: shared })
    expect(out).toEqual({ a: { id: TRUNCATED }, b: { id: TRUNCATED } })
  })
})
