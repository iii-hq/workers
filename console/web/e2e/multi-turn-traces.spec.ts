import { expect, expectPassingResult, openSession, test } from './harness-stack'

const SECOND_MESSAGE = 'Return the second trace phrase.'

test.use({ scenario: 'multi-turn-traces' })

test('shows two traces and exposes function arguments in trace events', async ({
  page,
  stack,
}) => {
  const firstCompleted = stack.waitForTurnCompleted()
  await stack.trigger()
  expect(await firstCompleted).toMatchObject({ status: 'completed' })

  await openSession(page, stack)
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'recorded once',
    }),
  ).toHaveCount(1)

  const composer = page.getByLabel('message composer')
  const secondCompleted = stack.waitForTurnCompleted()
  await composer.pressSequentially(SECOND_MESSAGE)
  await expect(composer).toHaveText(SECOND_MESSAGE)
  await page.getByRole('button', { name: 'send message' }).click()
  expect(await secondCompleted).toMatchObject({ status: 'completed' })
  await expect(
    page.locator('[data-message-role="user"]', { hasText: SECOND_MESSAGE }),
  ).toHaveCount(1)
  await expect(
    page.locator('[data-message-role="assistant"]', {
      hasText: 'second trace complete',
    }),
  ).toHaveCount(1)

  const traces = page.getByRole('region', { name: 'traces' })
  const grouping = traces.getByRole('button', {
    name: /^(?:no grouping|group by .+)$/,
  })
  if ((await grouping.textContent())?.trim() !== 'group by session') {
    await grouping.click()
    await page.getByRole('button', { name: 'session', exact: true }).click()
  }

  const group = traces.locator(
    `[data-trace-group-value="${stack.ready.session.id}"]`,
  )
  await expect(group).toHaveAttribute('data-trace-group-count', '2')
  await expect(group).toHaveAttribute('data-trace-group-errors', '0')
  const groupHeader = group.locator(':scope > button')
  if ((await groupHeader.getAttribute('aria-expanded')) !== 'true') {
    await groupHeader.click()
  }
  await expect(groupHeader).toHaveAttribute('aria-expanded', 'true')

  const traceRows = group.locator('[data-trace-row-id]')
  await expect(traceRows).toHaveCount(2)
  await expect(traceRows).toContainText([
    'Return the second trace phrase',
    'Call the recorder once',
  ])
  const traceIds = await traceRows.evaluateAll((rows) =>
    rows.map((row) => row.getAttribute('data-trace-row-id')),
  )
  expect(new Set(traceIds).size).toBe(2)
  for (const row of await traceRows.all()) {
    await expect(row.locator(':scope > button')).toHaveAttribute(
      'data-trace-status',
      'ok',
    )
  }

  const functionTrace = traceRows
    .filter({ hasText: stack.ready.message })
    .locator(':scope > button')
  await expect(functionTrace).toHaveCount(1)
  if ((await functionTrace.getAttribute('aria-expanded')) !== 'true') {
    await functionTrace.click()
  }
  await expect(functionTrace).toHaveAttribute('aria-expanded', 'true')
  await expect(
    traces.getByRole('button', { name: 'close trace detail' }),
  ).toBeVisible()
  await expect(traces.getByText(/\d+ spans/).first()).toBeVisible()
  await expect(traces.getByText(/\d+ workers?/).first()).toBeVisible()

  const functionId = stack.ready.functions.record
  expect(functionId).toBeTruthy()
  const functionSpan = traces.locator(
    `[data-trace-span-name="execute ${functionId}"]`,
  )
  await expect(functionSpan).toBeVisible()
  await functionSpan.click()

  const spanPanel = traces.locator('[data-span-panel]')
  await expect(spanPanel).toHaveAttribute(
    'data-span-name',
    `execute ${functionId}`,
  )
  await spanPanel.getByRole('tab', { name: /^events/ }).click()

  const inputEvent = spanPanel.locator(
    '[data-span-event-name="iii.invocation.input"]',
  )
  await expect(inputEvent).toBeVisible()
  const payload = inputEvent.locator(
    '[data-span-event-attribute="iii.payload.json"] pre',
  )
  await expect(payload).toContainText('"value": "expected"')
  const capturedPayload = JSON.parse(
    (await payload.textContent()) ?? 'null',
  ) as Record<string, unknown>
  const functionArguments = Object.fromEntries(
    Object.entries(capturedPayload).filter(([key]) => !key.startsWith('_')),
  )
  expect(functionArguments).toEqual({ value: 'expected' })
  await expect(
    inputEvent.locator('[data-span-event-attribute="iii.payload.truncated"]'),
  ).toContainText('false')

  const result = await stack.finish()
  expectPassingResult(result)
})
