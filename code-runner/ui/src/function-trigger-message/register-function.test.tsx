/**
 * The register_function card, rendered for real through `react-dom/server`.
 *
 * What this file is actually guarding: the claims the card makes. It must
 * never print the runtime_id capability (not even out of an error message,
 * which code-runner quotes it into by design), never assert an outcome that
 * did not happen (no output, non-record input), never claim a language the
 * request does not carry, and never flood the feed with an unbounded source.
 *
 * Renders against a stubbed `@iii-dev/console-ui` — the real package's JS
 * entry throws by design (compile-time-only; the console serves it at runtime
 * through its import map).
 */

import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { truncateRuntimeId } from '../lib/shared'
import { createRegisterFunctionRenderer } from './register-function'

vi.mock('@iii-dev/console-ui', () => ({
  Tooltip: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children?: React.ReactNode }) => (
    <>{children}</>
  ),
  TooltipContent: ({ children }: { children?: React.ReactNode }) => (
    <span data-stub="tooltip-content">{children}</span>
  ),
  CodeHighlight: ({ code, language }: { code: string; language: string }) => (
    <pre data-stub="code-highlight" data-lang={language}>
      {code}
    </pre>
  ),
}))

const FUNCTION_ID = 'code-runner::register_function'
const RUNTIME_ID = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const TRUNCATED = truncateRuntimeId(RUNTIME_ID)
const SOURCE = 'export function handler(payload) {\n  return payload\n}\n'

// This card reads nothing off the host (`void host`), so an empty stand-in is
// the whole fixture.
const renderer = createRegisterFunctionRenderer({} as Host)

function msg(
  over: Partial<FunctionTriggerMessage> = {},
): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId: FUNCTION_ID,
    createdAt: 0,
    input: {
      function_id: 'app::greet',
      source: SOURCE,
      description: 'greets a payload',
      lang: 'node',
    },
    output: { function_id: 'app::greet', registered: true },
    ...over,
  }
}

const html = (node: React.ReactNode | null) =>
  node === null ? null : renderToStaticMarkup(node)

describe('matching', () => {
  it('claims only its own function id', () => {
    expect(renderer.isMatch(FUNCTION_ID)).toBe(true)
    for (const other of [
      'code-runner::eval',
      'code-runner::teardown',
      'code-runner::inject-guidance',
      'node-engine::register_function',
    ]) {
      expect(renderer.isMatch(other)).toBe(false)
      expect(renderer.tryRender(msg({ functionId: other }))).toBeNull()
    }
  })
})

describe('falling through', () => {
  /** (c) the default card decodes double-encoded payloads; this one cannot. */
  it('declines a non-record input instead of claiming there is no source', () => {
    for (const input of [JSON.stringify({ source: SOURCE }), 42, null, []]) {
      expect(renderer.tryRender(msg({ input }))).toBeNull()
      expect(
        renderer.tryRenderPreview?.(msg({ input, pendingApproval: true })),
      ).toBeNull()
    }
  })

  /** (b) aborted call / reloaded session — a normal state, not a settled one. */
  it('declines a settled message with no parseable output', () => {
    for (const output of [undefined, 'oops', 7]) {
      expect(renderer.tryRender(msg({ output, running: false }))).toBeNull()
    }
  })

  it('leaves the approval preview to tryRenderPreview', () => {
    expect(renderer.tryRender(msg({ pendingApproval: true }))).toBeNull()
    expect(
      html(renderer.tryRenderPreview?.(msg({ pendingApproval: true }))),
    ).toContain('will register this function')
  })
})

describe('the settled card', () => {
  it('shows the id, its namespace claim, the description and the source', () => {
    const out = html(renderer.tryRender(msg())) ?? ''
    expect(out).toContain('app::greet')
    expect(out).toContain('claims <code>app::</code> for this runtime')
    expect(out).toContain('greets a payload')
    expect(out).toContain('export function handler(payload)')
    expect(out).toContain('registered')
  })

  it('reports registered:false as refused, and says what it means', () => {
    const out =
      html(
        renderer.tryRender(
          msg({ output: { function_id: 'app::greet', registered: false } }),
        ),
      ) ?? ''
    expect(out).toContain('not registered')
    expect(out).toContain('not callable on the bus')
  })

  it('does not invent a verdict when the response carries no flag', () => {
    const out =
      html(
        renderer.tryRender(msg({ output: { function_id: 'app::greet' } })),
      ) ?? ''
    expect(out).toContain('no `registered` flag')
    expect(out).not.toContain('cr-register-function-status')
  })

  it('flags a response that registered a different id', () => {
    const out =
      html(
        renderer.tryRender(
          msg({ output: { function_id: 'app::other', registered: true } }),
        ),
      ) ?? ''
    expect(out).toContain('the response registered app::other')
  })

  /**
   * The mismatch check requires both ids defined, so a request with no
   * `function_id` at all used to hide the response's id entirely — a card
   * could say "no function_id in the request" beside a green `registered`
   * badge while never showing the id that is actually live on the bus.
   */
  it('shows the response id when the request carried none at all', () => {
    const out =
      html(
        renderer.tryRender(
          msg({
            input: { source: SOURCE, description: 'greets a payload' },
            output: { function_id: 'app::greet', registered: true },
          }),
        ),
      ) ?? ''
    expect(out).toContain('app::greet')
    expect(out).toContain('from the response')
    expect(out).toContain('registered')
    expect(out).toContain('claims <code>app::</code> for this runtime')
  })
})

