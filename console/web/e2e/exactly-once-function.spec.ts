import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'exactly-once-function' })

test('renders one completed function call and its durable result', async ({
  page,
  stack,
}) => {
  const completed = stack.waitForTurnCompleted()
  await stack.trigger('harness::send', stack.ready.send)
  expect(await completed).toMatchObject({ status: 'completed' })

  await openSession(page, stack.ready)
  const functionId = stack.ready.functions.record
  expect(functionId).toBeTruthy()
  const card = page.locator('[data-message-role="function-call"]', {
    hasText: functionId,
  })
  await expect(card).toHaveCount(1)
  await expect(card).toHaveAttribute('data-function-id', functionId)
  await expect(card).toHaveAttribute('data-function-status', 'done')
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
})
