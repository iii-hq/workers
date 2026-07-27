import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'console-streamed-text' })

/**
 * The session metrics dialog opens from either of its two triggers and shows
 * the provider-reported numbers the harness persisted during the turn.
 *
 * Selectors are attributes rather than text, matching `ui-send.spec.ts` — the
 * copy in this panel is expected to change.
 */
test('opens session metrics from the header and from the ctx widget', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await openSession(page, stack)

  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(stack.ready.message)
  await page.getByRole('button', { name: 'send message' }).click()
  await completed
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'console fixture complete',
    }),
  ).toHaveCount(1)

  const dialog = page.locator('[data-testid="session-metrics"]')

  // Trigger 1: the header button. Addressed by testid, not accessible name —
  // `getByRole(name:)` matches substrings, so a sidebar row for a session
  // whose title contains "metrics" would win instead.
  await page.getByTestId('session-metrics-trigger').click()
  await expect(dialog).toBeVisible()

  // The three sections are structural, not decorative: which one a number
  // sits in is the statement about how much to trust it.
  await expect(dialog.getByText('exact', { exact: true })).toBeVisible()
  await expect(dialog.getByText('counted', { exact: true })).toBeVisible()
  await expect(dialog.getByText('estimated', { exact: true })).toBeVisible()

  // The scripted fixture reports usage, so input tokens must not be a dash.
  const inputRow = dialog.locator('div', { hasText: /^input tokens/ }).last()
  await expect(inputRow).not.toContainText('—')

  await dialog.getByRole('tab', { name: 'turns' }).click()
  await expect(dialog.getByRole('columnheader', { name: 'turn' })).toBeVisible()

  await dialog.getByRole('tab', { name: 'tree' }).click()
  await expect(dialog).toBeVisible()

  await page.keyboard.press('Escape')
  await expect(dialog).toBeHidden()

  // Trigger 2: clicking the ctx widget — the discoverability path.
  await page.getByRole('button', { name: /^ctx/ }).click()
  await expect(dialog).toBeVisible()

  expectPassingResult(await stack.finish())
})

test('shows a per-turn usage chip in the transcript', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await openSession(page, stack)

  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(stack.ready.message)
  await page.getByRole('button', { name: 'send message' }).click()
  await completed

  const chip = page.locator('[data-turn-usage]').first()
  await expect(chip).toBeVisible()

  // Expanding is the point of the chip: per-step is where cache warmth shows.
  await chip.getByRole('button').first().click()
  await expect(chip).toContainText('step 0')

  expectPassingResult(await stack.finish())
})
