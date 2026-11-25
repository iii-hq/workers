import type { Locator } from '@playwright/test'
import { expect, expectPassingResult, openSession, test } from './harness-stack'

/*
 * chat-projects §1.4 / §1.8 (4): the agent writes the conversation's
 * description with `harness::set-description` inside the turn it is already
 * running. The scripted provider of the `console-set-description` scenario
 * (harness-integration, Playground driver) answers the one generation with
 * reply text and a `harness::set-description` call in the same step, with
 * no `title`, so the turn ends without another request. The Console then
 * shows the description in the chat header (Muted, with "by agent · …") and
 * on the conversation's row in the sidebar, and the transcript shows the
 * reply but no call card for the write.
 */
const DESCRIPTION = 'Retries with backoff, then a PR'
const REPLY = 'console description fixture complete'

test.describe('the agent keeps the description up to date', () => {
  test.use({ scenario: 'console-set-description' })

  test('the header and the list show it, and the transcript shows no call', async ({
    page,
    stack,
  }) => {
    const completed = stack.waitForTurnCompleted()
    await stack.send()
    expect(await completed).toMatchObject({ status: 'completed' })

    await page.setViewportSize({ width: 1280, height: 800 })
    await openSession(page, stack)

    // Line 2 of the header: the description, then the agent's pill.
    const header = page.locator('[data-chat-header-details]')
    await expect(header.locator('[data-chat-description]')).toHaveText(
      DESCRIPTION,
    )
    await expect(header.locator('[data-by-agent]')).toContainText('by agent')
    // A mono Faint pill (§1.3.4): Geist Mono, though the chat sets its
    // font-mono in Inter.
    const monoFont = (pill: Locator) =>
      pill.evaluate((el) => getComputedStyle(el).fontFamily)
    expect(await monoFont(header.locator('[data-by-agent]'))).toMatch(
      /^"?Geist Mono/,
    )

    // Line 2 of the sidebar row.
    const row = page.getByRole('button', {
      name: `open ${stack.ready.session.title}`,
      exact: true,
    })
    await expect(row).toContainText(DESCRIPTION)

    // The reply is in the transcript; the write is not.
    const transcript = page.locator(
      `[data-chat-session-id="${stack.ready.session.id}"]`,
    )
    await expect(transcript).toContainText(REPLY)
    await expect(
      transcript.getByText('harness::set-description', { exact: false }),
    ).toHaveCount(0)
    await expect(
      transcript.getByText('set-description', { exact: false }),
    ).toHaveCount(0)

    // Conversation details opens from the description, with the switch on.
    await header.locator('[data-chat-description]').click()
    const details = page.getByRole('dialog', { name: 'Conversation details' })
    await expect(details).toBeVisible()
    await expect(
      details.getByRole('switch', { name: 'Let the agent keep it up to date' }),
    ).toBeChecked()
    expect(await monoFont(details.locator('[data-by-agent]'))).toMatch(
      /^"?Geist Mono/,
    )
    await page.keyboard.press('Escape')

    expectPassingResult(await stack.finish())
  })
})
