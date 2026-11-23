import type { Page } from '@playwright/test'
import { expect, test } from './context-tracking-stack'

/*
 * chat-projects §1.3.4 / §1.3.6: the header's pane button, Mod+Alt+0 and the
 * palette's "Show context pane" / "Hide context pane" open AND close the
 * pane in each of its three forms, and the button's name follows:
 * - the overlay drawer beside a chat under 900px (the default workspace at
 *   1440, the chat beside the traces pane, D48; or a narrower window),
 * - the fixed column when the chat has room,
 * - the phone's right drawer.
 * The review of round 1 found the drawer's second press did nothing.
 */
test.skip(
  !process.env.CONTEXT_ENGINE_BIN ||
    !process.env.CONTEXT_SESSION_BIN ||
    !process.env.CONTEXT_CONSOLE_BIN,
  'Set CONTEXT_{ENGINE,SESSION,CONSOLE}_BIN to run the isolated stack',
)

const TITLE = 'Pane toggle test'

function pane(page: Page) {
  const button = page.locator('[data-context-pane-button]')
  const aside = page.locator('aside[aria-label="Context pane"]')
  const expectShown = async (shown: boolean) => {
    await expect(button).toHaveAttribute(
      'aria-label',
      shown ? 'Hide context pane' : 'Show context pane',
    )
    await expect(button).toHaveAttribute('aria-expanded', String(shown))
    if (shown) await expect(aside).not.toHaveAttribute('data-collapsed', '')
    else await expect(aside).toHaveAttribute('data-collapsed', '')
  }
  /** Where a pointer lands on the button, whatever is drawn over it. */
  const clickAtButton = async () => {
    const box = await button.boundingBox()
    if (!box) throw new Error('no pane button')
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
  }
  const palette = async (
    title: 'Show context pane' | 'Hide context pane',
    pick: 'click' | 'enter' = 'click',
  ) => {
    await page.keyboard.press('ControlOrMeta+k')
    const dialog = page.getByRole('dialog', { name: 'Command palette' })
    await dialog.getByRole('textbox').fill(`>${title}`)
    const row = dialog.getByRole('button', { name: new RegExp(`^${title}`) })
    await expect(row).toHaveCount(1)
    // On a phone the open drawer is a later portal than the palette and
    // covers most of it: the keyboard that opened the palette picks.
    if (pick === 'enter') await page.keyboard.press('Enter')
    else await row.click()
  }
  /** Open and close it every way, from closed (the column remembers). */
  const cycle = async () => {
    if ((await button.getAttribute('aria-label')) === 'Hide context pane')
      await page.keyboard.press('Control+Alt+0')
    await expectShown(false)
    await button.click()
    await expectShown(true)
    await clickAtButton()
    await expectShown(false)
    await button.dispatchEvent('click')
    await expectShown(true)
    await button.dispatchEvent('click')
    await expectShown(false)
    await page.keyboard.press('Control+Alt+0')
    await expectShown(true)
    await page.keyboard.press('Control+Alt+0')
    await expectShown(false)
    await palette('Show context pane')
    await expectShown(true)
    await palette('Hide context pane')
    await expectShown(false)
  }
  return { button, aside, cycle, palette }
}

test('the context pane opens and closes as a drawer, a column and on a phone', async ({
  page,
  stack,
}) => {
  await stack.create(TITLE)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await page.goto(stack.base)
  await page
    .getByRole('button', { name: `open ${TITLE}`, exact: true })
    .getByText(TITLE, { exact: true })
    .click({ position: { x: 4, y: 4 } })
  const { aside, button, cycle, palette } = pane(page)
  await expect(button).toBeVisible()

  // The drawer: at 1440 beside the traces pane, else in a narrower window.
  const startedAsDrawer = (await aside.getAttribute('data-drawer')) !== null
  if (!startedAsDrawer) await page.setViewportSize({ width: 1000, height: 900 })
  await expect(aside).toHaveAttribute('data-drawer', 'closed')
  await cycle()
  await expect(aside).toHaveAttribute('data-drawer', 'closed')

  // The column: 1440 with the chat alone in its pane.
  await page.setViewportSize({ width: 1440, height: 900 })
  if ((await aside.getAttribute('data-drawer')) !== null) {
    // Close the workspace panel that does not hold the chat (the traces).
    await page.evaluate(() => {
      for (const close of document.querySelectorAll<HTMLButtonElement>(
        'button[aria-label="close panel"]',
      )) {
        let panel = close.parentElement
        while (
          panel &&
          !panel.querySelector('button[aria-label^="reorder panel"]')
        )
          panel = panel.parentElement
        if (panel && !panel.querySelector('[data-context-pane-button]')) {
          close.click()
          return
        }
      }
    })
  }
  await expect(aside).not.toHaveAttribute('data-drawer')
  await cycle()

  // The phone: the right drawer, named Context, the conversation's name
  // first and no "Side panel".
  await page.setViewportSize({ width: 390, height: 844 })
  const drawer = page.locator('[data-side-panel-drawer]')
  await expect(button).toHaveAttribute('aria-label', 'Show context pane')
  await button.click()
  await expect(drawer).toBeVisible()
  await expect(drawer).toHaveAttribute('aria-label', 'Context')
  expect((await drawer.innerText()).split('\n')[0]).toBe(TITLE)
  await expect(drawer).not.toContainText('Side panel')
  await expect(button).toHaveAttribute('aria-label', 'Hide context pane')
  await page.keyboard.press('Control+Alt+0')
  await expect(drawer).toHaveCount(0)
  await expect(button).toHaveAttribute('aria-label', 'Show context pane')
  await page.keyboard.press('Control+Alt+0')
  await expect(drawer).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(drawer).toHaveCount(0)
  await expect(button).toHaveAttribute('aria-label', 'Show context pane')
  await palette('Show context pane', 'enter')
  await expect(drawer).toBeVisible()
  await expect(button).toHaveAttribute('aria-label', 'Hide context pane')
  await palette('Hide context pane', 'enter')
  await expect(drawer).toHaveCount(0)
  await expect(button).toHaveAttribute('aria-label', 'Show context pane')
})
