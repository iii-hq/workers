/**
 * `code-runner::teardown` card — every state rendered for real, with the
 * capability rule (a full `rt-<uuid>` never reaches the DOM) asserted on each.
 *
 * Renders against a stubbed `@iii-dev/console-ui`: the real package's JS entry
 * throws by design (it is compile-time-only, served at runtime by the
 * console's import map).
 */
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { createTeardownRenderer } from './teardown'

vi.mock('@iii-dev/console-ui', () => ({
  Tooltip: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children?: React.ReactNode }) => (
    <>{children}</>
  ),
  TooltipContent: () => null,
}))

const ID = 'rt-3f9a2c1e-4b5d-6e7f-8a9b-0c1d2e3f4a5b'
// biome-ignore lint/suspicious/noExplicitAny: test doubles
const r = createTeardownRenderer({} as any)
// biome-ignore lint/suspicious/noExplicitAny: test doubles
const msg = (over: any) => ({
  id: 'm',
  role: 'function-trigger',
  functionId: 'code-runner::teardown',
  input: { runtime_id: ID },
  createdAt: 0,
  ...over,
})
// biome-ignore lint/suspicious/noExplicitAny: test doubles
const html = (node: any) => renderToStaticMarkup(node)

describe('teardown card', () => {
  it('matches only its own op', () => {
    expect(r.isMatch('code-runner::teardown')).toBe(true)
    expect(r.isMatch('code-runner::eval')).toBe(false)
    expect(r.isMatch('code-runner::inject-guidance')).toBe(false)
    expect(r.tryRender(msg({ functionId: 'code-runner::eval' }))).toBeNull()
  })

  it('empty unregistered reads as normal', () => {
    const out = html(
      r.tryRender(
        msg({ output: { runtime_id: ID, torn_down: true, unregistered: [] } }),
      ),
    )
    expect(out).toContain('registered no functions')
    expect(out).toContain('sandbox microVM(s) were stopped')
    expect(out).not.toContain(ID)
    expect(out).toContain('rt-3f9a…')
  })

  it('lists the ids that stopped resolving', () => {
    const out = html(
      r.tryRender(
        msg({
          output: {
            runtime_id: ID,
            torn_down: true,
            unregistered: ['app::a', 'app::b'],
          },
        }),
      ),
    )
    expect(out).toContain('2 function ids no longer resolve')
    expect(out).toContain('app::a')
    expect(out).toContain('app::b')
  })

  it('redacts a runtime id embedded in a function id', () => {
    const out = html(
      r.tryRender(
        msg({ output: { torn_down: true, unregistered: [`app::${ID}`] } }),
      ),
    )
    expect(out).not.toContain(ID)
  })

  it('keeps malformed entries visible', () => {
    const out = html(
      r.tryRender(
        msg({ output: { torn_down: true, unregistered: ['app::a', 7, null] } }),
      ),
    )
    expect(out).toContain('3 function ids')
    expect(out).toContain('malformed entry 2')
    expect(out).toContain('malformed entry 3')
  })

  it('clamps a long list and offers expansion', () => {
    const ids = Array.from({ length: 40 }, (_, i) => `app::f${i}`)
    const out = html(
      r.tryRender(msg({ output: { torn_down: true, unregistered: ids } })),
    )
    expect(out).toContain('40 function ids')
    expect(out).toContain('expand · 40 ids')
    expect(out).toContain('app::f11')
    expect(out).not.toContain('app::f12')
  })

  it('does not claim a count when unregistered is absent', () => {
    const out = html(
      r.tryRender(msg({ output: { runtime_id: ID, torn_down: true } })),
    )
    expect(out).toContain('did not list which function ids')
  })

  it('reflects torn_down:false', () => {
    const out = html(
      r.tryRender(
        msg({ output: { runtime_id: ID, torn_down: false, unregistered: [] } }),
      ),
    )
    expect(out).toContain('NOT torn down')
    expect(out).toContain('cr-ui-warn')
  })

  it('unwraps the harness envelope', () => {
    const out = html(
      r.tryRender(
        msg({
          output: {
            content: [{ type: 'text', text: '{}' }],
            details: { torn_down: true, unregistered: ['app::a'] },
          },
        }),
      ),
    )
    expect(out).toContain('app::a')
  })

  it('renders errors itself, redacted', () => {
    const out = html(
      r.tryRender(
        msg({ output: { error: { message: `unknown runtime_id ${ID}` } } }),
      ),
    )
    expect(out).not.toContain(ID)
    expect(out).toContain('unknown runtime_id rt-3f9a…')
    expect(out).toContain('cr-ui-alert')
  })

  it('redacts the Expired message shape too', () => {
    const out = html(
      r.tryRender(
        msg({
          output: { error: `runtime ${ID} expired: its idle VM was reaped` },
        }),
      ),
    )
    expect(out).not.toContain(ID)
  })

  /**
   * A gate DENIAL means no runtime was ever touched — the `'error' in
   * output` shape `errorInfo` matches also matches the gate's
   * DenialEnvelope, so this must be caught first and read as "never ran"
   * rather than the infrastructure-failure `ErrorCard`.
   */
  it('a gate denial reads as "never ran", not as an infrastructure failure, and leaks nothing', () => {
    const out = html(
      r.tryRender(
        msg({
          output: {
            error: {
              kind: 'function_error',
              message: 'Rejected by operator.',
              details: {
                schema_version: 1,
                status: 'denied',
                denied_by: 'user',
                function_id: 'code-runner::teardown',
                reason: 'Rejected by operator.',
                args_excerpt: { runtime_id: ID },
              },
            },
          },
        }),
      ),
    )
    expect(out).toContain('denied at the gate')
    expect(out).toContain('never ran')
    expect(out).toContain('user')
    expect(out).not.toContain('cr-ui-alert')
    expect(out).not.toContain(ID)
    // No RuntimeChip — a call the gate denied never touched a runtime.
    expect(out).not.toContain('rt-3f9a…')
  })

  it('falls through when there is no output', () => {
    expect(r.tryRender(msg({}))).toBeNull()
    expect(r.tryRender(msg({ output: undefined, running: false }))).toBeNull()
    expect(r.tryRender(msg({ output: 'not a record' }))).toBeNull()
  })

  it('renders a running card without asserting an outcome', () => {
    const out = html(r.tryRenderRunning?.(msg({ running: true })))
    expect(out).toContain('tearing down')
    expect(out).not.toContain('destroyed')
    expect(out).toContain('rt-3f9a…')
    expect(out).not.toContain(ID)
    // running with a non-record input: card, no chip, no claim
    const bare = html(
      r.tryRenderRunning?.(msg({ running: true, input: '{"runtime_id":"x"}' })),
    )
    expect(bare).toContain('tearing down')
    expect(bare).not.toContain('cr-ui-rt')
  })

  it('previews only on the approval gate, and falls through on non-record input', () => {
    expect(r.tryRenderPreview?.(msg({}))).toBeNull()
    expect(r.tryRender(msg({ pendingApproval: true }))).toBeNull()
    const out = html(r.tryRenderPreview?.(msg({ pendingApproval: true })))
    expect(out).toContain('will destroy this runtime')
    expect(out).not.toContain(ID)
    expect(
      r.tryRenderPreview?.(
        msg({ pendingApproval: true, input: '{"runtime_id":"x"}' }),
      ),
    ).toBeNull()
    expect(
      r.tryRenderPreview?.(msg({ pendingApproval: true, input: null })),
    ).toBeNull()
  })
})

