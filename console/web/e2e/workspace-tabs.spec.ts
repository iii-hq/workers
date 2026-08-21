import type { Page } from '@playwright/test'
import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'streamed-text' })

const strip = (page: Page) =>
  page.getByRole('tablist', { name: 'Workspace tabs' })
const tabs = (page: Page) => strip(page).getByRole('tab')
const activeTab = (page: Page) => strip(page).locator('[aria-selected="true"]')

async function settle(page: Page): Promise<void> {
  await page.locator('body').click({ position: { x: 4, y: 4 } })
}

test('workspace tabs stay deterministic across keys, reloads, deep links and other browsers', async ({
  page,
  stack,
  browser,
}) => {
  const completed = stack.waitForTurnCompleted()
  await stack.trigger()
  expect(await completed).toMatchObject({ status: 'completed' })

  await page.setViewportSize({ width: 1280, height: 800 })
  await openSession(page, stack)
  await expect(tabs(page)).toHaveCount(1)
  const homeId = await activeTab(page).getAttribute('data-tab-id')

  // Create two workspaces from the keyboard; each becomes active. A new
  // empty workspace focuses its search field, so leave it before the next key.
  await settle(page)
  await page.keyboard.press('t')
  await expect(tabs(page)).toHaveCount(2)
  await settle(page)
  await page.keyboard.press('t')
  await expect(tabs(page)).toHaveCount(3)
  await settle(page)
  const thirdId = await activeTab(page).getAttribute('data-tab-id')
  expect(thirdId).not.toBe(homeId)

  // Step with [ and ], jump with digits.
  await page.keyboard.press('[')
  await expect(activeTab(page)).not.toHaveAttribute(
    'data-tab-id',
    thirdId ?? '',
  )
  await settle(page)
  await page.keyboard.press(']')
  await expect(activeTab(page)).toHaveAttribute('data-tab-id', thirdId ?? '')
  await settle(page)
  await page.keyboard.press('1')
  await expect(activeTab(page)).toHaveAttribute('data-tab-id', homeId ?? '')

  // Shortcuts stand down while typing.
  const composer = page.getByLabel('message composer')
  await composer.click()
  await page.keyboard.press('t')
  await expect(tabs(page)).toHaveCount(3)
  await expect(composer).toHaveText('t')
  await page.keyboard.press('Backspace')
  await settle(page)

  // Middle tab closes onto its right-hand neighbour.
  await page.keyboard.press('2')
  const secondId = await activeTab(page).getAttribute('data-tab-id')
  await settle(page)
  await page.keyboard.press('Shift+W')
  await expect(tabs(page)).toHaveCount(2)
  await expect(activeTab(page)).toHaveAttribute('data-tab-id', thirdId ?? '')
  expect(secondId).not.toBe(thirdId)

  // Order, the active tab and the server copy survive a reload. Reload until
  // the server copy shows the close instead of guessing how long the
  // serialized write takes.
  await expect
    .poll(
      async () => {
        await page.reload()
        await strip(page).waitFor()
        return tabs(page).count()
      },
      { intervals: [1_000] },
    )
    .toBe(2)
  await expect(activeTab(page)).toHaveAttribute('data-tab-id', thirdId ?? '')
  const orderAfterReload = await tabs(page).evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute('data-tab-id')),
  )
  expect(orderAfterReload[0]).toBe(homeId)

  // A deep link opens the screen once and never a second tab for it.
  await page.goto(`${stack.consoleUrl}#/workers`)
  await expect(tabs(page).filter({ hasText: 'workers' })).toHaveCount(1)
  await page.goto(`${stack.consoleUrl}#/traces`)
  await page.goto(`${stack.consoleUrl}#/workers`)
  await expect(tabs(page).filter({ hasText: 'workers' })).toHaveCount(1)
  await expect(tabs(page)).toHaveCount(2)

  // Another browser's click never switches this one.
  const other = await browser.newPage()
  try {
    await other.goto(stack.consoleUrl)
    await expect(tabs(other)).toHaveCount(2)
    const mineBefore = await activeTab(page).getAttribute('data-tab-id')
    await tabs(other)
      .nth(mineBefore === homeId ? 1 : 0)
      .click()
    await expect(activeTab(other)).not.toHaveAttribute(
      'data-tab-id',
      mineBefore ?? '',
    )
    // Longer than one poll interval: the other pointer has reached us by now.
    await page.waitForTimeout(6_500)
    await expect(activeTab(page)).toHaveAttribute(
      'data-tab-id',
      mineBefore ?? '',
    )
  } finally {
    await other.close()
  }

  // On a phone the bottom sheet switches workspaces.
  await page.setViewportSize({ width: 375, height: 812 })
  await page.getByRole('button', { name: 'open workspace menu' }).click()
  const list = page.getByRole('list', { name: 'Workspaces' })
  await expect(list.getByRole('listitem')).toHaveCount(2)
  await list.getByRole('listitem').nth(0).getByRole('button').first().click()
  await expect(page.getByRole('list', { name: 'Workspaces' })).toHaveCount(0)
  await page.setViewportSize({ width: 1280, height: 800 })
  await expect(activeTab(page)).toHaveAttribute('data-tab-id', homeId ?? '')

  expectPassingResult(await stack.finish())
})