describe('the runtime id is a capability', () => {
  /**
   * This request carries no `runtime_id` field at all anymore — the
   * namespace runtime is resolved internally — so the settled/running/
   * preview states show no id, truncated or otherwise: there is nothing to
   * show. The error state is different: a message that HAPPENS to embed an
   * id (as error.rs's `Expired`/`RuntimeNotFound` would on the direct
   * eval/teardown paths, or a stray one on some other path) must still come
   * back truncated, never whole — belt-and-braces, since this card never
   * assumes an error message is safe.
   */
  it('never leaks the full id, and truncates one when an error message carries it', () => {
    const noId = [
      renderer.tryRender(msg()),
      renderer.tryRender(msg({ running: true })),
      renderer.tryRenderRunning?.(msg({ running: true })),
      renderer.tryRenderPreview?.(msg({ pendingApproval: true })),
    ]
    for (const node of noId) {
      const out = html(node ?? null) ?? ''
      expect(out).not.toBe('')
      expect(out).not.toContain(RUNTIME_ID)
      expect(out).not.toContain(TRUNCATED)
    }

    const errored = html(
      renderer.tryRender(
        msg({ output: { error: `unknown runtime_id ${RUNTIME_ID}` } }),
      ),
    )
    expect(errored).not.toContain(RUNTIME_ID)
    expect(errored).toContain(TRUNCATED)
  })

  /** (a) code-runner's own error messages quote the id — error.rs. */
  it('renders errors itself, redacted, rather than falling through', () => {
    for (const message of [
      `code-runner::runtime_not_found: unknown runtime_id ${RUNTIME_ID}`,
      `code-runner::expired: runtime ${RUNTIME_ID} expired: its idle VM was reaped`,
    ]) {
      const out = html(renderer.tryRender(msg({ output: { error: message } })))
      expect(out).not.toBeNull()
      expect(out).not.toContain(RUNTIME_ID)
      expect(out).toContain(TRUNCATED)
    }
  })

  it('redacts an id embedded in the function id or the source', () => {
    const out =
      html(
        renderer.tryRender(
          msg({
            input: {
              runtime_id: RUNTIME_ID,
              function_id: `${RUNTIME_ID}::greet`,
              source: `// planted from ${RUNTIME_ID}\nhandler`,
              description: `for ${RUNTIME_ID}`,
            },
          }),
        ),
      ) ?? ''
    expect(out).not.toContain(RUNTIME_ID)
  })
})

/**
 * A gate DENIAL means no source was ever published to the bus — the
 * `'error' in output` shape `errorInfo` matches also matches the gate's
 * DenialEnvelope, so this must be caught first and read as "never ran"
 * rather than one of `ErrorCard`'s infrastructure failures.
 */
describe('a gate denial', () => {
  const DENIAL_OUTPUT = {
    error: {
      kind: 'function_error',
      message: 'Rejected by operator.',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'user',
        function_id: FUNCTION_ID,
        reason: 'Rejected by operator.',
        args_excerpt: {
          runtime_id: RUNTIME_ID,
          function_id: 'app::greet',
          source: SOURCE,
        },
      },
    },
  }

  it('reads as "never ran", not as an infrastructure failure', () => {
    const out = html(renderer.tryRender(msg({ output: DENIAL_OUTPUT })))
    expect(out).not.toBeNull()
    expect(out).toContain('denied at the gate')
    expect(out).toContain('never ran')
    expect(out).toContain('user')
    expect(out).not.toContain('cr-ui-alert')
  })

  it('never prints the args_excerpt runtime id, and shows no RuntimeChip', () => {
    const out = html(renderer.tryRender(msg({ output: DENIAL_OUTPUT }))) ?? ''
    expect(out).not.toContain(RUNTIME_ID)
    expect(out).not.toContain(TRUNCATED)
  })
})

