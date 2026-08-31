/**
 * The family renderer's dispatch rules and the §4.1 upgrades, rendered
 * to markup: the 15-id membership, error-before-success, fall-through
 * on unparseable payloads, the never-throw fence, the exit-reason
 * verdict, the S003 `slots busy` chip, the 1 MiB truncation chip, the
 * SGR spans, and the sandbox-id copy+jump chip.
 *
 * Renders through `react-dom/server` against a stubbed
 * `@iii-dev/console-ui` (the real package's JS entry throws by design —
 * it is compile-time-only, served at runtime by the console's import
 * map, see packages/console-ui).
 */

import type { FunctionTriggerMessage, FunctionTriggerRenderer, Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { createSandboxFamilyRenderer, SandboxFunctionIdLabel } from '../index'

vi.mock('@iii-dev/console-ui', () => ({
  Badge: ({ children, variant }: { children?: ReactNode; variant?: string }) => (
    <span data-stub="badge" data-variant={variant}>
      {children}
    </span>
  ),
  StatusDot: ({ tone, pulse }: { tone?: string; pulse?: boolean }) => (
    <span data-stub="status-dot" data-tone={tone} data-pulse={pulse} />
  ),
  EmptyState: ({ title, description }: { title: string; description: string }) => (
    <div data-stub="empty-state">
      {title} — {description}
    </div>
  ),
  Tooltip: ({ children }: { children?: ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children?: ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children?: ReactNode }) => <span data-stub="tooltip-content">{children}</span>,
  CodeHighlight: ({ code, language }: { code: string; language: string }) => (
    <pre data-stub="code-highlight" data-language={language}>
      {code}
    </pre>
  ),
  JsonHighlight: ({ code }: { code: string }) => <pre data-stub="json-highlight">{code}</pre>,
}))

const SB = '00000000-0000-0000-0000-000000000000'

const renderer: FunctionTriggerRenderer = createSandboxFamilyRenderer({} as Host)

const ALL_IDS = [
  'sandbox::exec',
  'sandbox::run',
  'sandbox::create',
  'sandbox::stop',
  'sandbox::list',
  'sandbox::fs::ls',
  'sandbox::fs::stat',
  'sandbox::fs::read',
  'sandbox::fs::write',
  'sandbox::fs::mkdir',
  'sandbox::fs::rm',
  'sandbox::fs::mv',
  'sandbox::fs::chmod',
  'sandbox::fs::grep',
  'sandbox::fs::sed',
]

function msg(extra: Partial<FunctionTriggerMessage>): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId: 'sandbox::exec',
    input: {},
    createdAt: 0,
    ...extra,
  }
}

const html = (node: ReactNode) => renderToStaticMarkup(node)

const EXEC_INPUT = { sandbox_id: SB, cmd: 'cargo', args: ['test'] }
const EXEC_OK = {
  stdout: 'ok\n',
  stderr: '',
  exit_code: 0,
  timed_out: false,
  duration_ms: 42,
  success: true,
}

describe('dispatch', () => {
  it('claims exactly the 15 sandbox ids', () => {
    for (const id of ALL_IDS) {
      expect(renderer.isMatch(id)).toBe(true)
    }
    for (const other of [
      'sandbox::fs',
      'sandbox::exec2',
      'sandbox-code-runner::run',
      'shell::exec',
      'sandbox::fs::readx',
    ]) {
      expect(renderer.isMatch(other)).toBe(false)
    }
  })

  it('has the contracted renderer id', () => {
    expect(renderer.id).toBe('sandbox-code-runner/page.js#sandbox-family')
  })

  it('returns null for non-family ids and unparseable payloads', () => {
    expect(renderer.tryRender(msg({ functionId: 'shell::exec', input: EXEC_INPUT, output: EXEC_OK }))).toBeNull()
    // Input that fails the request schema falls through.
    const node = renderer.tryRender(msg({ functionId: 'sandbox::exec', input: { nope: 1 }, output: EXEC_OK }))
    expect(node === null || html(node) === '').toBe(true)
  })

  it('leaves pending-approval messages to tryRenderPreview', () => {
    expect(renderer.tryRender(msg({ input: EXEC_INPUT, output: EXEC_OK, pendingApproval: true }))).toBeNull()
    const preview = renderer.tryRenderPreview?.(msg({ input: EXEC_INPUT, pendingApproval: true }))
    expect(html(preview)).toContain('cargo test')
  })

  it('never throws, even on hostile payloads', () => {
    const cyclic: Record<string, unknown> = {}
    cyclic.self = cyclic
    for (const hostile of [cyclic, { content: 0, details: null }, Object.create(null)]) {
      // html() forces every child component body to execute — tryRender
      // alone only runs the element factories.
      expect(() => html(renderer.tryRender(msg({ input: hostile, output: hostile })))).not.toThrow()
    }
  })
})

