import type { Page } from '@playwright/test'
import { expect, expectPassingResult, openSession, test } from './harness-stack'

const cases = [
  {
    scenario: 'console-anthropic-messages-error',
    family: 'anthropic messages',
    reason: 'anthropic messages: credit balance is too low',
  },
  {
    scenario: 'console-openai-chat-error',
    family: 'openai chat completions',
    reason: 'openai chat completions: insufficient quota',
  },
  {
    scenario: 'console-openai-responses-error',
    family: 'openai responses',
    reason: 'openai responses: credit balance exhausted',
  },
] as const

const failureSummary = 'The provider rejected this request.'
// Every fixture reason names a billing wall, so the console classifies the
// failure as the user's provider account (not iii) and swaps the harness's
// generic "review the provider settings" advice for account-specific steps.
const failureTitle = 'Provider credit or quota exhausted'
const ownerLabel = 'Needs your attention'
const recoveryMessage =
  'Confirm the chat can continue after the provider issue is corrected.'

async function expectFailureNotice(page: Page) {
  const notice = page
    .locator('[data-message-role="turn-failure"]')
    .filter({ hasText: failureSummary })
  await expect(notice).toHaveCount(1)
  await expect(notice).toHaveAttribute('data-failure-category', 'billing')
  await expect(notice).toHaveAttribute('data-failure-owner', 'user')
  await expect(notice.getByRole('heading')).toHaveText(failureTitle)
  await expect(notice.getByText(ownerLabel)).toBeVisible()
  await expect(notice.locator('[data-message-summary]')).toHaveText(
    failureSummary,
  )
  await expect(notice.locator('[data-failure-ownership]')).toContainText(
    'not an iii or console failure',
  )
  const actions = notice.locator('[data-message-next-actions] li')
  await expect(actions).toHaveCount(2)
  await expect(actions.first()).toContainText('Add credit')
  const details = notice.locator('[data-message-technical-details]')
  await expect(details).not.toHaveAttribute('open', '')
  return details
}

for (const fixture of cases) {
  test.describe(`${fixture.family} provider failure`, () => {
    test.use({ scenario: fixture.scenario })

    test('renders a durable human-readable error with technical details', async ({
      page,
      stack,
    }, testInfo) => {
      const failed = stack.waitForTurnCompleted()
      await openSession(page, stack)
      const composer = page.getByLabel('message composer')
      await composer.pressSequentially(stack.ready.message)
      await page.getByRole('button', { name: 'send message' }).click()

      expect(await failed).toMatchObject({
        session_id: stack.ready.session.id,
        status: 'failed',
      })
      const details = await expectFailureNotice(page)
      await details.locator('summary').click()
      await expect(details).toHaveAttribute('open', '')
      await expect(
        details.locator('[data-technical-detail="code"]'),
      ).toHaveText('invocation_failed')
      await expect(
        details.locator('[data-technical-detail="class"]'),
      ).toHaveText('llm.permanent')
      await expect(
        details.locator('[data-technical-detail="detail"]'),
      ).toContainText(fixture.reason)

      const screenshot = testInfo.outputPath(`${fixture.scenario}.png`)
      await page.screenshot({ path: screenshot, fullPage: true })
      await testInfo.attach(`console-${fixture.scenario}`, {
        path: screenshot,
        contentType: 'image/png',
      })

      await page.reload()
      const session = page.getByRole('button', {
        name: `open ${stack.ready.session.title}`,
        exact: true,
      })
      await session.click()
      await expect(session).toHaveAttribute('aria-current', 'page')
      await expect(
        page.locator(`[data-chat-session-id="${stack.ready.session.id}"]`),
      ).toHaveAttribute('data-chat-session-hydrated', 'true')

      const rehydratedDetails = await expectFailureNotice(page)
      await rehydratedDetails.locator('summary').click()
      await expect(
        rehydratedDetails.locator('[data-technical-detail="detail"]'),
      ).toContainText(fixture.reason)

      const recovered = stack.waitForTurnCompleted()
      await composer.pressSequentially(recoveryMessage)
      await page.getByRole('button', { name: 'send message' }).click()
      expect(await recovered).toMatchObject({
        session_id: stack.ready.session.id,
        status: 'completed',
      })
      await expect(
        page.locator('[data-message-role="assistant"]', {
          hasText: 'provider family recovery complete',
        }),
      ).toHaveCount(1)

      expectPassingResult(await stack.finish())
    })
  })
}
