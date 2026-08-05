/**
 * The run card's own checks — the rules that are specific to THIS card and
 * that a wrong answer on would be a lie in the feed:
 *
 *  - a non-zero exit reads as the script failing, never as a system error;
 *  - the three fall-through states (non-record input, missing output, no
 *    readable `code` on an approval prompt) return null so the console's own
 *    card handles them, instead of asserting something that did not happen;
 *  - an error output renders here, redacted, rather than falling through to
 *    the default view that would print the runtime_id capability verbatim;
 *  - every block is capped;
 *  - an approval-gate excerpt is labeled as one and never has a line count
 *    quoted off it.
 *
 * Renders through `react-dom/server` against a stubbed `@iii-dev/console-ui`
 * (the real package's JS entry throws by design — it is compile-time-only,
 * served at runtime by the console's import map, see packages/console-ui).
 * The stub renders every prop it is handed, so a renderer that stopped
 * redacting would still be caught here rather than hidden behind an inert
 * mock.
 */

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
  Host,
} from '@iii-dev/console-ui'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { truncateRuntimeId } from '../lib/shared'
import { createRunRenderer } from './run'

vi.mock('@iii-dev/console-ui', () => ({
  Tooltip: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children?: React.ReactNode }) => (
    <>{children}</>
  ),
  TooltipContent: ({ children }: { children?: React.ReactNode }) => (
    <span data-stub="tooltip-content">{children}</span>
  ),
  CodeHighlight: ({ code, language }: { code: string; language: string }) => (
    <pre data-stub="code-highlight" data-language={language}>
      {code}
    </pre>
  ),
  JsonHighlight: ({ code }: { code: string }) => (
    <pre data-stub="json-highlight">{code}</pre>
  ),
}))

const FUNCTION_ID = 'sandbox-code-runner::run'
const RUNTIME_ID = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const TRUNCATED = truncateRuntimeId(RUNTIME_ID)

const renderer: FunctionTriggerRenderer = createRunRenderer({} as Host)

function msg(extra: Partial<FunctionTriggerMessage>): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId: FUNCTION_ID,
    input: {},
    createdAt: 0,
    ...extra,
  }
}

const html = (node: React.ReactNode) => renderToStaticMarkup(node)

/** A well-formed settled call: request + process result. */
function settled(
  input: Record<string, unknown>,
  output: Record<string, unknown>,
): string {
  const node = renderer.tryRender(msg({ input, output, running: false }))
  expect(node).not.toBeNull()
  return html(node)
}

const OK_RES = {
  runtime_id: RUNTIME_ID,
  stdout: 'hello\n',
  stderr: '',
  exit_code: 0,
  success: true,
  duration_ms: 12,
}

describe('createRunRenderer', () => {
  it('claims only its own function id', () => {
    expect(renderer.isMatch(FUNCTION_ID)).toBe(true)
    for (const other of [
      'sandbox-code-runner::teardown',
      'sandbox-code-runner::register_function',
      'sandbox-code-runner::inject-guidance',
      'node-engine::eval',
    ]) {
      expect(renderer.isMatch(other)).toBe(false)
      expect(
        renderer.tryRender(
          msg({ functionId: other, input: { code: 'x' }, output: OK_RES }),
        ),
      ).toBeNull()
    }
  })
})

describe('the process result', () => {
  it('renders stdout, the exit code and the duration', () => {
    const out = settled({ code: 'console.log("hello")', lang: 'node' }, OK_RES)
    expect(out).toContain('exit 0')
    expect(out).toContain('clean exit')
    expect(out).toContain('12ms')
    expect(out).toContain('hello')
    expect(out).toContain('data-language="javascript"')
  })

  /** The distinction the card exists for. */
  it('reads a non-zero exit as the script failing, not a system error', () => {
    const out = settled(
      { code: 'raise SystemExit(2)', lang: 'python' },
      {
        ...OK_RES,
        stdout: '',
        stderr: 'Traceback (most recent call last):\nSystemExit: 2\n',
        exit_code: 2,
        success: false,
      },
    )
    expect(out).toContain('exit 2')
    expect(out).toContain('its own message is in stderr')
    expect(out).toContain('cr-ui-exit-code failed')
    expect(out).toContain('SystemExit: 2')
    // warn, never alert — alert is reserved for infrastructure failures.
    expect(out).not.toContain('cr-ui-alert')
    expect(out).toContain('data-language="python"')
  })

  it('says nothing was printed when both streams are empty', () => {
    const out = settled({ code: 'x = 1' }, { ...OK_RES, stdout: '' })
    expect(out).toContain('no output on stdout or stderr')
  })

  /** Malformed entries get a placeholder, never a silent drop. */
  it('flags a stream that is not a string instead of dropping it', () => {
    const out = settled({ code: 'x' }, { ...OK_RES, stdout: { oops: 1 } })
    expect(out).toContain('stdout was not a string')
  })

  it('says so rather than inventing one when the exit code is missing', () => {
    const out = settled({ code: 'x' }, { runtime_id: RUNTIME_ID, stdout: '' })
    expect(out).toContain('exit ?')
    expect(out).toContain('no exit code in the response')
  })

  /** A reused runtime that omitted `lang` cannot be highlighted honestly. */
  it('renders unhighlighted rather than guessing a language', () => {
    const out = settled({ code: 'x = 1', runtime_id: RUNTIME_ID }, OK_RES)
    expect(out).toContain('data-language="text"')
  })
})

