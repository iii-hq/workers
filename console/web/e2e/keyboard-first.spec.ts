import type { Page } from '@playwright/test'
import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'streamed-text' })

const pane = (page: Page, index: number) =>
  page.locator('[data-workspace-pane-id]').nth(index)
const composer = (page: Page) => page.getByLabel('message composer')

async function settle(page: Page): Promise<void> {
  await page.evaluate(() => {
    const doc = (
      globalThis as {
        document?: { activeElement?: { blur?: () => void } | null }
      }
    ).document
    doc?.activeElement?.blur?.()
  })
}

test('the keyboard reaches the chat, the panes and every page command through ⌘K', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await stack.trigger()
  expect(await completed).toMatchObject({ status: 'completed' })

  await page.setViewportSize({ width: 1280, height: 800 })
  await openSession(page, stack)
  await expect(page.locator('[data-message-row]').first()).toBeVisible()

  // The chat has no bare keys — a letter is a letter everywhere — so its
  // commands reach the keyboard through the palette, which lists them under
  // the chat's name with the named keys they still own.
  await settle(page)
  await page.keyboard.press('ControlOrMeta+k')
  const palette = page.getByRole('dialog')
  await palette.getByRole('textbox').fill('>latest')
  await expect(
    palette
      .getByRole('button', { name: /Chat: Jump to the latest message/ })
      .locator('kbd'),
  ).toHaveText('End')
  await palette.getByRole('textbox').fill('>focus the composer')
  const row = palette.getByRole('button', { name: /Chat: Focus the composer/ })
  await expect(row).toBeVisible()
  await expect(row.locator('kbd')).toHaveCount(0)
  await row.click()
  await expect(composer(page)).toBeFocused()

  // Typing never fires a page key; a letter in the composer is a letter.
  await page.keyboard.press('j')
  await expect(composer(page)).toHaveText('j')
  await page.keyboard.press('Backspace')

  // A prefix narrows the palette to a mode, and the last choice leads the
  // next empty query.
  await settle(page)
  await page.keyboard.press('ControlOrMeta+k')
  await palette.getByRole('textbox').fill('>stop')
  await expect(
    palette.getByRole('button', { name: /Chat: Stop the turn/ }),
  ).toHaveCount(0)
  await palette.getByRole('textbox').fill('>model')
  await expect(
    palette.getByRole('button', { name: /Chat: Switch model/ }),
  ).toBeVisible()
  await expect(palette.getByRole('button', { name: /^workers/ })).toHaveCount(0)
  await palette.getByRole('textbox').fill('')
  await expect(palette.getByText('Recent', { exact: true })).toBeVisible()
  await expect(
    palette.getByRole('button', { name: /Chat: Focus the composer/ }).first(),
  ).toBeVisible()
  await page.keyboard.press('Escape')

  // A second pane, then the alt-braces move the keyboard between panes, and the
  // palette's "open" lands the keyboard in the page it opened.
  await settle(page)
  await expect(page.locator('[data-workspace-pane-id]')).toHaveCount(2)
  await page.keyboard.press('Alt+]')
  await expect(page.locator('[data-workspace-pane-id]')).toHaveCount(3)
  // The new pane opens with its search focused, where `}` is a character.
  await pane(page, 0).focus()
  const focusedPane = page.locator('[data-workspace-pane-id]:focus-within')
  await page.keyboard.press('Alt+}')
  await expect(focusedPane).toHaveAttribute('data-workspace-panel', '1')
  await page.keyboard.press('Alt+{')
  await expect(focusedPane).toHaveAttribute('data-workspace-panel', '0')

  await page.keyboard.press('ControlOrMeta+k')
  await palette.getByRole('textbox').fill('go to workers')
  await palette.getByRole('button', { name: /^workers/ }).click()
  await expect(
    page.locator(
      '[data-workspace-pane-id]:focus-within [aria-label="workers"]',
    ),
  ).toHaveCount(1)

  expectPassingResult(await stack.finish())
})
