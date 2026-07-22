import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.use({ scenario: 'streamed-text' })

test('hydrates a durable transcript again after a page reload', async ({
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

  expectPassingResult(await stack.finish())
})
