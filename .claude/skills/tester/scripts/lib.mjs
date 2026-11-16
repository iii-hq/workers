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

/**
 * Launch Chrome on the ADE. `theme` is 'dark' | 'light'.
 * `video: 'name'` records the context to `<OUT>videos/<name>.webm` (saved on
 * `finish()`), with a visible cursor and a caption strip (`caption()`), so
 * the clip explains itself. Both overlays ignore the pointer and sit in
 * `[data-qa-overlay]`: they are the recorder's, not the product's.
 */
export async function launch({
  width = 1800,
  height = 1000,
  theme = 'dark',
  mobile = false,
  video,
} = {}) {
  // Recordings use Playwright's own Chromium: Chrome's screencast cuts the
  // bottom of the frame off.
  const browser = await chromium.launch({
    ...(video ? {} : { channel: 'chrome' }),
    args: ['--disable-blink-features=AutomationControlled'],
  })
  const viewport = mobile ? { width: 390, height: 844 } : { width, height }
  const videoDir = `${OUT}videos/`
  if (video) mkdirSync(videoDir, { recursive: true })
  const context = await browser.newContext({
    viewport,
    isMobile: mobile,
    hasTouch: mobile,
    colorScheme: theme,
    ...(video ? { recordVideo: { dir: videoDir, size: viewport } } : {}),
  })
  if (video) await context.addInitScript(OVERLAY_SCRIPT)
  const page = await context.newPage()
  const problems = watchProblems(page)
  // Every page the context opens (a second tab, a popup) records too.
  const pages = [page]
  context.on('page', (p) => pages.push(p))
  /** Close the context and keep the clips: `<name>.webm` for the first page,
   *  `<name>-tab<N>.webm` for any other; returns the first path. */
  const finish = async () => {
    const recordings = pages.map((p) => p.video())
    await context.close()
    let first = null
    for (const [i, recording] of recordings.entries()) {
      if (!recording) continue
      const path = `${videoDir}${video}${i === 0 ? '' : `-tab${i + 1}`}.webm`
      await recording.saveAs(path).catch(() => {})
      await recording.delete().catch(() => {})
      first ??= path
    }
    await browser.close()
    return first
  }
  return { browser, context, page, problems, finish }
}

/** Show what the test is doing in the recording's caption strip. */
export async function caption(page, text) {
  await page
    .evaluate((t) => window.__qaCaption?.(t), text)
    .catch(() => {})
  await page.waitForTimeout(400)
}

const OVERLAY_SCRIPT = `(() => {
  if (window.top !== window) return
  const mount = () => {
    if (document.querySelector('[data-qa-overlay]')) return
    const root = document.createElement('div')
    root.setAttribute('data-qa-overlay', '')
    root.style.cssText = 'position:fixed;inset:0;pointer-events:none;z-index:2147483647'
    const dot = document.createElement('div')
    dot.style.cssText = 'position:absolute;width:18px;height:18px;margin:-9px 0 0 -9px;border-radius:50%;background:rgba(255,64,129,.55);border:2px solid #fff;box-shadow:0 0 0 1px #000;transition:transform .12s;left:-40px;top:-40px'
    const cap = document.createElement('div')
    cap.style.cssText = 'position:absolute;left:50%;bottom:10px;transform:translateX(-50%);max-width:92%;padding:6px 12px;border-radius:8px;background:rgba(20,20,20,.88);color:#fff;font:600 13px/1.35 -apple-system,system-ui,sans-serif;box-shadow:0 2px 10px rgba(0,0,0,.4);display:none'
    root.append(dot, cap)
    document.documentElement.append(root)
    addEventListener('mousemove', (e) => { dot.style.left = e.clientX + 'px'; dot.style.top = e.clientY + 'px' }, true)
    addEventListener('mousedown', () => { dot.style.transform = 'scale(.6)' }, true)
    addEventListener('mouseup', () => { dot.style.transform = '' }, true)
    window.__qaCaption = (t) => { cap.textContent = 'QA ▸ ' + t; cap.style.display = t ? 'block' : 'none' }
    if (window.__qaPending) window.__qaCaption(window.__qaPending)
  }
  if (document.documentElement) mount()
  addEventListener('DOMContentLoaded', mount)
})()`

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
