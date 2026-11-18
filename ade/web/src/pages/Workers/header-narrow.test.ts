import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const css = readFileSync(new URL('./workers.css', import.meta.url), 'utf8')
const page = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8')

/** The body of the first `@container (max-width: 560px)` block. */
function narrowPaneRules(): string {
  const at = css.indexOf('@container (max-width: 560px)')
  expect(at).toBeGreaterThan(-1)
  let depth = 0
  for (let index = css.indexOf('{', at); index < css.length; index++) {
    if (css[index] === '{') depth++
    if (css[index] === '}' && --depth === 0) return css.slice(at, index)
  }
  return css.slice(at)
}

function rule(source: string, selector: string): string {
  const at = source.indexOf(`${selector} {`)
  if (at < 0) return ''
  return source.slice(at, source.indexOf('}', at))
}

// QA 2026-11-16 F13: in a narrow pane "+ Add worker" grew to a 48px block
// (taller than every other header control, and the bar) and clipped to
// "Add worl". A narrow desktop pane keeps the button's 32px and folds it to
// its +; 48px touch targets are the phone's (below 640px) alone.
describe('Workers header in a narrow pane', () => {
  it('folds Add worker to its icon at its own 32px, never 48px', () => {
    const narrow = narrowPaneRules()
    expect(narrow).not.toMatch(/\.wk-header\s*{[^}]*min-height:\s*56px/)
    const add = rule(narrow, '.wk-add-worker')
    expect(add).toMatch(/height:\s*2rem/)
    expect(add).not.toMatch(/48px/)
    expect(rule(narrow, '.wk-add-worker-label')).toMatch(/display:\s*none/)
  })

  it('keeps the button named when only its icon shows', () => {
    expect(page).toMatch(/aria-label="Add worker"/)
    expect(page).toMatch(/className="wk-add-worker-label">Add worker</)
  })

  it('keeps 48px touch targets for phones only', () => {
    const phone = css.slice(css.indexOf('@media (max-width: 640px)'))
    expect(phone).toMatch(/\.wk-add-worker\s*{[^}]*height:\s*48px/)
  })
})
