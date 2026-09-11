import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'streamed-text' })

test('hydrates a durable transcript after reload even when the first response is lost', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await stack.trigger()
  expect(await completed).toMatchObject({ status: 'completed' })

  await openSession(page, stack)
  await expect(
    page.locator('[data-message-role="user"]', {
      hasText: stack.ready.message,
    }),
  ).toHaveCount(1)
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'fixture complete',
    }),
  ).toHaveCount(1)

  // Reproduce a live connection losing just the history response: the worker
  // completed the read in CI, but the browser never received its result.
  let droppedHistoryResponse = false
  await page.routeWebSocket('**/ws', (socket) => {
    const server = socket.connectToServer()
    server.onMessage((message) => {
      if (!droppedHistoryResponse && typeof message === 'string') {
        const response = JSON.parse(message)
        if (
          response.type === 'invocationresult' &&
          response.function_id === 'session::messages-tail' &&
          !response.error
        ) {
          droppedHistoryResponse = true
          return
        }
      }
      socket.send(message)
    })
  })

  await page.reload()
  await page
    .getByRole('button', {
      name: `open ${stack.ready.session.title}`,
      exact: true,
    })
    .click()
  await expect(
    page.locator('[data-message-role="user"]', {
      hasText: stack.ready.message,
    }),
  ).toHaveCount(1)
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'fixture complete',
    }),
  ).toHaveCount(1)
  expect(droppedHistoryResponse).toBe(true)

  expectPassingResult(await stack.finish())
})
