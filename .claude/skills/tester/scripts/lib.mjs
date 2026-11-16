// Shared helpers for the tester skill's Playwright checks. Copy this folder
// into ade/web/node_modules/.cache/tester/ and run from there, so
// `@playwright/test` resolves (see SKILL.md, step 0).
//
// Every helper is safe on the person's real stack: sends are captured and
// DROPPED on the WebSocket (nothing reaches an agent), and the workspace
// layout is put back the way it was found.

import { chromium } from '@playwright/test'
import { mkdirSync } from 'node:fs'

export const BASE = process.env.TESTER_URL ?? 'http://localhost:5190/'
export const OUT =
  process.env.TESTER_OUT ??
  new URL('../../../../../.context/tester/', import.meta.url).pathname
mkdirSync(OUT, { recursive: true })

/** Launch Chrome on the ADE. `theme` is 'dark' | 'light'. */
export async function launch({ width = 1800, height = 1000, theme = 'dark', mobile = false } = {}) {
  const browser = await chromium.launch({
    channel: 'chrome',
    args: ['--disable-blink-features=AutomationControlled'],
  })
  const context = await browser.newContext({
    viewport: mobile ? { width: 390, height: 844 } : { width, height },
    isMobile: mobile,
    hasTouch: mobile,
    colorScheme: theme,
  })
  const page = await context.newPage()
  const problems = watchProblems(page)
  return { browser, context, page, problems }
}

/** Console errors, uncaught exceptions and failed requests, as they happen. */
export function watchProblems(page) {
  const problems = []
  page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`))
  page.on('console', (m) => {
    if (m.type() === 'error') problems.push(`console.error: ${m.text().slice(0, 300)}`)
  })
  page.on('requestfailed', (r) =>
    problems.push(`requestfailed: ${r.method()} ${r.url()} ${r.failure()?.errorText ?? ''}`),
  )
  page.on('response', (r) => {
    if (r.status() >= 500) problems.push(`http ${r.status()}: ${r.url()}`)
  })
  return problems
}

/**
 * Capture outgoing engine invocations of `functionIds` and DROP them, so a
 * click on Send/Dismiss/Approve reaches nothing. Call before `page.goto`.
 * Returns the live list of `{ function_id, data }` that were stopped.
 */
export async function captureCalls(page, functionIds = ['harness::send']) {
  const captured = []
  await page.routeWebSocket(/.*/, (ws) => {
    const server = ws.connectToServer()
    ws.onMessage((msg) => {
      const text = typeof msg === 'string' ? msg : msg.toString()
      if (functionIds.some((id) => text.includes(`"function_id":"${id}"`))) {
        try {
          const parsed = JSON.parse(text)
          captured.push({ function_id: parsed.function_id, data: parsed.data })
        } catch {
          captured.push({ function_id: '?', raw: text.slice(0, 500) })
        }
        return
      }
      server.send(msg)
    })
    server.onMessage((msg) => ws.send(msg))
  })
  return captured
}

/**
 * Serve a patched copy of a worker bundle, e.g. to run the "before" side of
 * a fix: `patchBundle(page, 'harness', (js) => js.replace(...))`.
 */
export async function patchBundle(page, scope, patch) {
  await page.route(new RegExp(`/ui/${scope}/page\\.js`), async (route) => {
    const res = await route.fetch()
    await route.fulfill({ response: res, body: patch(await res.text()) })
  })
}

export const panels = (page) => page.locator('section[aria-label^="panel "]')
export const workspaceTabs = (page) =>
  page.locator('[role=tablist] button[aria-label="Close chat"]')

/** What the layout looks like now, to put back at the end. */
export async function snapshotLayout(page) {
  return {
    panels: await panels(page).count(),
    workspaceTabs: await workspaceTabs(page).count(),
    pinned: await page.getByText('Pinned', { exact: true }).count(),
  }
}

/** Close the panes and workspace tabs this run added. */
export async function restoreLayout(page, before) {
  while ((await panels(page).count()) > Math.max(1, before.panels)) {
    await panels(page).last().getByRole('button', { name: 'close panel', exact: true }).first().click()
    await page.waitForTimeout(600)
  }
  while ((await workspaceTabs(page).count()) > before.workspaceTabs) {
    await workspaceTabs(page).last().click({ force: true })
    await page.waitForTimeout(800)
  }
  return snapshotLayout(page)
}

/**
 * Open a chat by title in `scope` (the page or one pane). Uses the row's
 * own `open <title>` button: clicking the title text can land on the row's
 * hover pin button and pin the chat.
 */
export async function openChat(page, title, scope = page) {
  const back = scope.getByRole('button', { name: 'back to chats', exact: true })
  if (await back.count()) {
    await back.first().click()
    await page.waitForTimeout(700)
  }
  await scope.getByRole('button', { name: `open ${title.slice(0, 20)}` }).first().click()
  await page.waitForTimeout(2500)
}

/** Add a pane on the right showing `screen` (an option label, e.g. 'chat'). */
export async function splitRight(page, screen = 'chat') {
  const option = page.getByRole('option', { name: new RegExp(`^${screen}\\b`, 'i') })
  // The first click on the edge control may only reveal its preview.
  for (let i = 0; i < 2 && !(await option.count()); i++) {
    await page.getByRole('button', { name: 'Split right' }).click()
    await page.waitForTimeout(800)
  }
  await option.first().click()
  await page.waitForTimeout(2500)
  return panels(page).last()
}

export const shot = (page, name) => page.screenshot({ path: `${OUT}${name}.png` })