describe('language', () => {
  const langOf = (source: string) => {
    const out = html(renderer.tryRender(msg({ input: { source } }))) ?? ''
    return /data-lang="([^"]*)"/.exec(out)?.[1]
  }

  /** `langOf`'s fixture omits `lang` entirely — the fallback path. */
  it('guesses from the source and says it guessed, when lang is missing', () => {
    expect(langOf('def handler(payload):\n  return payload')).toBe('python')
    expect(langOf(SOURCE)).toBe('javascript')
    const out = html(
      renderer.tryRender(msg({ input: { source: SOURCE } })),
    )
    expect(out).toContain('highlighted as javascript — guessed')
  })

  /** The normal case now: `lang` is on the wire, so no guessing is needed
   * and none is claimed. */
  it('highlights from the request lang, without guessing', () => {
    const out = html(renderer.tryRender(msg())) ?? ''
    expect(out).toContain('data-lang="javascript"')
    expect(out).not.toContain('highlighted as')
    expect(out).not.toContain('guessed')
  })

  /** No `lang` and an unrecognizable source: unhighlighted beats
   * mislabelled. */
  it('falls back to text with no language claim at all', () => {
    expect(langOf('handler = 1')).toBe('text')
    const out = html(renderer.tryRender(msg({ input: { source: 'handler' } })))
    expect(out).not.toContain('highlighted as')
  })

  /** A malformed `lang` (not "node"/"python") is treated the same as
   * missing — never echoed as a language claim. */
  it('falls back to guessing when lang is present but invalid', () => {
    const out =
      html(
        renderer.tryRender(
          msg({ input: { source: SOURCE, lang: 'ruby' } }),
        ),
      ) ?? ''
    expect(out).toContain('data-lang="javascript"')
    expect(out).toContain('highlighted as javascript — guessed')
  })
})

describe('the handler convention', () => {
  it('advises quietly when nothing named handler appears', () => {
    const out =
      html(renderer.tryRender(msg({ input: { source: 'console.log(1)' } }))) ??
      ''
    expect(out).toContain('nothing named `handler`')
    expect(out).not.toContain('cr-ui-alert')
  })

  it('stays silent when it does', () => {
    expect(html(renderer.tryRender(msg()))).not.toContain('nothing named')
  })
})

describe('size caps', () => {
  /** (d) a 5 000-line source must not become a 5 000-line chat message. */
  it('clamps a long source and offers expansion', () => {
    const source = Array.from({ length: 400 }, (_, i) => `// line ${i}`).join(
      '\n',
    )
    const out = html(renderer.tryRender(msg({ input: { source } }))) ?? ''
    expect(out).toContain('// line 0')
    expect(out).not.toContain('// line 399')
    expect(out).toContain('more of 400 lines')
  })

  it('clamps a one-line bundle, which has no newlines to clamp on', () => {
    const out =
      html(
        renderer.tryRender(msg({ input: { source: 'x'.repeat(60_000) } })),
      ) ?? ''
    expect(out.length).toBeLessThan(10_000)
    expect(out).toContain('expand · 60000 chars')
    expect(out).not.toContain('more of')
  })
})

describe('malformed and partial requests', () => {
  /** (e) nothing is dropped silently. */
  it('renders placeholders instead of hiding bad fields', () => {
    const out =
      html(
        renderer.tryRender(
          msg({
            input: { function_id: 42, source: SOURCE, description: [] },
            // No usable id in the response either, so the placeholder text
            // below is not just the id-fallback path from a different test.
            output: {},
          }),
        ),
      ) ?? ''
    expect(out).toContain('no function_id in the request')
    expect(out).toContain('non-string function_id, description')
  })

  it('says the worker refuses an id with no namespace', () => {
    const out =
      html(
        renderer.tryRender(
          msg({ input: { function_id: 'greet', source: SOURCE } }),
        ),
      ) ?? ''
    expect(out).toContain('no namespace')
  })

  it('calls out a missing or empty source', () => {
    expect(
      html(renderer.tryRender(msg({ input: { function_id: 'app::greet' } }))),
    ).toContain('no source in the request')
    expect(html(renderer.tryRender(msg({ input: { source: '' } })))).toContain(
      'empty source',
    )
  })
})

describe('the approval preview', () => {
  /** (f) the gate clips every string to 256 code points + `…`. */
  it('labels a clipped source an excerpt and counts no lines from it', () => {
    const clipped = `${'a\n'.repeat(120)}…`
    const out =
      html(
        renderer.tryRenderPreview?.(
          msg({ pendingApproval: true, input: { source: clipped } }),
        ),
      ) ?? ''
    expect(out).toContain('source (excerpt)')
    expect(out).toContain('clips strings to 256 characters')
    expect(out).not.toContain('more of')
    // The `handler` definition may simply be past the cut — no advisory.
    expect(out).not.toContain('nothing named')
  })

  it('presents an unclipped source as the whole program', () => {
    const out =
      html(renderer.tryRenderPreview?.(msg({ pendingApproval: true }))) ?? ''
    expect(out).toContain('>source<')
    expect(out).not.toContain('excerpt')
    expect(out).not.toContain('clips strings')
  })
})
