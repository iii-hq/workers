import type { Page } from '@playwright/test'
import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'streamed-text' })

const strip = (page: Page) =>
  page.getByRole('tablist', { name: 'Workspace tabs' })
const tabs = (page: Page) => strip(page).getByRole('tab')

// The console's own catalog script (`console/catalog-page.js`) registers the
// `functions` and `triggers` pages, so the isolated shell has a worker page
// to render without any extra worker in the stack.
const settingsModifier = process.platform === 'darwin' ? 'Control' : 'Alt'

test('#/worker/<scope> renders one injected page alone and leaves the workspace untouched', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await stack.trigger()
  expect(await completed).toMatchObject({ status: 'completed' })

  await page.setViewportSize({ width: 1280, height: 800 })
  await openSession(page, stack)
  await expect(tabs(page)).toHaveCount(1)

  // A bare scope resolves to the worker's first page and canonicalizes the
  // URL; no tab strip, no chat composer.
  await page.goto(`${stack.consoleUrl}#/worker/console`)
  await expect(page.getByRole('heading', { name: 'Functions' })).toBeVisible()
  await expect(strip(page)).toHaveCount(0)
  await expect(page.getByLabel('message composer')).toHaveCount(0)
  await expect(page).toHaveURL(/#\/worker\/console\/functions$/)
  await expect(page).toHaveTitle('iii - console')

  // The settings shortcut opens the configuration overlay in place; closing
  // it lands back on the same standalone page.
  await page.locator('body').click({ position: { x: 4, y: 4 } })
  await page.keyboard.press(`${settingsModifier}+,`)
  const settings = page.getByRole('dialog', { name: 'Settings' })
  await expect(settings).toBeVisible()
  await expect(strip(page)).toHaveCount(0)
  await page.keyboard.press('Escape')
  await expect(settings).toHaveCount(0)
  await expect(page).toHaveURL(/#\/worker\/console\/functions$/)
  await expect(page.getByRole('heading', { name: 'Functions' })).toBeVisible()

  // A reload stays isolated on the same page.
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Functions' })).toBeVisible()
  await expect(strip(page)).toHaveCount(0)

  // An explicit page id picks another page of the same worker.
  await page.goto(`${stack.consoleUrl}#/worker/console/triggers`)
  await expect(page.getByRole('heading', { name: 'Triggers' })).toBeVisible()
  await expect(strip(page)).toHaveCount(0)

  // The traces explorer has the same standalone shell.
  await page.goto(`${stack.consoleUrl}#/traces`)
  await expect(page.getByRole('heading', { name: 'Traces' })).toBeVisible()
  await expect(strip(page)).toHaveCount(0)
  await expect(page).toHaveTitle('iii - traces')

  // Leaving the shell by hash reloads into the workspace, which never saw
  // any of this: still the one tab, no `functions`/`triggers` screen.
  await page.goto(`${stack.consoleUrl}#/`)
  await expect(tabs(page)).toHaveCount(1)
  await expect(tabs(page).filter({ hasText: 'functions' })).toHaveCount(0)

  expectPassingResult(await stack.finish())
})
