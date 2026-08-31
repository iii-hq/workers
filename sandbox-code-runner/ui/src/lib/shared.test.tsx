/**
 * The shell's own checks: the pieces with logic in them (id redaction,
 * language mapping, stream clamping, exit tone) rendered for real through
 * `react-dom/server`.
 *
 * The per-card redaction suite lives beside the renderers
 * (`../function-trigger-message/`), the way node-engine's does; this file
 * covers what those cards build on.
 *
 * Renders against a stubbed `@iii-dev/console-ui` — the real package's JS
 * entry throws by design (it is compile-time-only, served at runtime by the
 * console's import map, see packages/console-ui/index.js).
 */

import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import {
  ExitStatus,
  langToPrism,
  RuntimeChip,
  redactRuntimeIds,
  Stream,
  truncateRuntimeId,
} from './shared'

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

const RUNTIME_ID = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
const TRUNCATED = truncateRuntimeId(RUNTIME_ID)

const html = (node: React.ReactNode) => renderToStaticMarkup(node)

describe('redactRuntimeIds', () => {
  it('replaces a bare runtime id with its truncated form', () => {
    expect(redactRuntimeIds(RUNTIME_ID)).toBe(TRUNCATED)
  })

  /** error.rs quotes the id by design — `unknown runtime_id {id}`. */
  it('replaces the id embedded in sandbox-code-runner own error messages', () => {
    for (const message of [
      `sandbox-code-runner::runtime_not_found: unknown runtime_id ${RUNTIME_ID}`,
      `sandbox-code-runner::expired: runtime ${RUNTIME_ID} expired: its idle VM was reaped`,
    ]) {
      expect(redactRuntimeIds(message)).not.toContain(RUNTIME_ID)
      expect(redactRuntimeIds(message)).toContain(TRUNCATED)
    }
  })

  it('leaves ordinary text and non-uuid-shaped ids untouched', () => {
    expect(redactRuntimeIds('app::save')).toBe('app::save')
    expect(redactRuntimeIds('rt-custom-short-id')).toBe('rt-custom-short-id')
  })

  it('is case-insensitive on the hex groups', () => {
    const upper = RUNTIME_ID.toUpperCase()
    expect(redactRuntimeIds(upper)).toBe(truncateRuntimeId(upper))
  })

  /**
   * `\b` never fires between two word characters, and hex digits ARE word
   * characters — so a runtime id glued to `[A-Za-z0-9_]` on either side
   * matched neither anchor and passed through whole. Proven against the real
   * shapes this worker produces: run stdout/stderr embedding an id inside a
   * filename or identifier, and register_function/teardown ids suffixed with
   * `::a`-style segments.
   */
  it('redacts an id with no non-word character on either side', () => {
    const cases = [
      `${RUNTIME_ID}_worker`,
      `prefix_${RUNTIME_ID}`,
      `/tmp/${RUNTIME_ID}_out.json`,
      `app::${RUNTIME_ID}_a`,
      `${RUNTIME_ID}1`,
    ]
    for (const text of cases) {
      const redacted = redactRuntimeIds(text)
      expect(redacted).not.toContain(RUNTIME_ID)
      expect(redacted).toContain(TRUNCATED)
    }
  })
})

describe('RuntimeChip', () => {
  it('never puts the full id in the DOM, not even in the aria-label', () => {
    const out = html(<RuntimeChip runtimeId={RUNTIME_ID} />)
    expect(out).not.toContain(RUNTIME_ID)
    expect(out).toContain(TRUNCATED)
  })
})

describe('langToPrism', () => {
  it('maps the two runtimes to their Prism ids', () => {
    expect(langToPrism('node')).toBe('javascript')
    expect(langToPrism('python')).toBe('python')
  })

  /** register_function carries no `lang` — guessing one would be a claim. */
  it('is undefined for anything else, including a missing field', () => {
    for (const v of [undefined, null, '', 'js', 'deno', 42, {}]) {
      expect(langToPrism(v)).toBeUndefined()
    }
  })
})

describe('Stream', () => {
  it('renders nothing for an empty stream', () => {
    expect(html(<Stream label="stdout" text="" />)).toBe('')
  })

  it('shows short output whole, with no expand affordance', () => {
    const out = html(<Stream label="stdout" text={'a\nb\nc'} />)
    expect(out).toContain('a\nb\nc')
    expect(out).not.toContain('expand')
  })

  /** A chatty script must not flood the chat: clamp, and say so. */
  it('clamps long output and offers expansion', () => {
    const text = Array.from({ length: 200 }, (_, i) => `line ${i}`).join('\n')
    const out = html(<Stream label="stdout" text={text} />)
    expect(out).toContain('line 0')
    expect(out).not.toContain('line 199')
    expect(out).toContain('expand')
    expect(out).toContain('200 lines')
  })

  /** One 400 KB line has one newline and still has to be clamped. */
  it('clamps on characters too, not just newlines', () => {
    const out = html(<Stream label="stdout" text={'x'.repeat(50_000)} />)
    expect(out.length).toBeLessThan(10_000)
    expect(out).toContain('expand')
  })

  it('redacts a runtime id that reaches program output', () => {
    const out = html(<Stream label="stderr" text={`boom ${RUNTIME_ID}`} />)
    expect(out).not.toContain(RUNTIME_ID)
  })
})

describe('ExitStatus', () => {
  it('reads a zero exit as clean', () => {
    const out = html(<ExitStatus exitCode={0} success durationMs={42} />)
    expect(out).toContain('exit 0')
    expect(out).toContain('clean exit')
    expect(out).toContain('42ms')
    expect(out).toContain('cr-ui-exit-code ok')
  })

  /**
   * The distinction the whole card exists for: a non-zero exit is the user's
   * script failing (warn), never a system error (alert).
   */
  it('reads a non-zero exit as the script failing, not a system error', () => {
    const out = html(<ExitStatus exitCode={1} success={false} />)
    expect(out).toContain('exit 1')
    expect(out).toContain('stderr')
    expect(out).toContain('cr-ui-exit-code failed')
    expect(out).not.toContain('cr-ui-alert')
  })

  it('says so rather than inventing one when the exit code is missing', () => {
    const out = html(<ExitStatus />)
    expect(out).toContain('exit ?')
    expect(out).toContain('no exit code in the response')
  })

  /**
   * `manager.rs`'s `success: …unwrap_or(false)` makes `{exit_code: 0,
   * success: false}` reachable from an honest daemon reply, not just
   * malformed input. The badge must keep saying `exit 0` (that is what
   * happened) while the note stops claiming "exited non-zero" — a claim the
   * badge right next to it disproves.
   */
  it('does not contradict itself on a 0 exit code paired with success: false', () => {
    const out = html(<ExitStatus exitCode={0} success={false} />)
    expect(out).toContain('exit 0')
    expect(out).toContain('cr-ui-exit-code failed')
    expect(out).not.toContain('exited non-zero')
    expect(out).toContain('success: false')
  })

  /** An omitted `success` is not promoted to a claim either way — exit code
   * 0 alone still reads as clean. */
  it('reads a 0 exit with no success field as clean', () => {
    const out = html(<ExitStatus exitCode={0} />)
    expect(out).toContain('clean exit')
    expect(out).toContain('cr-ui-exit-code ok')
  })
})
