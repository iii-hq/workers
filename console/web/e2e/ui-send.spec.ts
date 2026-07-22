import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'console-streamed-text' })

test('renders a harness-started streamed turn in the Console', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await openSession(page, stack)
  await stack.start()

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
