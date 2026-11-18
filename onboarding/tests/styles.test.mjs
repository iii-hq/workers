import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const css = readFileSync(join(root, 'ui', 'styles.css'), 'utf8')

/** The declarations of the first rule whose selector ends in `selector`. */
function rule(selector) {
  const at = css.indexOf(`${selector} {`)
  assert.ok(at >= 0, `no rule for ${selector}`)
  return css.slice(at, css.indexOf('}', at))
}

// QA 2026-11-16 F8: on agent steps the "Message to the agent" box showed
// five lines and scrolled the rest inside the card with no sign of more, so
// the message read as cut off. It is what the button sends: shown whole.
test('the message to the agent is never clipped to a few lines', () => {
  const text = rule(".learn-prompt-text")
  assert.doesNotMatch(text, /max-height/)
  assert.doesNotMatch(text, /overflow(-y)?:\s*(auto|hidden|scroll)/)
  assert.doesNotMatch(text, /line-clamp/)
})

test('a long step still scrolls with the card body', () => {
  assert.match(rule('.learn-overlay-scroll'), /overflow-y:\s*auto/)
})