/**
 * The other addressing mode: `namespace` (every runtime — one per language
 * — backing a `register_function` namespace), which carries no capability
 * of its own, so it renders as plain text rather than through `RuntimeChip`.
 */
describe('teardown by namespace', () => {
  const nsMsg = (over: object) =>
    msg({ input: { namespace: 'app' }, ...over })

  it('shows a namespace chip, not a runtime chip, when settled', () => {
    const out = html(
      r.tryRender(
        nsMsg({
          output: {
            namespace: 'app::',
            torn_down: true,
            unregistered: ['app::a', 'app::b'],
          },
        }),
      ),
    )
    expect(out).toContain('namespace')
    expect(out).toContain('app::')
    expect(out).not.toContain('cr-ui-rt')
    expect(out).toContain('2 function ids no longer resolve')
    expect(out).toContain('sandbox microVM(s) were stopped')
  })

  it('previews destroying every runtime backing the namespace', () => {
    const out = html(r.tryRenderPreview?.(nsMsg({ pendingApproval: true })))
    expect(out).toContain('will destroy every runtime backing this namespace')
    expect(out).toContain('app')
  })

  it('shows the namespace while running, with no runtime chip', () => {
    const out = html(r.tryRenderRunning?.(nsMsg({ running: true })))
    expect(out).toContain('tearing down')
    expect(out).toContain('app')
    expect(out).not.toContain('cr-ui-rt')
  })

  it('renders an error for a namespace teardown without a runtime chip', () => {
    const out = html(
      r.tryRender(
        nsMsg({
          output: { error: 'no runtime is registered for namespace "app::"' },
        }),
      ),
    )
    expect(out).toContain('no runtime is registered')
    expect(out).not.toContain('cr-ui-rt')
  })
})
