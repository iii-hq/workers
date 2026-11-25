/**
 * Where the terminal's inset lives. The fit addon sizes the terminal from its
 * parent's computed height and width and subtracts only the padding of the
 * terminal element (`.xterm`) itself. Under the console's border-box sizing, a
 * padding on the parent (the viewport) is counted as room for rows, so the
 * terminal gets a row and a column more than fit and the last row is drawn
 * under the status strip.
 */
/// <reference types="node" />
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

// Read from disk: a `?raw` import comes back empty under vitest, which would
// let the viewport check pass on nothing.
const css = readFileSync(new URL('./styles.css', import.meta.url), 'utf8')

/** The declarations of the first rule whose selector is exactly `selector`. */
function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = new RegExp(`(^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css)
  return match ? match[2] : ''
}

describe('the terminal inset', () => {
  it('is not padding on the viewport, which the fit addon would count as rows', () => {
    expect(rule('.agent-terminal-viewport')).not.toMatch(/padding\s*:/)
  })

  it('is padding on the terminal element, which the fit addon subtracts', () => {
    expect(rule('.agent-terminal-viewport .xterm')).toMatch(/padding\s*:\s*8px 4px 8px 12px/)
  })
})
