import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'exactly-once-function' })

test('renders one completed function call and its durable result', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await stack.trigger()
  expect(await completed).toMatchObject({ status: 'completed' })

  await openSession(page, stack)
  const functionId = stack.ready.functions.record
  expect(functionId).toBeTruthy()
  const card = page.locator('[data-message-role="function-call"]', {
    hasText: functionId,
  })
  await expect(card).toHaveCount(1)
  await expect(card).toHaveAttribute('data-function-id', functionId)
  await expect(card).toHaveAttribute('data-function-status', 'done')

  const cardHeader = card.locator(':scope > button')
  await expect(cardHeader).toHaveAttribute('aria-expanded', 'false')
  await cardHeader.click()
  await expect(cardHeader).toHaveAttribute('aria-expanded', 'true')

  const requestPane = card.locator('[data-function-pane="request"]')
  await expect(requestPane).toBeVisible()
  await expect(requestPane).toContainText('request · value')
  await expect(requestPane.locator('code')).toHaveText('expected')

  const responsePane = card.locator('[data-function-pane="response"]')
  await expect(responsePane).toBeVisible()
  await expect(responsePane).toContainText('recorded')
  await expect(responsePane).toContainText('details')
  await expect(responsePane).toContainText(/"text"\s*:\s*"recorded"/)
  await expect(responsePane).toContainText(/"is_error"\s*:\s*false/)

  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'recorded once',
    }),
  ).toHaveCount(1)

  const result = await stack.finish()
  expectPassingResult(result)
  const recordCalls = (result.evidence?.recorder_events ?? []).filter(
    (event) => event.kind === 'target_call' && event.function_id === functionId,
  )
  expect(recordCalls).toHaveLength(1)
  expect(recordCalls[0]?.payload).toEqual({ value: 'expected' })
})
