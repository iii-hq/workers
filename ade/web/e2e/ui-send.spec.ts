import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'console-streamed-text' })

test('sends and renders a streamed turn through the Console', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await openSession(page, stack)
  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(stack.ready.message)
  await expect(composer).toHaveText(stack.ready.message)
  await page.getByRole('button', { name: 'send message' }).click()

  await expect(
    page.locator('[data-message-role="user"]', {
      hasText: stack.ready.message,
    }),
  ).toHaveCount(1)
  expect(await completed).toMatchObject({
    session_id: stack.ready.session.id,
    status: 'completed',
  })
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'console fixture complete',
    }),
  ).toHaveCount(1)

  expectPassingResult(await stack.finish())
})
