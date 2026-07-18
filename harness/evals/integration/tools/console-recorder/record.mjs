// Diagnostics-only console screen recorder (spec: recordings are never an
// oracle). Launched by the integration runner alongside a scenario when
// `--record-console` is set:
//
//   node record.mjs --url <console-url> --out <file.webm> --chrome <path>
//
// Opens the console SPA in headless system Chrome with playwright-core's
// recordVideo, clicks the first conversation row once it appears (the chat
// dock is always rendered; rows come from session::list / live triggers),
// and keeps recording until SIGTERM/SIGINT — then closes the context (which
// flushes the video) and renames it to --out. Never fails the scenario: any
// error here exits non-zero and the runner logs it as a diagnostics loss.

import { chromium } from 'playwright-core'
import { mkdirSync, renameSync } from 'node:fs'
import { dirname } from 'node:path'

function arg(name, fallback = undefined) {
  const index = process.argv.indexOf(`--${name}`)
  if (index === -1 || index + 1 >= process.argv.length) {
    if (fallback !== undefined) return fallback
    console.error(`missing --${name}`)
    process.exit(2)
  }
  return process.argv[index + 1]
}

const url = arg('url')
const out = arg('out')
const chrome = arg('chrome', '/usr/bin/google-chrome')
// Session id of the integration run; rows render as
// aria-label="open <title>" where a fresh session's title starts with its id.
const session = arg('session', '')
const videoDir = `${out}.frames`

mkdirSync(dirname(out), { recursive: true })

const browser = await chromium.launch({
  executablePath: chrome,
  headless: true,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
})
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  recordVideo: { dir: videoDir, size: { width: 1440, height: 900 } },
})
const page = await context.newPage()

browser.on('disconnected', () =>
  console.error(`recorder: browser disconnected at ${new Date().toISOString()}`),
)
page.on('crash', () => console.error('recorder: page crashed'))

let stopping = false
async function stop(code) {
  if (stopping) return
  stopping = true
  const video = page.video()
  // Close steps are individually best-effort: if Chrome already died the
  // context close throws, but the video file written so far is still
  // recoverable via video.path().
  try {
    await context.close() // flushes the recording
  } catch (e) {
    console.error(`recorder: context close: ${e.message ?? e}`)
  }
  try {
    await browser.close()
  } catch (e) {
    console.error(`recorder: browser close: ${e.message ?? e}`)
  }
  try {
    if (video) {
      renameSync(await video.path(), out)
    }
  } catch (e) {
    console.error(`recorder stop failed: ${e}`)
    process.exit(1)
  }
  process.exit(code)
}
process.on('SIGTERM', () => void stop(0))
process.on('SIGINT', () => void stop(0))

try {
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30_000 })
  // Synchronization marker: the runner holds Send until this line appears
  // in the recorder log, so the video always covers the whole turn.
  console.error('recorder: page loaded')
  // The traces view ships with a default display filter
  // (`display: iii.tag.message`) that blanks the pane on a fresh stack;
  // clear it so the recording shows the live trace stream. Best-effort —
  // a missing button just means the layout changed.
  try {
    await page.getByText('clear all', { exact: true }).click({ timeout: 10_000 })
    console.error('recorder: trace filters cleared')
  } catch {
    console.error('recorder: no trace filter to clear')
  }
  // The integration session appears in the conversation sidebar once the
  // harness creates it; open THAT row (not the "new chat" draft) so the
  // recording shows the scenario's transcript. Titles truncate in the UI
  // but aria-labels carry the full title, which starts with the session id.
  const selector = session
    ? `[aria-label^="open ${session.slice(0, 16)}"]`
    : '[aria-label^="open s_"]'
  const row = page.locator(selector).first()
  await row.click({ timeout: 90_000 })
  console.error(`recorder: conversation opened (${selector})`)
} catch (e) {
  // Keep recording whatever is on screen — a blank or erroring console is
  // itself useful diagnostics.
  console.error(`recorder: could not open a conversation: ${e}`)
}

// Record until the runner signals teardown.
await new Promise(() => {})
