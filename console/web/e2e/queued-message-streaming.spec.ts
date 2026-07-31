import { expect, expectPassingResult, openSession, test } from './harness-stack'

const GATE = 'console-queued-message-streaming-in-flight'
const QUEUED_MESSAGE = 'Queue this while the first response is streaming.'

test.use({ scenario: 'console-queued-message-streaming' })

test('keeps the composer editable and queues a message while streaming', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await openSession(page, stack)

  const composer = page.getByLabel('message composer')
  await composer.pressSequentially(stack.ready.message)
  await page.getByRole('button', { name: 'send message' }).click()

  await stack.waitForRouterGate(GATE)
  await expect(
    page.getByRole('button', { name: 'stop generating' }),
  ).toBeVisible()
  await expect(composer).toBeEditable()
  await expect(composer).toHaveAttribute('aria-placeholder', 'queue a message…')

  await composer.pressSequentially(QUEUED_MESSAGE)
  await expect(composer).toHaveText(QUEUED_MESSAGE)
  await page.getByRole('button', { name: 'queue message' }).click()

  const queued = page.getByRole('region', { name: 'queued messages' })
  await expect(queued).toContainText(QUEUED_MESSAGE)
  await stack.waitForQueuedMessage(QUEUED_MESSAGE)
  await stack.releaseRouterGate(GATE)

  expect(await completed).toMatchObject({
    session_id: stack.ready.session.id,
    status: 'completed',
  })
  await expect(
    page.locator('[data-message-role="user"]', { hasText: QUEUED_MESSAGE }),
  ).toHaveCount(1)
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'first Console response complete',
    }),
  ).toHaveCount(1)
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'queued Console message complete',
    }),
  ).toHaveCount(1)
  await expect(queued).toHaveCount(0)

  expectPassingResult(await stack.finish())
})
