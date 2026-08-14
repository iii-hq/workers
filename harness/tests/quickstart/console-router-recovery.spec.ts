import { execFile } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { expect, type Page, test, type WebSocketRoute } from '@playwright/test'

const execFileAsync = promisify(execFile)
const SONNET_LABEL = /claude[\s-]+sonnet[\s-]+5/i

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is required`)
  return value
}

async function iiiJson(args: string[]): Promise<Record<string, unknown>> {
  const iiiBin = required('HARNESS_QUICKSTART_III_BIN')
  const projectDir = required('HARNESS_QUICKSTART_PROJECT_DIR')
  const { stdout } = await execFileAsync(iiiBin, args, {
    cwd: projectDir,
    env: process.env,
    maxBuffer: 4 * 1024 * 1024,
  })
  return JSON.parse(stdout) as Record<string, unknown>
}

async function routerId(): Promise<string | null> {
  const port = required('HARNESS_QUICKSTART_ENGINE_PORT')
  const result = await iiiJson([
    'trigger',
    'engine::workers::list',
    '--port',
    port,
    '--json',
    '{}',
  ])
  const workers = Array.isArray(result.workers) ? result.workers : []
  const router = workers.find(
    (worker) =>
      worker &&
      typeof worker === 'object' &&
      (worker as Record<string, unknown>).name === 'llm-router',
  ) as Record<string, unknown> | undefined
  return typeof router?.id === 'string' ? router.id : null
}

async function modelCount(): Promise<number> {
  const port = required('HARNESS_QUICKSTART_ENGINE_PORT')
  const result = await iiiJson([
    'trigger',
    'router::models::list',
    '--port',
    port,
    '--json',
    '{}',
  ])
  return Array.isArray(result.models) ? result.models.length : 0
}

async function expectSonnetInPicker(
  page: Page,
  timeout = 30_000,
): Promise<void> {
  const picker = page.getByRole('button', { name: /^model(?::|\s|$)/i })
  await expect(picker).toBeEnabled({ timeout })
  await picker.click()

  const anthropic = page.getByRole('menuitem', { name: /^anthropic/i })
  await expect(anthropic).toBeVisible()
  if ((await anthropic.getAttribute('aria-expanded')) !== 'true') {
    await anthropic.click()
  }
  await expect(
    page.getByRole('menuitemradio', { name: SONNET_LABEL }),
  ).toHaveCount(1, { timeout })
  await page.keyboard.press('Escape')
}

async function runWorkerCommand(command: 'restart' | 'start' | 'stop') {
  const iiiBin = required('HARNESS_QUICKSTART_III_BIN')
  const projectDir = required('HARNESS_QUICKSTART_PROJECT_DIR')
  await execFileAsync(iiiBin, ['worker', command, 'llm-router'], {
    cwd: projectDir,
    env: process.env,
    timeout: 120_000,
    maxBuffer: 4 * 1024 * 1024,
  })
}

test('recovers the model catalogue after the router is replaced without reloading the page', async ({
  page,
}) => {
  const consoleUrl = required('HARNESS_QUICKSTART_CONSOLE_URL')
  const artifactsRoot = required('HARNESS_QUICKSTART_ARTIFACTS_DIR')

  await page.goto(consoleUrl)
  const chatTab = page.getByRole('tab', { name: /^chat \+ traces/i })
  await chatTab.click()
  await expect(chatTab).toHaveAttribute('aria-selected', 'true')
  await expectSonnetInPicker(page)

  const documentMarker = 'quickstart-router-recovery-document'
  await page.evaluate((marker) => {
    Reflect.set(window, '__quickstartRouterRecovery', marker)
  }, documentMarker)

  const beforeId = await routerId()
  expect(beforeId).toBeTruthy()

  // Restart only llm-router. The engine, Console worker, browser
  // document, and browser-to-engine WebSocket all remain alive.
  await runWorkerCommand('restart')

  await expect
    .poll(routerId, { timeout: 60_000, intervals: [250, 500, 1_000] })
    .not.toBe(beforeId)
  const afterId = await routerId()
  expect(afterId).toBeTruthy()

  await expect
    .poll(modelCount, { timeout: 60_000, intervals: [250, 500, 1_000] })
    .toBeGreaterThan(0)
  await expectSonnetInPicker(page)

  await expect(
    page.evaluate(() => Reflect.get(window, '__quickstartRouterRecovery')),
  ).resolves.toBe(documentMarker)

  await mkdir(artifactsRoot, { recursive: true })
  await writeFile(
    path.join(artifactsRoot, 'router-recovery-browser-evidence.json'),
    `${JSON.stringify(
      {
        schema_version: 1,
        router_worker_id_before: beforeId,
        router_worker_id_after: afterId,
        backend_model_count: await modelCount(),
        same_browser_document: true,
        model_picker_recovered_without_reload: true,
      },
      null,
      2,
    )}\n`,
  )
})

test('recovers presence when the router starts during a browser WebSocket outage', async ({
  page,
}) => {
  const consoleUrl = required('HARNESS_QUICKSTART_CONSOLE_URL')
  const artifactsRoot = required('HARNESS_QUICKSTART_ARTIFACTS_DIR')
  let allowBrowserConnection = true
  let browserSocket: WebSocketRoute | null = null

  await page.routeWebSocket('**/ws', (socket) => {
    browserSocket = socket
    if (allowBrowserConnection) {
      socket.connectToServer()
    } else {
      void socket.close({ code: 1001, reason: 'quickstart controlled outage' })
    }
  })

  await page.goto(consoleUrl)
  const chatTab = page.getByRole('tab', { name: /^chat \+ traces/i })
  await chatTab.click()
  await expect(chatTab).toHaveAttribute('aria-selected', 'true')
  await expectSonnetInPicker(page)

  const documentMarker = 'quickstart-presence-reconnect-document'
  await page.evaluate((marker) => {
    Reflect.set(window, '__quickstartPresenceReconnect', marker)
  }, documentMarker)

  await runWorkerCommand('stop')
  await expect
    .poll(routerId, { timeout: 30_000, intervals: [250, 500, 1_000] })
    .toBeNull()
  await expect(
    page.getByRole('button', { name: /^model(?::|\s|$)/i }),
  ).toBeDisabled()

  allowBrowserConnection = false
  const activeSocket = browserSocket as WebSocketRoute | null
  if (!activeSocket)
    throw new Error('Console did not open its engine WebSocket')
  await activeSocket.close({
    code: 1001,
    reason: 'quickstart controlled outage',
  })

  try {
    // The router becomes healthy while the browser cannot receive its worker
    // arrival event. Reconnect must therefore re-read workers::list.
    await runWorkerCommand('start')
    await expect
      .poll(routerId, { timeout: 60_000, intervals: [250, 500, 1_000] })
      .not.toBeNull()
    await expect
      .poll(modelCount, { timeout: 60_000, intervals: [250, 500, 1_000] })
      .toBeGreaterThan(0)

    allowBrowserConnection = true
    await expectSonnetInPicker(page, 60_000)
    await expect(
      page.evaluate(() => Reflect.get(window, '__quickstartPresenceReconnect')),
    ).resolves.toBe(documentMarker)

    await mkdir(artifactsRoot, { recursive: true })
    await writeFile(
      path.join(artifactsRoot, 'presence-reconnect-browser-evidence.json'),
      `${JSON.stringify(
        {
          schema_version: 1,
          router_started_while_browser_disconnected: true,
          backend_model_count: await modelCount(),
          same_browser_document: true,
          model_picker_recovered_without_reload: true,
        },
        null,
        2,
      )}\n`,
    )
  } finally {
    allowBrowserConnection = true
    if ((await routerId().catch(() => null)) === null) {
      await runWorkerCommand('start').catch(() => undefined)
    }
  }
})
