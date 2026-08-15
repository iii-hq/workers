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

const recoveryMessage =
  'Confirm the chat can continue after the provider issue is corrected.'

for (const fixture of cases) {
  test.describe(`${fixture.family} provider failure`, () => {
    test.use({ scenario: fixture.scenario })

    test('renders and captures the permanent error notice', async ({
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
      const notice = page
        .locator(
          '[data-message-role="system-notice"][data-message-tone="error"]',
        )
        .filter({ hasText: fixture.reason })
      await expect(notice).toHaveCount(1)
      await expect(notice).toContainText('turn failed [llm.permanent]')

      const screenshot = testInfo.outputPath(`${fixture.scenario}.png`)
      await page.screenshot({ path: screenshot, fullPage: true })
      await testInfo.attach(`console-${fixture.scenario}`, {
        path: screenshot,
        contentType: 'image/png',
      })

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
