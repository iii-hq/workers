// Deterministic browser acceptance of the real picker; no engine or credentials.
// Run from ade/web: node e2e/xai-discovery.browser.mjs
import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { chromium } from '@playwright/test'
import { createServer } from 'vite'

const artifacts = path.resolve('../../target/xai-feedback-impl-a82d/browser')
await mkdir(artifacts, { recursive: true })
const fixtureContext = path.resolve('e2e/xai-discovery-context.tsx')
const server = await createServer({
  server: { host: '127.0.0.1', port: 0, strictPort: false },
  resolve: { alias: [{ find: '@/lib/conversations-context', replacement: fixtureContext }] },
  plugins: [{
    name: 'xai-isolated-fixture-context',
    enforce: 'pre',
    resolveId(id, importer) {
      if (id === '@/lib/conversations-context' && importer?.endsWith('/ModelPicker.tsx')) return fixtureContext
    },
  }],
})
let browser
try {
  await server.listen()
  const address = server.httpServer.address()
  const base = `http://127.0.0.1:${address.port}`
  browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH ?? '/usr/bin/google-chrome' })
  for (const width of [320, 390, 1280]) {
    for (const theme of ['dark', 'light']) {
      const page = await browser.newPage({ viewport: { width, height: 844 }, reducedMotion: 'reduce' })
      const errors = []
      page.on('pageerror', (error) => errors.push(error.message))
      await page.route(`${base}/xai-fixture`, async (route) => route.fulfill({ contentType: 'text/html', body: await server.transformIndexHtml('/xai-fixture', `<!doctype html><html data-theme="${theme}"><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body class="bg-bg text-ink font-sans"><div id="root"></div><script type="module" src="/e2e/xai-discovery.fixture.tsx"></script></body></html>`) }))
      await page.goto(`${base}/xai-fixture`)
      const trigger = page.locator('button[aria-label^="model"]')
      await trigger.click()
      const group = page.locator('[data-provider-group="xai"]')
      await group.getByText('Credits or spending limit', { exact: true }).waitFor({ timeout: 10000 }).catch(async (error) => { console.log(await page.locator('body').innerText(), errors); throw error })
      assert.equal(await group.locator('details').evaluate((node) => node.open), false)
      assert.equal(await group.locator('[data-model-option]').isDisabled(), true)
      assert.equal(await page.locator('[data-model-option="openai::gpt-5"]').isEnabled(), true)
      const retry = group.getByRole('button', { name: 'Check again', exact: true })
      const link = group.getByRole('link', { name: 'Open xAI console' })
      assert.equal(await link.getAttribute('href'), 'https://console.x.ai')
      await retry.scrollIntoViewIfNeeded()
      await page.evaluate(() => Promise.all(document.getAnimations().map((animation) => animation.finished)))
      const retryBox = await retry.boundingBox()
      const linkBox = await link.boundingBox()
      assert.ok(retryBox.height >= (width < 768 ? 44 : 32) && linkBox.height >= (width < 768 ? 44 : 32))
      assert.ok(linkBox.y >= retryBox.y + retryBox.height)
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
      await group.screenshot({ path: path.join(artifacts, `${width}-${theme}-billing.png`) })
      await group.locator('summary').focus()
      await page.keyboard.press('Enter')
      assert.equal(await group.locator('details').evaluate((node) => node.open), true)
      assert.match(await group.locator('details').innerText(), /HTTP 403 \/ permission-denied/)
      await retry.focus()
      await page.keyboard.press('Enter')
      await group.getByRole('button', { name: 'Checking…' }).waitFor()
      await group.getByText('xAI models updated.').waitFor()
      assert.equal(await page.getByLabel('refresh calls', { exact: true }).innerText(), 'xai')
      assert.equal(await page.getByLabel('selection', { exact: true }).innerText(), 'xai::grok-4')
      assert.equal(await group.locator('[data-model-option]').isEnabled(), true)
      assert.equal(await group.locator('[data-model-option]').evaluate((node) => node === document.activeElement), true)
      await group.screenshot({ path: path.join(artifacts, `${width}-${theme}-recovered.png`) })
      assert.deepEqual(errors, [])
      console.log(`PASS ${width}px ${theme}: shared picker, safe details, touch targets, no overflow, keyboard retry, focus, unchanged selection`)
      await page.close()
    }
  }
} finally {
  await browser?.close()
  await server.close()
}