describe('the settled exec card', () => {
  it('renders the command, streams, verdict and duration', () => {
    const out = html(renderer.tryRender(msg({ input: EXEC_INPUT, output: EXEC_OK })))
    expect(out).toContain('cargo test')
    expect(out).toContain('ok')
    expect(out).toContain('exit 0')
    expect(out).toContain('cr-fam-pill ok')
    expect(out).toContain('42ms')
  })

  it('unwraps the harness envelope around the response', () => {
    const wrapped = {
      content: [{ type: 'text', text: JSON.stringify(EXEC_OK) }],
      details: EXEC_OK,
      terminate: false,
    }
    const out = html(renderer.tryRender(msg({ input: EXEC_INPUT, output: wrapped })))
    expect(out).toContain('exit 0')
  })

  it('renders the running shimmer through tryRenderRunning', () => {
    const out = html(renderer.tryRenderRunning?.(msg({ input: EXEC_INPUT, running: true })))
    expect(out).toContain('triggering…')
    expect(out).toContain('cargo test')
  })
})

describe('the exit-reason verdict (§4.1.1)', () => {
  function verdict(over: Record<string, unknown>): string {
    return html(renderer.tryRender(msg({ input: EXEC_INPUT, output: { ...EXEC_OK, ...over } })))
  }

  it('folds a timeout into the single pill', () => {
    const out = verdict({ exit_code: null, timed_out: true, duration_ms: 1500 })
    expect(out).toContain('timed out @ 1.5s')
    expect(out).toContain('cr-fam-pill warn')
    expect(out).not.toContain('>no exit<')
  })

  it('names not-found and not-executable', () => {
    expect(verdict({ exit_code: 127, success: false })).toContain('not found (127)')
    expect(verdict({ exit_code: 126, success: false })).toContain('not executable (126)')
    expect(
      verdict({
        exit_code: 127,
        success: false,
        stderr: 'exec: pytohn: not found\n',
      }),
    ).toContain('not found (127)')
  })

  it('keeps a plain failing exit as alert', () => {
    const out = verdict({ exit_code: 2, success: false })
    expect(out).toContain('exit 2')
    expect(out).toContain('cr-fam-pill alert')
  })
})

describe('ANSI SGR colors (§4.1.2)', () => {
  it('maps SGR runs onto token classes inside the stream', () => {
    const out = html(
      renderer.tryRender(
        msg({
          input: EXEC_INPUT,
          output: {
            ...EXEC_OK,
            stdout: 'test result: \x1b[32mok\x1b[0m. 3 passed; \x1b[1;31m0 failed\x1b[0m',
          },
        }),
      ),
    )
    expect(out).toContain('cr-fam-ansi-ok')
    expect(out).toContain('cr-fam-ansi-alert')
    expect(out).toContain('cr-fam-ansi-bold')
    expect(out).not.toContain('\x1b')
  })
})

describe('the 1 MiB truncation chip (§4.1.3)', () => {
  it('flags a stream that filled the daemon buffer', () => {
    const out = html(
      renderer.tryRender(
        msg({
          input: EXEC_INPUT,
          output: { ...EXEC_OK, stdout: 'x'.repeat(1024 * 1024) },
        }),
      ),
    )
    expect(out).toContain('output truncated at 1 MiB')
  })

  it('stays quiet below the cap', () => {
    const out = html(renderer.tryRender(msg({ input: EXEC_INPUT, output: { ...EXEC_OK, stdout: 'short' } })))
    expect(out).not.toContain('output truncated')
  })
})