describe('the runtime capability', () => {
  it('never puts the full runtime id in the DOM, in any state', () => {
    const input = { code: `connect("${RUNTIME_ID}")`, runtime_id: RUNTIME_ID }
    const states = [
      renderer.tryRender(msg({ input, output: OK_RES, running: false })),
      renderer.tryRenderRunning?.(msg({ input, running: true })),
      renderer.tryRenderPreview?.(msg({ input, pendingApproval: true })),
      renderer.tryRender(
        msg({
          input,
          running: false,
          output: {
            error: {
              message: `sandbox-code-runner::expired: runtime ${RUNTIME_ID} expired: its idle VM was reaped`,
            },
          },
        }),
      ),
    ]
    expect(states.every((n) => n !== null && n !== undefined)).toBe(true)
    for (const node of states) {
      const out = html(node)
      expect(out).not.toContain(RUNTIME_ID) // incl. the id inside the source
      expect(out).toContain(TRUNCATED)
    }
  })

  /** error.rs quotes the id by design, so we must render errors ourselves. */
  it('renders an error output itself, redacted, instead of falling through', () => {
    const node = renderer.tryRender(
      msg({
        input: { code: 'x' },
        running: false,
        output: { error: `unknown runtime_id ${RUNTIME_ID}` },
      }),
    )
    expect(node).not.toBeNull()
    const out = html(node)
    expect(out).not.toContain(RUNTIME_ID)
    expect(out).toContain(TRUNCATED)
    expect(out).toContain('unknown runtime_id')
  })
})

/**
 * A gate DENIAL is not an `ErrorCard`-shaped infrastructure failure: the call
 * never reached a runtime. `errorInfo`'s `'error' in output` check also
 * matches the gate's DenialEnvelope shape (approval-gate/src/types.rs), so
 * this must be caught first and rendered distinctly.
 */
describe('a gate denial', () => {
  const DENIAL_OUTPUT = {
    error: {
      kind: 'function_error',
      message: 'Permission denied: sandbox-code-runner::run matched rule no-eval.',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'permissions',
        function_id: FUNCTION_ID,
        rule_id: 'no-eval',
        rule_action: 'deny',
        reason: 'Permission denied: sandbox-code-runner::run matched rule no-eval.',
        // The gate's own excerpt of what the caller submitted — including
        // the runtime_id the caller passed — is the leak this test rules out.
        args_excerpt: {
          runtime_id: RUNTIME_ID,
          code: `evalIn("${RUNTIME_ID}")`,
        },
      },
    },
  }

  it('reads as "never ran", not as an infrastructure failure', () => {
    const node = renderer.tryRender(
      msg({
        input: { runtime_id: RUNTIME_ID },
        running: false,
        output: DENIAL_OUTPUT,
      }),
    )
    expect(node).not.toBeNull()
    const out = html(node)
    expect(out).toContain('denied at the gate')
    expect(out).toContain('never ran')
    expect(out).toContain('permissions')
    // Never the alert tone `ErrorCard` uses for an infrastructure failure.
    expect(out).not.toContain('cr-ui-alert')
  })

  it('never prints the args_excerpt runtime id, and shows no RuntimeChip', () => {
    const node = renderer.tryRender(
      msg({
        input: { runtime_id: RUNTIME_ID },
        running: false,
        output: DENIAL_OUTPUT,
      }),
    )
    const out = html(node)
    expect(out).not.toContain(RUNTIME_ID)
    // No RuntimeChip at all — a call the gate denied never touched a runtime.
    expect(out).not.toContain(TRUNCATED)
  })
})

