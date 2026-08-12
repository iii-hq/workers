/**
 * Render assertions go through `renderToStaticMarkup` (no jsdom —
 * console/web's convention, see redact-raw.test.tsx). Click flows can't
 * run without a DOM, so the copy affordance's insecure-origin fallback is
 * pinned at the source level instead: the handler must funnel through
 * `copyTextToClipboard` (async clipboard API first, execCommand textarea
 * second — covered by src/lib/clipboard.test.ts), never raw
 * `navigator.clipboard`, which is undefined over `http://<LAN-IP>`.
 */

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { Badge } from './Badge'
import { TerminalCommandLine } from './TerminalCommandLine'

const SRC = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), 'TerminalCommandLine.tsx'),
  'utf8',
)

describe('TerminalCommandLine', () => {
  it('renders the prompt glyph and the command', () => {
    const html = renderToStaticMarkup(
      <TerminalCommandLine command="cargo test --all" />,
    )
    expect(html).toContain('$')
    expect(html).toContain('cargo test --all')
  })

  it('supports a custom prompt glyph', () => {
    const html = renderToStaticMarkup(
      <TerminalCommandLine command="ls" prompt="❯" />,
    )
    expect(html).toContain('❯')
  })

  it('ellipsizes with the full command riding on title', () => {
    const command = `echo ${'x'.repeat(300)}`
    const html = renderToStaticMarkup(<TerminalCommandLine command={command} />)
    expect(html).toContain('truncate')
    expect(html).toContain(`title="${command}"`)
  })

  it('renders the trailing chip slot', () => {
    const html = renderToStaticMarkup(
      <TerminalCommandLine
        command="pnpm build"
        chips={<Badge variant="accent">exit 0</Badge>}
      />,
    )
    expect(html).toContain('exit 0')
  })

  it('renders the copy affordance only when asked', () => {
    const withCopy = renderToStaticMarkup(
      <TerminalCommandLine command="ls" copy />,
    )
    expect(withCopy).toContain('copy command')
    const without = renderToStaticMarkup(<TerminalCommandLine command="ls" />)
    expect(without).not.toContain('copy command')
  })

  it('funnels the copy through the insecure-origin-safe helper', () => {
    expect(SRC).toContain("from '@/lib/clipboard'")
    expect(SRC).toContain('copyTextToClipboard(command)')
    expect(SRC).not.toContain('navigator.clipboard')
  })

  it('carries both flash outcomes, copied and failed', () => {
    expect(SRC).toContain("'copied'")
    expect(SRC).toContain("'failed'")
  })

  it('tracks the flash timer so re-copies and unmount clear it', () => {
    // Rapid clicks must not let a stale timer cut a fresh flash short,
    // and a pending reset must not fire into an unmounted component.
    expect(SRC).toContain('window.clearTimeout(flashRef.current)')
    expect(SRC).toContain('useEffect')
  })
})