describe('error-before-success and the S-code card', () => {
  it('renders the wire error card instead of the per-op view', () => {
    const out = html(
      renderer.tryRender(
        msg({
          input: EXEC_INPUT,
          output: {
            type: 'validation',
            code: 'S003',
            message: `concurrent exec on sandbox ${SB}: an exec is already in flight`,
            retryable: false,
            fix_note: 'wait for the in-flight exec, then retry',
          },
        }),
      ),
    )
    expect(out).toContain('S003')
    expect(out).toContain('slots busy') // §4.1.4
    expect(out).toContain('wait for the in-flight exec')
    expect(out).not.toContain('cr-fam-term-head')
  })

  it('keeps S200 partial streams by re-parsing fix as an ExecResponse', () => {
    const out = html(
      renderer.tryRender(
        msg({
          input: EXEC_INPUT,
          output: {
            type: 'execution',
            code: 'S200',
            message: 'exec timed out after 1500 ms',
            fix: {
              stdout: 'partial output before the kill',
              stderr: '',
              exit_code: null,
              timed_out: true,
              duration_ms: 1500,
              success: false,
            },
          },
        }),
      ),
    )
    expect(out).toContain('S200')
    expect(out).toContain('partial output before the kill')
    expect(out).not.toContain('slots busy')
  })

  it('renders the dispatch-denied card with the blocked id', () => {
    const out = html(
      renderer.tryRender(
        msg({
          functionId: 'sandbox::fs::write',
          input: { sandbox_id: SB, path: '/x' },
          output: {
            error: {
              kind: 'function_error',
              message: "function sandbox::fs::write is not permitted by this agent's dispatch policy",
            },
          },
        }),
      ),
    )
    expect(out).toContain('dispatch policy')
    expect(out).toContain('sandbox::fs::write')
  })
})

describe('the sandbox-id chip (§4.1.5)', () => {
  it('exec card: copy button plus a keyboard-accessible open affordance', () => {
    const out = html(renderer.tryRender(msg({ input: EXEC_INPUT, output: EXEC_OK })))
    expect(out).toContain(`aria-label="copy sandbox id ${SB}"`)
    expect(out).toContain(`aria-label="open sandbox ${SB} in the fleet page"`)
    expect(out).toContain('cr-fam-sbx-open')
  })

  it('create card carries the chip for the minted id', () => {
    const out = html(
      renderer.tryRender(
        msg({
          functionId: 'sandbox::create',
          input: { image: 'node' },
          output: { sandbox_id: SB, image: 'node' },
        }),
      ),
    )
    expect(out).toContain(`copy sandbox id ${SB}`)
    expect(out).toContain('cr-fam-sbx-open')
  })

  it('create card masks env values — names only, never the secret', () => {
    const out = html(
      renderer.tryRender(
        msg({
          functionId: 'sandbox::create',
          input: { image: 'node', env: { API_KEY: 'sk-super-secret' } },
          output: { sandbox_id: SB, image: 'node' },
        }),
      ),
    )
    expect(out).toContain('API_KEY')
    expect(out).toContain('••••')
    expect(out).not.toContain('sk-super-secret')
  })

  it('fs cards carry the chip too', () => {
    const out = html(
      renderer.tryRender(
        msg({
          functionId: 'sandbox::fs::ls',
          input: { sandbox_id: SB, path: '/app' },
          output: { entries: [] },
        }),
      ),
    )
    expect(out).toContain(`copy sandbox id ${SB}`)
    expect(out).toContain('directory is empty')
  })

  it('the stop card keeps copy but drops the jump — nothing to open', () => {
    const out = html(
      renderer.tryRender(
        msg({
          functionId: 'sandbox::stop',
          input: { sandbox_id: SB },
          output: { sandbox_id: SB, stopped: true },
        }),
      ),
    )
    expect(out).toContain(`copy sandbox id ${SB}`)
    expect(out).not.toContain('cr-fam-sbx-open')
    expect(out).toContain('stopped sandbox')
  })
})

describe('the list card', () => {
  it('renders the EmptyState on an empty fleet', () => {
    const out = html(
      renderer.tryRender(
        msg({
          functionId: 'sandbox::list',
          input: {},
          output: { sandboxes: [] },
        }),
      ),
    )
    expect(out).toContain('no sandboxes')
  })

  it('renders rows with the status dot', () => {
    const out = html(
      renderer.tryRender(
        msg({
          functionId: 'sandbox::list',
          input: {},
          output: {
            sandboxes: [
              {
                sandbox_id: SB,
                name: 'worker',
                image: 'node',
                age_secs: 90,
                exec_in_progress: true,
                stopped: false,
              },
            ],
          },
        }),
      ),
    )
    expect(out).toContain('worker')
    expect(out).toContain('1m')
    expect(out).toContain('data-stub="status-dot"')
  })
})

describe('SandboxFunctionIdLabel', () => {
  it('fades the sandbox:: prefix and keeps the tail strong', () => {
    const out = html(<SandboxFunctionIdLabel functionId="sandbox::fs::grep" />)
    expect(out).toContain('cr-fam-fnid-prefix')
    expect(out).toContain('sandbox::')
    expect(out).toContain('fs::grep')
  })

  it('renders foreign ids whole', () => {
    const out = html(<SandboxFunctionIdLabel functionId="web::fetch" />)
    expect(out).toContain('web::fetch')
    expect(out).not.toContain('cr-fam-fnid-prefix')
  })
})