describe('the runtime-origin and network chips', () => {
  /**
   * Three cases, and the card must name exactly which one applies: no
   * runtime_id and no keep is a one-shot (the default, nothing persists);
   * no runtime_id with keep: true mints a runtime to keep; a runtime_id in
   * the request is a reuse of one the caller already holds.
   */
  it('names one-shot, kept, or reused correctly', () => {
    const oneShot = settled({ code: 'x' }, OK_RES)
    expect(oneShot).toContain('>one-shot<')
    expect(oneShot).not.toContain('>keeps the VM<')
    expect(oneShot).not.toContain('>reused runtime<')

    const kept = settled({ code: 'x', keep: true }, OK_RES)
    expect(kept).toContain('>keeps the VM<')
    expect(kept).not.toContain('>one-shot<')

    expect(settled({ code: 'x', runtime_id: RUNTIME_ID }, OK_RES)).toContain(
      'reused runtime',
    )
  })

  /**
   * Neither a one-shot run nor `keep: true` can ever create a networked
   * VM (`sandbox::run` has no network flag at all), so `network: true`
   * without a `runtime_id` always means the worker will refuse the call —
   * the card must say so, not claim network is "on". `network: false`
   * needs no callout: it is simply guaranteed on this path.
   */
  it('flags network: true without a runtime_id as refused, and stays quiet when false', () => {
    const refused = settled({ code: 'x', network: true }, OK_RES)
    expect(refused).toContain('refused: no runtime_id')
    expect(refused).not.toMatch(/network <\/span>on\b/)

    const quiet = settled({ code: 'x' }, OK_RES)
    expect(quiet).not.toContain('network')
  })

  /** run.rs: "Ignored when `runtime_id` is set" — do not claim otherwise. */
  it('reports network as ignored on a reused runtime', () => {
    const out = settled(
      { code: 'x', runtime_id: RUNTIME_ID, network: true },
      OK_RES,
    )
    expect(out).toContain('ignored on reuse')
  })

  it('shows the timeout the request asked for', () => {
    expect(settled({ code: 'x', timeout_ms: 4000 }, OK_RES)).toContain('4000ms')
  })
})

describe('size caps', () => {
  it('clamps a chatty stdout instead of flooding the chat', () => {
    const stdout = Array.from({ length: 500 }, (_, i) => `line ${i}`).join('\n')
    const out = settled({ code: 'x' }, { ...OK_RES, stdout })
    expect(out).toContain('line 0')
    expect(out).not.toContain('line 499')
    expect(out).toContain('expand')
  })

  it('clamps a long program, and a one-line minified one', () => {
    const long = Array.from({ length: 400 }, (_, i) => `let a${i} = ${i}`).join(
      '\n',
    )
    const many = settled({ code: long }, OK_RES)
    expect(many).not.toContain('a399')
    expect(many).toContain('400 lines')

    const oneLine = settled({ code: 'x'.repeat(50_000) }, OK_RES)
    expect(oneLine.length).toBeLessThan(20_000)
    expect(oneLine).toContain('expand')
  })
})

describe('falling through', () => {
  /** A double-encoded payload: the default card decodes it, this one cannot. */
  it('falls through on a non-record input in every state', () => {
    for (const input of [JSON.stringify({ code: 'x' }), null, ['code'], 7]) {
      expect(renderer.tryRender(msg({ input, output: OK_RES }))).toBeNull()
      expect(renderer.tryRenderRunning?.(msg({ input, running: true }))).toBe(
        null,
      )
      expect(
        renderer.tryRenderPreview?.(msg({ input, pendingApproval: true })),
      ).toBeNull()
    }
  })

  /** An aborted call, or a reloaded session whose last call never paired. */
  it('falls through when there is no response body and nothing is running', () => {
    for (const output of [undefined, 'not-a-record', 42]) {
      expect(
        renderer.tryRender(msg({ input: { code: 'x' }, output })),
      ).toBeNull()
    }
  })

  it('leaves the approval prompt alone when it carries no readable code', () => {
    expect(
      renderer.tryRenderPreview?.(
        msg({ input: { runtime_id: RUNTIME_ID }, pendingApproval: true }),
      ),
    ).toBeNull()
    // …and never renders a settled/running card for a pending message.
    expect(
      renderer.tryRender(msg({ input: { code: 'x' }, pendingApproval: true })),
    ).toBeNull()
  })
})

describe('the approval preview', () => {
  it('shows the code that is about to run', () => {
    const out = html(
      renderer.tryRenderPreview?.(
        msg({
          input: { code: 'print(1)', lang: 'python', network: true },
          pendingApproval: true,
        }),
      ),
    )
    expect(out).toContain('will run this code')
    expect(out).toContain('print(1)')
    expect(out).toContain('>one-shot<')
    expect(out).toContain('data-language="python"')
  })

  /**
   * The gate clips every string to 256 code points with a trailing `…`
   * (approval-gate ARGS_EXCERPT_LEN_CAP): label it, and never quote a line
   * count off a string that is not the whole program.
   */
  it('labels an approval-gate excerpt and quotes no line count from it', () => {
    const excerpt = `${Array.from({ length: 40 }, (_, i) => `l${i}`).join('\n')}…`
    const out = html(
      renderer.tryRenderPreview?.(
        msg({ input: { code: excerpt }, pendingApproval: true }),
      ),
    )
    expect(out).toContain('code (excerpt)')
    expect(out).toContain('clipped to 256 characters by the approval gate')
    expect(out).toContain('expand')
    expect(out).not.toContain('40 lines')
  })
})
